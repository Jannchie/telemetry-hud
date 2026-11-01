// release 构建不弹控制台窗口（适合开机自启）；debug 构建保留控制台
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod install;

use hud_telemetry::{server, sysinfo};
use std::net::TcpListener;

/// windows 子系统下没有控制台；命令行调用时如果 stdout 没被重定向，就挂到父控制台上输出
fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h.is_null() || h as isize == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);

    // 无参数（双击）→ 安装 / 卸载对话框
    if args.len() == 1 {
        install::interactive();
        return;
    }
    if has("--install") || has("--uninstall") {
        attach_console();
        let mode = if has("--wallpaper") {
            Some(install::Mode::Wallpaper)
        } else if has("--background") {
            Some(install::Mode::Helper)
        } else {
            None
        };
        std::process::exit(install::cli(has("--install"), mode));
    }
    if has("--probe-disks") || has("--probe-health") || has("--probe-board") {
        attach_console();
    }

    let mut port = std::env::var("HUD_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(server::DEFAULT_PORT);
    let mut lhm_port = 8085u16;
    for w in args.windows(2) {
        match w[0].as_str() {
            "--port" => port = w[1].parse().unwrap_or(port),
            "--lhm-port" => lhm_port = w[1].parse().unwrap_or(lhm_port),
            _ => {}
        }
    }
    if args.iter().any(|a| a == "--probe-disks") {
        for (v, used, total) in sysinfo::fixed_volumes() {
            println!("{v} {used:.1}/{total:.1} GB type={:?}", sysinfo::disk_type(&v));
        }
        return;
    }
    if args.iter().any(|a| a == "--probe-health") {
        for (v, _, _) in sysinfo::fixed_volumes() {
            let kind = sysinfo::disk_type(&v);
            let t = std::time::Instant::now();
            let h = hud_telemetry::health::read(&v, kind);
            println!("{v} {kind:?} health={} ({:?})", h.map_or("<omitted>".into(), |h| h.to_string()), t.elapsed());
        }
        println!("power={}", sysinfo::power());
        let mut emi = hud_telemetry::emi::Emi::new();
        emi.sample();
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            println!("emi (cpuW, systemW) = {:?}", emi.sample());
        }
        return;
    }
    if args.iter().any(|a| a == "--probe-board") {
        let t = std::time::Instant::now();
        let b = hud_telemetry::board::Board::new();
        let t_smbios = t.elapsed();
        let t = std::time::Instant::now();
        let v = b.scan();
        let t_scan = t.elapsed();
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        println!("smbios={t_smbios:?} pci_scan={t_scan:?}");
        return;
    }
    // 独立壁纸：嵌到桌面图标后面，自带数据服务
    if has("--wallpaper") {
        desktop::run(port, lhm_port);
        return;
    }
    // 以下是服务模式（--background，或带 --port / --lhm-port 等参数）
    // 端口被占用（例如已有实例在跑）就直接退出
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 127.0.0.1:{port} failed: {e}");
            std::process::exit(1);
        }
    };
    let snap = server::spawn_sampler(lhm_port);
    eprintln!("hud-telemetry listening on http://127.0.0.1:{port}/stats");
    server::serve(listener, snap);
}
