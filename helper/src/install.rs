// 双击安装 / 卸载：复制到 %LOCALAPPDATA%\TelemetryHUD，写 HKCU Run 自启，无需管理员
// 测试覆盖：HUD_TEST_ROOT 代替 %LOCALAPPDATA%（安装到 <HUD_TEST_ROOT>\TelemetryHUD）；
//           HUD_TEST_RUNKEY 代替 Run 键路径（只接受 HKCU\...，如 HKCU\Software\TelemetryHUD-Test\Run）
use hud_telemetry::pdh::wide;
use hud_telemetry::sysinfo::reg_string;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, SetHandleInformation, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const VALUE: &str = "TelemetryHUD";
const TITLE: &str = "Telemetry HUD";
const EXE: &str = "hud-telemetry.exe";
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// 按 Windows 显示语言选择提示文本：中文、日文系统用对应语言，其余用英文
pub fn tr(en: &'static str, zh: &'static str, ja: &'static str) -> &'static str {
    match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
        0x04 => zh,
        0x11 => ja,
        _ => en,
    }
}

/// 安装后以哪种方式运行：只跑数据服务（给 Wallpaper Engine / Lively 的网页壁纸用），或独立壁纸（自带数据服务）
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Helper,
    Wallpaper,
}

impl Mode {
    pub fn arg(self) -> &'static str {
        match self {
            Mode::Helper => "--background",
            Mode::Wallpaper => "--wallpaper",
        }
    }
}

pub struct Ctx {
    pub dir: PathBuf,
    pub exe: PathBuf,
    run_key: String,
}

