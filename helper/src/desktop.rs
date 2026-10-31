// 独立壁纸模式（--wallpaper）：不依赖 Wallpaper Engine，把 HUD 嵌到桌面图标后面
// - 进程内采样 + HTTP 服务（/stats），页面和字体编译时内嵌，经 wry 自定义协议 hud:// 提供（来源固定，设置不随端口变化）
// - 每个显示器一个窗口：HUD 按 1920×1080 设计、等比适配，跨屏铺一个窗口会被拉伸并横跨边框，
//   各屏 DPI 也可能不同；分屏还能只暂停有全屏应用的那块屏
// - 全屏应用在前台时暂停对应屏幕，托盘可整体暂停；Explorer 重启（TaskbarCreated）或显示器变化后重新嵌入
// - 嵌在桌面里的窗口收不到鼠标事件，轮询 GetCursorPos 把光标位置交给页面做视差（不用低级鼠标钩子，避免拖慢全局输入）
use crate::install::{self, tr};
use hud_telemetry::{pdh::wide, server};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use std::borrow::Cow;
use std::cell::RefCell;
use std::net::TcpListener;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::http::{header::CONTENT_TYPE, Request, Response};
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

pub const CONTROL_CLASS: &str = "TelemetryHUDDesktop";
const WALL_CLASS: &str = "TelemetryHUDWall";
const SETTINGS_CLASS: &str = "TelemetryHUDSettings";
const DEFAULT_PORT: u16 = 27182;
const WM_TRAY: u32 = WM_APP + 1;
const TIMER_TICK: usize = 1;
const TIMER_MOUSE: usize = 2;
const CMD_SETTINGS: usize = 1;
const CMD_PAUSE: usize = 2;
const CMD_EXIT: usize = 3;
/// 让 Progman 在图标层后面生成 WorkerW 的未公开消息
const SPAWN_WORKERW: u32 = 0x052C;

// 页面和字体：编译时内嵌，自定义协议直接返回
const ASSETS: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../../hud/index.html")),
    ("fonts/fonts.css", "text/css; charset=utf-8", include_bytes!("../../hud/fonts/fonts.css")),
    ("fonts/chakra-petch-400.woff2", "font/woff2", include_bytes!("../../hud/fonts/chakra-petch-400.woff2")),
    ("fonts/chakra-petch-600.woff2", "font/woff2", include_bytes!("../../hud/fonts/chakra-petch-600.woff2")),
    ("fonts/jetbrains-mono-400-700.woff2", "font/woff2", include_bytes!("../../hud/fonts/jetbrains-mono-400-700.woff2")),
    ("fonts/michroma-400.woff2", "font/woff2", include_bytes!("../../hud/fonts/michroma-400.woff2")),
];

struct Hwnd(HWND);
impl HasWindowHandle for Hwnd {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let h = Win32WindowHandle::new(NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(h)) })
    }
}

/// 窗口过程只把事件记下来，由主循环统一处理：创建 WebView 时 wry 会嵌套跑消息循环，在窗口过程里直接改状态会重入
enum Ev {
    Tick,
    Mouse,
    Tray(u32),
    Command(usize),
    Rebuild,
    CloseSettings,
    Quit,
}

