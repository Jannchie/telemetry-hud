// Wallpaper Engine「应用程序」壁纸：进程内采样 + HTTP 服务，WebView2 渲染 HUD 页面
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hud_telemetry::{pdh::wide, server};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use std::net::TcpListener;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::CreateSolidBrush;
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

const DEFAULT_PORT: u16 = 27182;
const EMBEDDED_PAGE: &str = include_str!("../../../hud/index.html");

struct Hwnd(HWND);
impl HasWindowHandle for Hwnd {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let h = Win32WindowHandle::new(NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(h)) })
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // WS_THICKFRAME 保留可缩放，但去掉非客户区边框 → 无边框
        WM_NCCALCSIZE if wp != 0 => 0,
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        // WM_SIZE 由 wry 对父窗口的 subclass 处理（把 WebView 铺满客户区）
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

fn main() {
    unsafe {
        // 单实例
        let _mutex = CreateMutexW(std::ptr::null(), 0, wide(r"Local\TelemetryHUD-Wallpaper").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }

    // 默认端口被占用（例如独立的 hud-telemetry 在跑）就换一个空闲端口
    let listener = TcpListener::bind(("127.0.0.1", DEFAULT_PORT)).or_else(|_| TcpListener::bind(("127.0.0.1", 0))).expect("bind failed");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(DEFAULT_PORT);
    let page_path = exe_dir().map(|d| d.join("index.html"));
    // exe 同目录有 index.html 就用它（可不重编译更新页面），否则用编译时内嵌的副本
    let page: server::PageFn = Arc::new(move || {
        page_path.as_ref().and_then(|p| std::fs::read(p).ok()).unwrap_or_else(|| EMBEDDED_PAGE.as_bytes().to_vec())
    });
    let snap = server::spawn_sampler(8085);
    std::thread::spawn(move || server::serve(listener, snap, Some(page)));

    let mut url = format!("http://127.0.0.1:{port}/");
    if port != DEFAULT_PORT {
        url += &format!("?endpoint=http://127.0.0.1:{port}/stats");
    }

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("TelemetryHUDWallpaper");
        let title = wide("Telemetry HUD");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: CreateSolidBrush(0x000E_0C0C), // #0c0c0e（COLORREF 为 BGR）
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_POPUP | WS_THICKFRAME | WS_VISIBLE, 0, 0, w, h, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        if hwnd.is_null() {
            return;
        }

        let data_dir = std::env::var_os("HUD_WEBVIEW_DATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join(r"TelemetryHUD\webview")));
        let mut ctx = WebContext::new(data_dir);
        let release = !cfg!(debug_assertions);
        let parent = Hwnd(hwnd);
        let webview = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_url(url)
            .with_background_color((12, 12, 14, 255))
            .with_autoplay(true) // wry 会追加 --autoplay-policy=no-user-gesture-required
            .with_devtools(!release)
            .with_default_context_menus(!release)
            .with_browser_accelerator_keys(!release)
            .with_hotkeys_zoom(false)
            .build(&parent);
        let Ok(webview) = webview else {
            MessageBoxW(hwnd, wide("WebView2 初始化失败，请确认已安装 WebView2 Runtime。").as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
            return;
        };

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(webview);
    }
    std::process::exit(0);
}