impl Ctx {
    pub fn new() -> Result<Ctx, String> {
        let root = match std::env::var_os("HUD_TEST_ROOT") {
            Some(r) => PathBuf::from(r),
            None => PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or(tr("LOCALAPPDATA is not set", "找不到 LOCALAPPDATA", "LOCALAPPDATA が見つかりません"))?),
        };
        let run_key = match std::env::var("HUD_TEST_RUNKEY") {
            Ok(k) => {
                let up = k.to_uppercase();
                let sub = ["HKCU\\", "HKEY_CURRENT_USER\\"].iter().find(|p| up.starts_with(*p)).map(|p| k[p.len()..].to_string());
                sub.ok_or("HUD_TEST_RUNKEY must start with HKCU\\")?
            }
            Err(_) => r"Software\Microsoft\Windows\CurrentVersion\Run".to_string(),
        };
        let dir = root.join("TelemetryHUD");
        Ok(Ctx { exe: dir.join(EXE), dir, run_key })
    }

    fn run_value(&self) -> Option<String> {
        reg_string(HKEY_CURRENT_USER, &self.run_key, VALUE)
    }

    /// Run 值存在且指向安装路径，且安装文件还在
    pub fn is_installed(&self) -> bool {
        let target = self.exe.to_string_lossy().to_lowercase();
        self.run_value().is_some_and(|v| v.to_lowercase().contains(&target)) && self.exe.exists()
    }

    /// 已安装的运行方式（从 Run 值里的参数判断）
    pub fn installed_mode(&self) -> Option<Mode> {
        let v = self.run_value()?;
        Some(if v.contains(Mode::Wallpaper.arg()) { Mode::Wallpaper } else { Mode::Helper })
    }

    pub fn is_running(&self) -> bool {
        !pids_by_path(&self.exe).is_empty()
    }

    /// 从安装位置启动后台实例（继承环境变量，测试时 HUD_PORT 会传下去）
    pub fn start(&self, mode: Mode) -> Result<(), String> {
        // 自己的标准句柄如果可继承（例如 PowerShell 管道），后台实例会一直持有它，调用方的管道就永远等不到结束
        unsafe {
            for h in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                SetHandleInformation(GetStdHandle(h), HANDLE_FLAG_INHERIT, 0);
            }
        }
        // 调用方在 Job 里（IDE / Agent 的终端）时，后台实例默认会留在同一个 Job，终端一关就被连带杀掉；
        // 所以先尝试脱离 Job，Job 不允许脱离时 CreateProcess 会失败，再退回普通启动
        let spawn = |flags: u32| {
            Command::new(&self.exe)
                .arg(mode.arg())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(flags)
                .spawn()
        };
        spawn(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
            .or_else(|_| spawn(DETACHED_PROCESS | CREATE_NO_WINDOW))
            .map(|_| ())
            .map_err(|e| tr("Failed to start: {e}", "启动失败：{e}", "起動に失敗しました：{e}").replace("{e}", &e.to_string()))
    }

    pub fn install(&self, mode: Mode) -> Result<(), String> {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.dir).map_err(|e| {
            tr("Cannot create {p}: {e}", "无法创建 {p}：{e}", "{p} を作成できません：{e}")
                .replace("{p}", &self.dir.display().to_string())
                .replace("{e}", &e.to_string())
        })?;
        // 覆盖安装（升级 / 切换运行方式）：先停掉安装位置正在运行的实例
        kill_by_path(&self.exe);
        if !same_path(&me, &self.exe) {
            retry(|| std::fs::copy(&me, &self.exe).map(|_| ())).map_err(|e| tr("Copy failed: {e}", "复制失败：{e}", "コピーに失敗しました：{e}").replace("{e}", &e.to_string()))?;
        }
        let cmd = wide(&format!("\"{}\" {}", self.exe.display(), mode.arg()));
        let r = unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, wide(&self.run_key).as_ptr(), wide(VALUE).as_ptr(), REG_SZ, cmd.as_ptr() as _, (cmd.len() * 2) as u32)
        };
        if r != 0 {
            return Err(tr("Failed to register autostart (error {r})", "写入自启注册表失败（错误 {r}）", "自動起動の登録に失敗しました（エラー {r}）").replace("{r}", &r.to_string()));
        }
        self.start(mode)
    }

    pub fn uninstall(&self) -> Result<(), String> {
        kill_by_path(&self.exe);
        let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(&self.run_key).as_ptr(), wide(VALUE).as_ptr()) };
        if r != 0 && r != 2 {
            return Err(tr("Failed to remove autostart (error {r})", "删除自启注册表失败（错误 {r}）", "自動起動の解除に失敗しました（エラー {r}）").replace("{r}", &r.to_string()));
        }
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        // 独立壁纸的 WebView2 数据（含控制面板设置）一起删掉；WebView2 子进程可能还要一会儿才退出
        let webview = self.dir.join("webview");
        if same_path(&me, &self.exe) {
            // 正在运行的就是安装文件本身：交给 cmd 延迟 2 秒后删除（目录非空时 rmdir 会失败，保留其他数据）
            let cmd = format!(
                "/c ping 127.0.0.1 -n 3 >nul & del /f /q \"{}\" & rmdir /s /q \"{}\" & rmdir \"{}\"",
                self.exe.display(),
                webview.display(),
                self.dir.display()
            );
            Command::new("cmd")
                .raw_arg(cmd)
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| tr("Cannot schedule deletion: {e}", "无法安排删除：{e}", "削除を予約できません：{e}").replace("{e}", &e.to_string()))?;
        } else {
            if self.exe.exists() {
                retry(|| std::fs::remove_file(&self.exe)).map_err(|e| {
                    tr("Failed to delete {p}: {e}", "删除 {p} 失败：{e}", "{p} を削除できません：{e}")
                        .replace("{p}", &self.exe.display().to_string())
                        .replace("{e}", &e.to_string())
                })?;
            }
            if webview.exists() {
                let _ = retry(|| std::fs::remove_dir_all(&webview));
            }
            // 只删空目录，不动用户自己放进去的其他文件
            let _ = std::fs::remove_dir(&self.dir);
        }
        Ok(())
    }
}