thread_local! {
    static EVENTS: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
    static CONTROL: std::cell::Cell<HWND> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

fn push(ev: Ev) {
    EVENTS.with(|e| e.borrow_mut().push(ev));
    // 发送来的消息（SendMessage）在 GetMessage 内部处理、不会让它返回，投一条空消息把主循环唤醒
    unsafe { PostMessageW(CONTROL.with(|c| c.get()), WM_NULL, 0, 0) };
}

static TASKBAR_CREATED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

unsafe extern "system" fn control_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp == TIMER_TICK => push(Ev::Tick),
        WM_TIMER if wp == TIMER_MOUSE => push(Ev::Mouse),
        WM_TRAY => push(Ev::Tray(lp as u32)),
        WM_DISPLAYCHANGE | WM_DPICHANGED => push(Ev::Rebuild),
        WM_CLOSE | WM_ENDSESSION => push(Ev::Quit),
        _ if Some(&msg) == TASKBAR_CREATED.get() => push(Ev::Rebuild),
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

unsafe extern "system" fn wall_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            push(Ev::CloseSettings);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// [left, top, right, bottom]，物理像素（windows-sys 的 RECT 不能比较）
type Rect = [i32; 4];
fn rect(r: RECT) -> Rect {
    [r.left, r.top, r.right, r.bottom]
}

#[derive(Clone, Copy, PartialEq)]
struct Monitor {
    rect: Rect,
    dpi: u32,
    primary: bool,
}

fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> windows_sys::core::BOOL {
        let out = &mut *(lp as *mut Vec<Monitor>);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi) != 0 {
            let (mut dx, mut dy) = (96, 96);
            GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            out.push(Monitor { rect: rect(mi.rcMonitor), dpi: dx, primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0 });
        }
        1
    }
    let mut v: Vec<Monitor> = Vec::new();
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(cb), &mut v as *mut _ as LPARAM) };
    v
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// 桌面图标后面的宿主：(父窗口, 插在它后面的兄弟窗口)
/// - Windows 11 24H2 起：图标层 SHELLDLL_DefView 和 WorkerW 都是 Progman 的子窗口，
///   挂到 Progman 下、z 序紧贴在 DefView 后面（在负责画系统壁纸的 WorkerW 之上）
/// - 更早的版本：DefView 所在顶层窗口后面的那个顶层 WorkerW
fn desktop_host() -> Option<(HWND, HWND)> {
    unsafe {
        let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
        if progman.is_null() {
            return None;
        }
        let mut r = 0;
        SendMessageTimeoutW(progman, SPAWN_WORKERW, 0xD, 0x1, SMTO_NORMAL, 1000, &mut r);
        SendMessageTimeoutW(progman, SPAWN_WORKERW, 0, 0, SMTO_NORMAL, 1000, &mut r);

        let defview = FindWindowExW(progman, std::ptr::null_mut(), wide("SHELLDLL_DefView").as_ptr(), std::ptr::null());
        let inner = FindWindowExW(progman, std::ptr::null_mut(), wide("WorkerW").as_ptr(), std::ptr::null());
        if !defview.is_null() && !inner.is_null() {
            return Some((progman, defview));
        }

        unsafe extern "system" fn find(top: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
            let dv = FindWindowExW(top, std::ptr::null_mut(), wide("SHELLDLL_DefView").as_ptr(), std::ptr::null());
            if !dv.is_null() {
                *(lp as *mut HWND) = FindWindowExW(std::ptr::null_mut(), top, wide("WorkerW").as_ptr(), std::ptr::null());
                return 0;
            }
            1
        }
        let mut worker: HWND = std::ptr::null_mut();
        EnumWindows(Some(find), &mut worker as *mut _ as LPARAM);
        (!worker.is_null()).then_some((worker, HWND_TOP))
    }
}

/// 被其他窗口挡住的显示器：上面有最大化的窗口，或有窗口铺满整块屏（全屏应用、无边框游戏）。
/// 不只看前台窗口：焦点切到另一块屏时，这块屏上的最大化窗口照样挡着壁纸。
/// 跳过最小化、隐身（其他虚拟桌面上的窗口、后台 UWP）、鼠标穿透的覆盖层和工具窗口。
/// 独占全屏的 D3D 游戏不一定有正常的窗口矩形，另由 QUNS 判断为全部暂停。
/// 壁纸窗口是子窗口，EnumWindows 不会列出；设置窗口最大化时同样挡住壁纸，照常算
fn covered_monitors() -> Vec<Rect> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let out = &mut *(lp as *mut Vec<Rect>);
        if IsWindowVisible(h) == 0 || IsIconic(h) != 0 {
            return 1;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & (WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW) != 0 {
            return 1;
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as _, 4);
        if cloaked != 0 || matches!(class_of(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return 1;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return 1;
        }
        let (m, mut wr) = (mi.rcMonitor, std::mem::zeroed::<RECT>());
        GetWindowRect(h, &mut wr);
        let full = wr.left <= m.left && wr.top <= m.top && wr.right >= m.right && wr.bottom >= m.bottom;
        if (full || IsZoomed(h) != 0) && !out.contains(&rect(m)) {
            out.push(rect(m));
        }
        1
    }
    let mut out: Vec<Rect> = Vec::new();
    unsafe { EnumWindows(Some(cb), &mut out as *mut _ as LPARAM) };
    out
}

fn exclusive_fullscreen() -> bool {
    let mut s = 0;
    unsafe { SHQueryUserNotificationState(&mut s) >= 0 && s == QUNS_RUNNING_D3D_FULL_SCREEN }
}

/// 托盘 / 窗口图标：运行时画一个 ember 配色的环 + 中心点（项目没有 build script，不嵌资源）
fn make_icon(size: i32) -> HICON {
    let n = size as usize;
    let c = size as f32 / 2.0;
    let mut px = vec![0u32; n * n];
    let blend = |dst: u32, rgb: u32, a: f32| -> u32 {
        let a = a.clamp(0.0, 1.0);
        let da = (dst >> 24) as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        if oa <= 0.0 {
            return 0;
        }
        let ch = |sh: u32| {
            let s = ((rgb >> sh) & 255) as f32;
            let d = ((dst >> sh) & 255) as f32;
            ((s * a + d * da * (1.0 - a)) / oa).round() as u32
        };
        ((oa * 255.0).round() as u32) << 24 | ch(16) << 16 | ch(8) << 8 | ch(0)
    };
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let r = c - 0.5;
            let mut p = blend(0, 0x0c0c0e, r - d + 0.5);
            let ring = c * 0.62;
            p = blend(p, 0xff6b2c, c * 0.14 - (d - ring).abs() + 0.5);
            p = blend(p, 0x8fd3d6, c * 0.2 - d + 0.5);
            px[y * n + x] = p;
        }
    }
    unsafe {
        let color = CreateBitmap(size, size, 1, 32, px.as_ptr() as _);
        let mask = CreateBitmap(size, size, 1, 1, vec![0u8; n.div_ceil(16) * 2 * n].as_ptr() as _);
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

struct Wall {
    hwnd: HWND,
    parent: HWND,
    rect: Rect,
    webview: WebView,
    paused: Option<bool>,
}

struct App {
    hinst: HINSTANCE,
    control: HWND,
    ctx: WebContext,
    query: String,
    monitors: Vec<Monitor>,
    walls: Vec<Wall>,
    settings: Option<(HWND, WebView)>,
    user_paused: bool,
    rebuild_at: Option<Instant>,
    first_build: bool,
    cursor: POINT,
    tray: NOTIFYICONDATAW,
}

fn protocol(_: wry::WebViewId, req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match ASSETS.iter().find(|a| a.0 == path) {
        Some((_, mime, body)) => Response::builder().header(CONTENT_TYPE, *mime).body(Cow::Borrowed(*body)).unwrap(),
        None => Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

impl App {
    fn url(&self, extra: &str) -> String {
        let q: Vec<&str> = self.query.split('&').chain(extra.split('&')).filter(|p| !p.is_empty()).collect();
        if q.is_empty() {
            "hud://localhost/index.html".into()
        } else {
            format!("hud://localhost/index.html?{}", q.join("&"))
        }
    }

    fn webview(&mut self, parent: HWND, url: String, focused: bool) -> wry::Result<WebView> {
        let release = !cfg!(debug_assertions);
        WebViewBuilder::new_with_web_context(&mut self.ctx)
            .with_custom_protocol("hud".into(), protocol)
            .with_url(url)
            .with_background_color((12, 12, 14, 255))
            .with_autoplay(true)
            .with_focused(focused)
            .with_devtools(!release)
            .with_default_context_menus(!release)
            .with_browser_accelerator_keys(!release)
            .with_hotkeys_zoom(false)
            .build(&Hwnd(parent))
    }

    fn destroy_walls(&mut self) {
        for w in self.walls.drain(..) {
            drop(w.webview);
            unsafe {
                if IsWindow(w.hwnd) != 0 {
                    DestroyWindow(w.hwnd);
                }
            }
        }
    }

    /// 按当前显示器重新建壁纸窗口。桌面宿主还没就绪（刚登录、Explorer 正在重启）时稍后重试
    fn rebuild(&mut self) {
        self.destroy_walls();
        self.rebuild_at = None;
        self.monitors = monitors();
        let Some((parent, after)) = desktop_host() else {
            self.rebuild_at = Some(Instant::now() + Duration::from_secs(2));
            return;
        };
        // 开屏动画只在第一次放；只让主显示器出声，免得多块屏叠在一起响
        let intro = if self.first_build { "" } else { "intro=0" };
        self.first_build = false;
        let class = wide(WALL_CLASS);
        for m in self.monitors.clone() {
            // 子窗口坐标相对父窗口客户区
            let [l, t, r, b] = m.rect;
            let mut pt = [POINT { x: l, y: t }];
            unsafe { MapWindowPoints(std::ptr::null_mut(), parent, pt.as_mut_ptr(), 1) };
            let (w, h) = (r - l, b - t);
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    wide("Telemetry HUD").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                    pt[0].x,
                    pt[0].y,
                    w,
                    h,
                    parent,
                    std::ptr::null_mut(),
                    self.hinst,
                    std::ptr::null(),
                )
            };
            if hwnd.is_null() {
                continue;
            }
            unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
            let url = self.url(&[intro, if m.primary { "" } else { "mute=1" }].join("&"));
            match self.webview(hwnd, url, false) {
                Ok(webview) => self.walls.push(Wall { hwnd, parent, rect: m.rect, webview, paused: None }),
                Err(_) => unsafe {
                    DestroyWindow(hwnd);
                },
            }
        }
        if self.walls.is_empty() {
            self.rebuild_at = Some(Instant::now() + Duration::from_secs(5));
        }
    }

    fn tick(&mut self) {
        if self.rebuild_at.is_some_and(|t| Instant::now() >= t) {
            self.rebuild();
        }
        // 宿主没了（Explorer 崩溃 / 重启）或显示器布局、缩放变了：过一会儿重建
        let host_lost = self.walls.iter().any(|w| unsafe { IsWindow(w.hwnd) == 0 || IsWindow(w.parent) == 0 });
        if self.rebuild_at.is_none() && (host_lost || monitors() != self.monitors) {
            self.rebuild_at = Some(Instant::now() + Duration::from_millis(1500));
        }
        let covered = covered_monitors();
        let all = self.user_paused || exclusive_fullscreen();
        for w in &mut self.walls {
            let p = all || covered.contains(&w.rect);
            // 每秒都重发：页面刚载入时前一次调用可能还没有 hudPause
            if w.paused != Some(p) || p {
                let _ = w.webview.evaluate_script(&format!("window.hudPause && hudPause({p})"));
                w.paused = Some(p);
            }
        }
    }

    fn mouse(&mut self) {
        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt) } == 0 || (pt.x, pt.y) == (self.cursor.x, self.cursor.y) {
            return;
        }
        self.cursor = pt;
        // 只发给光标所在显示器的那块壁纸；其他屏 3 秒没收到就自己回正
        for w in &self.walls {
            let [l, t, r, b] = w.rect;
            if w.paused == Some(true) || pt.x < l || pt.x >= r || pt.y < t || pt.y >= b {
                continue;
            }
            let nx = (pt.x - l) as f64 / (r - l) as f64 * 2.0 - 1.0;
            let ny = (pt.y - t) as f64 / (b - t) as f64 * 2.0 - 1.0;
            let _ = w.webview.evaluate_script(&format!("window.hudPointer && hudPointer({nx:.4}, {ny:.4})"));
        }
    }

    /// 托盘「设置」：普通窗口里开一份带控制面板的 HUD；和壁纸共用 WebView2 数据目录，改动经 storage 事件同步到各屏
    fn open_settings(&mut self) {
        if let Some((h, _)) = &self.settings {
            unsafe {
                ShowWindow(*h, SW_RESTORE);
                SetForegroundWindow(*h);
            }
            return;
        }
        let scale = unsafe { GetDpiForSystem() } as f64 / 96.0;
        let (w, h) = ((1280.0 * scale) as i32, (760.0 * scale) as i32);
        let title = wide(tr("Telemetry HUD Settings", "Telemetry HUD 设置", "Telemetry HUD 設定"));
        let hwnd = unsafe {
            let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            CreateWindowExW(
                0,
                wide(SETTINGS_CLASS).as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                (sw - w) / 2,
                (sh - h) / 2,
                w,
                h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                self.hinst,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return;
        }
        let url = self.url("panel=1&mute=1&intro=0");
        match self.webview(hwnd, url, true) {
            Ok(wv) => {
                unsafe { SetForegroundWindow(hwnd) };
                self.settings = Some((hwnd, wv));
            }
            Err(_) => unsafe {
                DestroyWindow(hwnd);
            },
        }
    }

    fn close_settings(&mut self) {
        if let Some((h, wv)) = self.settings.take() {
            drop(wv);
            unsafe { DestroyWindow(h) };
        }
    }

    fn add_tray(&mut self) {
        unsafe { Shell_NotifyIconW(NIM_ADD, &self.tray) };
    }

    fn tray_menu(&mut self) {
        unsafe {
            let menu = CreatePopupMenu();
            AppendMenuW(menu, MF_STRING, CMD_SETTINGS, wide(tr("Settings…", "设置…", "設定…")).as_ptr());
            AppendMenuW(menu, MF_STRING | if self.user_paused { MF_CHECKED } else { 0 }, CMD_PAUSE, wide(tr("Pause", "暂停", "一時停止")).as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, CMD_EXIT, wide(tr("Exit", "退出", "終了")).as_ptr());
            SetMenuDefaultItem(menu, CMD_SETTINGS as u32, 0);
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            // 先把隐藏窗口设为前台，否则点菜单外面菜单不会消失
            SetForegroundWindow(self.control);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, self.control, std::ptr::null());
            PostMessageW(self.control, WM_NULL, 0, 0);
            DestroyMenu(menu);
            if cmd > 0 {
                push(Ev::Command(cmd as usize));
            }
        }
    }

    fn set_paused(&mut self, on: bool) {
        self.user_paused = on;
        unsafe {
            if on {
                KillTimer(self.control, TIMER_MOUSE);
            } else {
                SetTimer(self.control, TIMER_MOUSE, 33, None);
            }
        }
        self.tick();
    }

    fn quit(&mut self) {
        self.close_settings();
        self.destroy_walls();
        unsafe {
            // 让桌面把壁纸重画一遍，免得残留最后一帧
            let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
            RedrawWindow(progman, std::ptr::null(), std::ptr::null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
            Shell_NotifyIconW(NIM_DELETE, &self.tray);
            DestroyWindow(self.control);
        }
    }
}