fn retry(mut f: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut last = Ok(());
    for _ in 0..15 {
        last = f();
        if last.is_ok() {
            return last;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    last
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

/// 镜像路径等于 exe 的进程（排除自己）。只按完整路径匹配，不会误伤其他位置运行的副本
fn pids_by_path(exe: &Path) -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(target) = std::fs::canonicalize(exe) else { return out };
    let target = target.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase();
    let me = std::process::id();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            if e.th32ProcessID != me && String::from_utf16_lossy(&e.szExeFile[..n]).eq_ignore_ascii_case(EXE) {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, e.th32ProcessID);
                if !h.is_null() {
                    let mut buf = [0u16; 1024];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0 && String::from_utf16_lossy(&buf[..len as usize]).to_lowercase() == target {
                        out.push(e.th32ProcessID);
                    }
                    CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

/// 独立壁纸先让它自己退出（移除托盘图标、销毁嵌在桌面里的窗口），3 秒没退出或只是数据服务就直接结束进程
fn kill_by_path(exe: &Path) {
    let pids = pids_by_path(exe);
    if pids.is_empty() {
        return;
    }
    let mut asked = Vec::new();
    unsafe {
        let class = wide(crate::desktop::CONTROL_CLASS);
        let mut w = std::ptr::null_mut();
        loop {
            w = FindWindowExW(std::ptr::null_mut(), w, class.as_ptr(), std::ptr::null());
            if w.is_null() {
                break;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(w, &mut pid);
            if pids.contains(&pid) {
                PostMessageW(w, WM_CLOSE, 0, 0);
                asked.push(pid);
            }
        }
        for pid in pids {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if !h.is_null() {
                if WaitForSingleObject(h, if asked.contains(&pid) { 3000 } else { 0 }) != WAIT_OBJECT_0 {
                    TerminateProcess(h, 0);
                    WaitForSingleObject(h, 3000);
                }
                CloseHandle(h);
            }
        }
    }
}

fn msg(text: &str, style: u32) -> i32 {
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(TITLE).as_ptr(), style) }
}

/// 无参数双击时的交互流程
pub fn interactive() {
    let ctx = match Ctx::new() {
        Ok(c) => c,
        Err(e) => {
            msg(&e, MB_OK | MB_ICONERROR);
            return;
        }
    };
    if ctx.is_installed() {
        let r = msg(
            tr(
                "Telemetry HUD is already installed.\n\nUninstall it?\nThis stops the background service, removes it from startup and deletes the installed program.\n\nChoose \"No\" to keep it (the service will be started if it isn't running).",
                "Telemetry HUD 已安装。\n\n要卸载吗？\n卸载会停止后台服务、取消开机自启，并删除已安装的程序。\n\n选“否”保持现状（如果服务没在运行，会把它启动起来）。",
                "Telemetry HUD はインストール済みです。\n\nアンインストールしますか？\nバックグラウンドサービスを停止し、自動起動を解除して、インストールしたプログラムを削除します。\n\n「いいえ」を選ぶとそのままにします（サービスが動いていなければ起動します）。",
            ),
            MB_YESNO | MB_ICONQUESTION,
        );
        if r == IDYES {
            match ctx.uninstall() {
                Ok(()) => msg(tr("Telemetry HUD has been uninstalled.", "Telemetry HUD 已卸载。", "Telemetry HUD をアンインストールしました。"), MB_OK | MB_ICONINFORMATION),
                Err(e) => msg(&tr("Uninstall failed: {e}", "卸载失败：{e}", "アンインストールに失敗しました：{e}").replace("{e}", &e), MB_OK | MB_ICONERROR),
            };
        } else if !ctx.is_running() {
            if let Err(e) = ctx.start(ctx.installed_mode().unwrap_or(Mode::Helper)) {
                msg(&e, MB_OK | MB_ICONERROR);
            }
        }
        return;
    }
    let mode = match msg(
        tr(
            "Install Telemetry HUD.\n\nShow it as your desktop wallpaper without Wallpaper Engine?\n\nYes: standalone wallpaper behind the desktop icons (includes the hardware helper).\nNo: hardware helper only, for the Wallpaper Engine or Lively wallpaper.",
            "安装 Telemetry HUD。\n\n要不用 Wallpaper Engine，直接把它显示为桌面壁纸吗？\n\n是：独立壁纸，显示在桌面图标后面（自带硬件数据服务）。\n否：只安装硬件数据服务，配合 Wallpaper Engine 或 Lively 的壁纸使用。",
            "Telemetry HUD をインストールします。\n\nWallpaper Engine を使わずに、デスクトップの壁紙として表示しますか？\n\nはい：デスクトップアイコンの後ろに表示する単体の壁紙（ハードウェアヘルパーを含む）。\nいいえ：Wallpaper Engine / Lively の壁紙で使うハードウェアヘルパーのみ。",
        ),
        MB_YESNOCANCEL | MB_ICONQUESTION,
    ) {
        IDYES => Mode::Wallpaper,
        IDNO => Mode::Helper,
        _ => return,
    };
    match ctx.install(mode) {
        Ok(()) => msg(
            &match mode {
                Mode::Helper => tr(
                    "Telemetry HUD is installed, set to start when you sign in, and now running in the background.\n\nLocation: {p}\n\nYou can delete the downloaded file now.\nTo uninstall, double-click this program again.",
                    "Telemetry HUD 已安装并设为开机自启，服务已在后台运行。\n\n安装位置：{p}\n\n现在可以删除下载的文件了。\n想卸载的话，再双击一次这个程序即可。",
                    "Telemetry HUD をインストールし、サインイン時に自動起動するよう設定しました。サービスはバックグラウンドで動作中です。\n\nインストール先：{p}\n\nダウンロードしたファイルは削除して構いません。\nアンインストールするには、このプログラムをもう一度ダブルクリックしてください。",
                ),
                Mode::Wallpaper => tr(
                    "Telemetry HUD is installed as your desktop wallpaper and set to start when you sign in.\n\nRight-click its tray icon to open the settings, pause it or exit.\n\nLocation: {p}\n\nYou can delete the downloaded file now.\nTo uninstall, double-click this program again.",
                    "Telemetry HUD 已作为桌面壁纸安装，并设为开机自启。\n\n右键托盘图标可以打开设置、暂停或退出。\n\n安装位置：{p}\n\n现在可以删除下载的文件了。\n想卸载的话，再双击一次这个程序即可。",
                    "Telemetry HUD をデスクトップの壁紙としてインストールし、サインイン時に自動起動するよう設定しました。\n\nタスクトレイのアイコンを右クリックすると、設定・一時停止・終了ができます。\n\nインストール先：{p}\n\nダウンロードしたファイルは削除して構いません。\nアンインストールするには、このプログラムをもう一度ダブルクリックしてください。",
                ),
            }
            .replace("{p}", &ctx.exe.display().to_string()),
            MB_OK | MB_ICONINFORMATION,
        ),
        Err(e) => msg(&tr("Install failed: {e}", "安装失败：{e}", "インストールに失敗しました：{e}").replace("{e}", &e), MB_OK | MB_ICONERROR),
    };
}

/// --install / --uninstall：静默，输出到 stdout，返回进程退出码。
/// --install 可带 --wallpaper / --background 指定运行方式；不带时沿用已安装的方式，新装默认只装数据服务
pub fn cli(install: bool, mode: Option<Mode>) -> i32 {
    let r = Ctx::new().and_then(|c| {
        if install {
            let mode = mode.or(c.installed_mode()).unwrap_or(Mode::Helper);
            c.install(mode).map(|_| format!("installed: {} {} (autostart: HKCU\\{}\\{VALUE})", c.exe.display(), mode.arg(), c.run_key))
        } else {
            c.uninstall().map(|_| format!("uninstalled: {}", c.exe.display()))
        }
    });
    match r {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