fn register(hinst: HINSTANCE, name: &str, proc_: WNDPROC, icon: HICON, bg: HBRUSH) {
    let class = wide(name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: proc_,
        hInstance: hinst,
        hIcon: icon,
        hIconSm: icon,
        hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
        hbrBackground: bg,
        lpszClassName: class.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { RegisterClassExW(&wc) };
}

pub fn run(lhm_port: u16) {
    unsafe {
        // 单实例
        let _mutex = CreateMutexW(std::ptr::null(), 0, wide(r"Local\TelemetryHUD-Desktop").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }

    // 默认端口被占用（例如还有一个 --background 实例）就换空闲端口，并通过 ?endpoint= 告诉页面
    let port = std::env::var("HUD_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let listener = match TcpListener::bind(("127.0.0.1", port)).or_else(|_| TcpListener::bind(("127.0.0.1", 0))) {
        Ok(l) => l,
        Err(_) => return,
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let snap = server::spawn_sampler(lhm_port);
    std::thread::spawn(move || server::serve(listener, snap));

    let data_dir: Option<PathBuf> = install::Ctx::new().ok().map(|c| c.dir.join("webview"));
    let hinst = unsafe { GetModuleHandleW(std::ptr::null()) };
    let icon_big = make_icon(unsafe { GetSystemMetrics(SM_CXICON) });
    let icon_small = make_icon(unsafe { GetSystemMetrics(SM_CXSMICON) });
    let bg = unsafe { CreateSolidBrush(0x000E_0C0C) }; // #0c0c0e（COLORREF 为 BGR）
    register(hinst, CONTROL_CLASS, Some(control_proc), std::ptr::null_mut(), std::ptr::null_mut());
    register(hinst, WALL_CLASS, Some(wall_proc), std::ptr::null_mut(), bg);
    register(hinst, SETTINGS_CLASS, Some(settings_proc), icon_big, bg);
    let _ = TASKBAR_CREATED.set(unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) });

    // 隐藏的顶层窗口（不是 message-only：要收 TaskbarCreated、WM_DISPLAYCHANGE 这类广播）
    let control = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide(CONTROL_CLASS).as_ptr(),
            wide("Telemetry HUD").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        )
    };
    if control.is_null() {
        return;
    }
    CONTROL.with(|c| c.set(control));

    let mut tray: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    tray.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    tray.hWnd = control;
    tray.uID = 1;
    tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    tray.uCallbackMessage = WM_TRAY;
    tray.hIcon = icon_small;
    let tip = wide("Telemetry HUD");
    tray.szTip[..tip.len()].copy_from_slice(&tip);

    let mut app = App {
        hinst,
        control,
        ctx: WebContext::new(data_dir),
        // HUD_QUERY：额外的网址参数（如 theme=glacier&fps=30），开发调试用
        query: [
            if port == DEFAULT_PORT { String::new() } else { format!("endpoint=http://127.0.0.1:{port}/stats") },
            std::env::var("HUD_QUERY").unwrap_or_default(),
        ]
        .join("&"),
        monitors: Vec::new(),
        walls: Vec::new(),
        settings: None,
        user_paused: false,
        rebuild_at: None,
        first_build: true,
        cursor: POINT { x: i32::MIN, y: i32::MIN },
        tray,
    };
    app.add_tray();
    app.rebuild();
    unsafe {
        SetTimer(control, TIMER_TICK, 1000, None);
        SetTimer(control, TIMER_MOUSE, 33, None);
    }

    let mut quit = false;
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while !quit && unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        loop {
            let evs = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
            if evs.is_empty() {
                break;
            }
            for ev in evs {
                match ev {
                    Ev::Tick => app.tick(),
                    Ev::Mouse => app.mouse(),
                    Ev::Tray(m) => match m {
                        WM_LBUTTONDBLCLK => app.open_settings(),
                        WM_RBUTTONUP | WM_LBUTTONUP => app.tray_menu(),
                        _ => {}
                    },
                    Ev::Command(CMD_SETTINGS) => app.open_settings(),
                    Ev::Command(CMD_PAUSE) => app.set_paused(!app.user_paused),
                    Ev::Command(CMD_EXIT) | Ev::Quit => quit = true,
                    Ev::Command(_) => {}
                    Ev::Rebuild => {
                        // TaskbarCreated 时托盘图标已随旧 Explorer 消失，需要重新添加
                        app.add_tray();
                        app.rebuild_at = Some(Instant::now() + Duration::from_millis(1500));
                    }
                    Ev::CloseSettings => app.close_settings(),
                }
            }
        }
    }
    app.quit();
    std::process::exit(0);
}
