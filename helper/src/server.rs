// 采样器与 HTTP 服务（hud-telemetry 和 hud-wallpaper 共用）
use crate::gpu::{self, r1, r2};
use crate::pdh::{Counter, Query, FMT_DOUBLE, FMT_NOCAP};
use crate::{lhm, mem, sysinfo};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Arc<Vec<u8>>>>;
/// 可选：GET / 返回的页面（每次请求调用，便于不重编译就更新页面）
pub type PageFn = Arc<dyn Fn() -> Vec<u8> + Send + Sync>;

struct Sampler {
    q: Query,
    c_util: Option<Counter>,
    c_perf: Option<Counter>,
    c_freq: Option<Counter>,
    c_dr: Option<Counter>,
    c_dw: Option<Counter>,
    c_idle: Option<Counter>,
    c_geng: Option<Counter>,
    c_gmem: Option<Counter>,
    cpu: sysinfo::CpuStatic,
    mem: mem::Mem,
    cpu_spec: Value,
    nvml: Option<gpu::Nvml>,
    gpu_static: Map<String, Value>,
    gpu_spec: Map<String, Value>,
    host: Option<String>,
    vols: Vec<(String, f64, f64)>,
    vol_types: HashMap<String, &'static str>,
    vols_at: Option<Instant>,
    health: HashMap<String, Value>,
    health_at: Option<Instant>,
    net_prev: HashMap<u64, (u64, u64)>,
    net_at: Instant,
    lhm: Arc<lhm::Lhm>,
    emi: crate::emi::Emi,
    board: crate::board::Board,
    board_val: Value,
    board_at: Option<Instant>,
}

impl Sampler {
    fn new(lhm_port: u16) -> Sampler {
        let q = Query::new().expect("PdhOpenQuery failed");
        let cpu = sysinfo::cpu_static();
        let mut spec = Map::new();
        if cpu.p_cores > 0 {
            spec.insert("pCores".into(), json!(cpu.p_cores));
        }
        if cpu.e_cores > 0 {
            spec.insert("eCores".into(), json!(cpu.e_cores));
        }
        if cpu.threads > 0 {
            spec.insert("threads".into(), json!(cpu.threads));
        }
        if let Some(b) = cpu.base_ghz {
            spec.insert("baseClock".into(), json!(r2(b)));
        }
        if cpu.l3_mb > 0.0 {
            spec.insert("cacheMB".into(), json!(r1(cpu.l3_mb)));
        }

        let nvml = if std::env::var_os("HUD_NO_NVML").is_some() { None } else { gpu::Nvml::load() };
        let (gpu_static, gpu_spec, c_geng, c_gmem) = match &nvml {
            Some(n) => (n.static_info.clone(), n.spec.clone(), None, None),
            None => {
                let (i, s) = gpu::registry_gpu();
                // 只在没有 NVML 时才挂 GPU Engine 通配计数器（实例多，开销相对大）
                (i, s, q.add(r"\GPU Engine(*)\Utilization Percentage"), q.add(r"\GPU Adapter Memory(*)\Dedicated Usage"))
            }
        };

        let mem = mem::Mem::new(&q);
        let s = Sampler {
            c_util: q.add(r"\Processor Information(*)\% Processor Utility"),
            c_perf: q.add(r"\Processor Information(_Total)\% Processor Performance"),
            c_freq: q.add(r"\Processor Information(_Total)\Processor Frequency"),
            c_dr: q.add(r"\LogicalDisk(*)\Disk Read Bytes/sec"),
            c_dw: q.add(r"\LogicalDisk(*)\Disk Write Bytes/sec"),
            c_idle: q.add(r"\LogicalDisk(*)\% Idle Time"),
            c_geng,
            c_gmem,
            q,
            mem,
            cpu,
            cpu_spec: Value::Object(spec),
            nvml,
            gpu_static,
            gpu_spec,
            host: sysinfo::hostname().or_else(|| std::env::var("COMPUTERNAME").ok()),
            vols: Vec::new(),
            vol_types: HashMap::new(),
            vols_at: None,
            health: HashMap::new(),
            health_at: None,
            net_prev: sysinfo::net_octets(),
            net_at: Instant::now(),
            lhm: lhm::spawn(lhm_port),
            emi: crate::emi::Emi::new(),
            board: crate::board::Board::new(),
            board_val: Value::Null,
            board_at: None,
        };
        s.q.collect();
        s
    }

    /// 电源：AC / 电池 / 计划 / 模式，加上实测功率（只用真实计量，绝不估算）
    fn power(&mut self) -> Value {
        let mut p = sysinfo::power();
        let (emi_cpu, emi_sys) = self.emi.sample();
        let (cpu, cpu_src) = match (emi_cpu, self.lhm.power()) {
            (Some(w), _) => (Some(w), Some("emi")),
            (None, Some(w)) => (Some(w), Some("lhm")),
            _ => (None, None),
        };
        // 用电池供电时取放电功率；否则看 EMI 的平台 / PSYS 通道
        let on_battery = p["ac"] == Value::Bool(false) && !p["battery"].is_null();
        let (sys, sys_src) = match (on_battery.then(crate::emi::battery_discharge_w).flatten(), emi_sys) {
            (Some(w), _) => (Some(w), Some("battery")),
            (None, Some(w)) => (Some(w), Some("emi")),
            _ => (None, None),
        };
        p["cpuW"] = json!(cpu);
        p["systemW"] = json!(sys);
        p["source"] = json!({ "cpuW": cpu_src, "systemW": sys_src });
        p
    }

    fn sample(&mut self) -> Value {
        self.q.collect();
        let mut root = Map::new();
        if let Some(h) = &self.host {
            root.insert("host".into(), json!(h));
        }
        root.insert("uptime".into(), json!(sysinfo::uptime()));

        // 主板：SMBIOS 启动时读一次，PCI 设备每 30 s 重扫
        if self.board_at.map_or(true, |t| t.elapsed() > Duration::from_secs(30)) {
            self.board_val = self.board.scan();
            self.board_at = Some(Instant::now());
        }
        root.insert("board".into(), self.board_val.clone());

        // CPU
        let mut cpu = Map::new();
        if let Some(c) = self.c_util {
            let mut per: Vec<((u16, u8), f64)> = Vec::new();
            for (name, v) in self.q.array(c, FMT_NOCAP) {
                let v = v.clamp(0.0, 100.0);
                if name == "_Total" {
                    cpu.insert("usage".into(), json!(r1(v)));
                } else if let Some((g, n)) = name.split_once(',') {
                    if let (Ok(g), Ok(n)) = (g.parse(), n.parse()) {
                        per.push(((g, n), v));
                    }
                }
            }
            per.sort_by_key(|p| p.0);
            if !per.is_empty() {
                cpu.insert("cores".into(), json!(per.iter().map(|p| r1(p.1)).collect::<Vec<_>>()));
                if !self.cpu.types.is_empty() {
                    let t: Vec<String> = per.iter().map(|p| self.cpu.types.get(&p.0).copied().unwrap_or('P').to_string()).collect();
                    cpu.insert("coreTypes".into(), json!(t));
                }
            }
        }
        if let (Some(p), Some(f)) = (self.c_perf.and_then(|c| self.q.value(c, FMT_NOCAP)), self.c_freq.and_then(|c| self.q.value(c, FMT_DOUBLE))) {
            if f > 0.0 {
                cpu.insert("freq".into(), json!(r2(p / 100.0 * f / 1000.0)));
            }
        }
        if let Some(t) = self.lhm.temp() {
            cpu.insert("temp".into(), json!(t));
        }
        if let Some(n) = &self.cpu.name {
            cpu.insert("name".into(), json!(n));
        }
        cpu.insert("spec".into(), self.cpu_spec.clone());
        root.insert("cpu".into(), Value::Object(cpu));

        // GPU
        let mut g = self.gpu_static.clone();
        match &self.nvml {
            Some(n) => n.sample(&mut g),
            None => {
                if let Some(c) = self.c_geng {
                    let mut eng: HashMap<String, f64> = HashMap::new();
                    for (name, v) in self.q.array(c, FMT_DOUBLE) {
                        if let Some(i) = name.find("luid_") {
                            *eng.entry(name[i..].to_string()).or_default() += v;
                        }
                    }
                    if !eng.is_empty() {
                        let m = eng.values().cloned().fold(0.0, f64::max).min(100.0);
                        g.insert("usage".into(), json!(r1(m)));
                    }
                }
                if let Some(c) = self.c_gmem {
                    let m = self.q.array(c, FMT_DOUBLE).into_iter().map(|x| x.1).fold(0.0, f64::max);
                    if m > 0.0 {
                        g.insert("vramUsed".into(), json!(r2(m / 1073741824.0)));
                    }
                }
            }
        }
        if !self.gpu_spec.is_empty() {
            g.insert("spec".into(), Value::Object(self.gpu_spec.clone()));
        }
        if !g.is_empty() {
            root.insert("gpu".into(), Value::Object(g));
        }

        // 内存
        root.insert("mem".into(), self.mem.sample(&self.q));

        // 磁盘：容量每 10 s 刷新一次，读写速率每秒
        if self.vols_at.map_or(true, |t| t.elapsed() > Duration::from_secs(10)) {
            let vols = sysinfo::fixed_volumes();
            // 卷集合变化（含启动时）才重新探测盘类型
            let changed = vols.iter().map(|v| &v.0).ne(self.vols.iter().map(|v| &v.0));
            if changed {
                self.vol_types = vols.iter().filter_map(|v| Some((v.0.clone(), sysinfo::disk_type(&v.0)?))).collect();
            }
            self.vols = vols;
            self.vols_at = Some(Instant::now());
            // 寿命信息每 5 分钟读一次（卷集合变化时立即重读）
            if changed || self.health_at.map_or(true, |t| t.elapsed() > Duration::from_secs(300)) {
                self.health = self.vols.iter().filter_map(|v| Some((v.0.clone(), crate::health::read(&v.0, self.vol_types.get(&v.0).copied())?))).collect();
                self.health_at = Some(Instant::now());
            }
        }
        let rd: HashMap<String, f64> = self.c_dr.map(|c| self.q.array(c, FMT_DOUBLE).into_iter().collect()).unwrap_or_default();
        let wr: HashMap<String, f64> = self.c_dw.map(|c| self.q.array(c, FMT_DOUBLE).into_iter().collect()).unwrap_or_default();
        let idle: HashMap<String, f64> = self.c_idle.map(|c| self.q.array(c, FMT_NOCAP).into_iter().collect()).unwrap_or_default();
        let disks: Vec<Value> = self
            .vols
            .iter()
            .map(|(name, used, total)| {
                let mut d = json!({ "name": name, "used": r1(*used), "total": r1(*total) });
                if let Some(t) = self.vol_types.get(name) {
                    d["type"] = json!(t);
                }
                if let Some(h) = self.health.get(name) {
                    d["health"] = h.clone();
                }
                // 每个卷单独的读写速率；计数器可用但该实例暂缺时记 0
                if self.c_dr.is_some() {
                    d["read"] = json!(rd.get(name).map_or(0, |v| v.round() as u64));
                }
                if self.c_dw.is_some() {
                    d["write"] = json!(wr.get(name).map_or(0, |v| v.round() as u64));
                }
                // 活动时间 = 100 − % Idle Time（该计数器可能 >100 或 <0，需钳制）
                if let Some(idle) = idle.get(name) {
                    d["busy"] = json!(r1((100.0 - idle).clamp(0.0, 100.0)));
                }
                d
            })
            .collect();
        root.insert("disks".into(), json!(disks));

        // 网络
        let now = sysinfo::net_octets();
        let dt = self.net_at.elapsed().as_secs_f64().max(0.001);
        let (mut down, mut up) = (0u64, 0u64);
        for (k, (i, o)) in &now {
            if let Some((pi, po)) = self.net_prev.get(k) {
                down += i.saturating_sub(*pi);
                up += o.saturating_sub(*po);
            }
        }
        self.net_prev = now;
        self.net_at = Instant::now();
        root.insert("net".into(), json!({ "up": (up as f64 / dt).round() as u64, "down": (down as f64 / dt).round() as u64 }));
        root.insert("power".into(), self.power());

        Value::Object(root)
    }
}

fn handle(mut s: TcpStream, snap: &Shared, page: Option<&PageFn>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 8192];
    let mut n = 0;
    while n < buf.len() {
        match s.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf[..n]);
    let mut parts = head.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = path.split('?').next().unwrap_or("");
    const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nAccess-Control-Allow-Private-Network: true\r\nCache-Control: no-store\r\nConnection: close\r\n";
    let resp: Vec<u8> = match (method, path) {
        ("OPTIONS", _) => format!("HTTP/1.1 204 No Content\r\n{CORS}Access-Control-Max-Age: 86400\r\nContent-Length: 0\r\n\r\n").into_bytes(),
        ("GET", "/") if page.is_some() => {
            let body = page.unwrap()();
            let mut r = format!("HTTP/1.1 200 OK
Cache-Control: no-store
Connection: close
Content-Type: text/html; charset=utf-8
Content-Length: {}

", body.len()).into_bytes();
            r.extend_from_slice(&body);
            r
        }
        ("GET", "/stats") | ("GET", "/") => {
            let body = snap.lock().unwrap().clone();
            let mut r = format!("HTTP/1.1 200 OK\r\n{CORS}Content-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
            r.extend_from_slice(&body);
            r
        }
        _ => format!("HTTP/1.1 404 Not Found\r\n{CORS}Content-Length: 0\r\n\r\n").into_bytes(),
    };
    let _ = s.write_all(&resp);
}


/// 后台采样线程，返回共享快照
pub fn spawn_sampler(lhm_port: u16) -> Shared {
    let snap: Shared = Arc::new(Mutex::new(Arc::new(b"{}".to_vec())));
    let sw = snap.clone();
    std::thread::spawn(move || {
        let mut s = Sampler::new(lhm_port);
        std::thread::sleep(Duration::from_millis(500));
        loop {
            let t0 = Instant::now();
            let v = s.sample();
            *sw.lock().unwrap() = Arc::new(serde_json::to_vec(&v).unwrap_or_default());
            std::thread::sleep(Duration::from_secs(1).saturating_sub(t0.elapsed()));
        }
    });
    snap
}

/// 阻塞地处理 HTTP 连接
pub fn serve(listener: TcpListener, snap: Shared, page: Option<PageFn>) {
    for s in listener.incoming().flatten() {
        let (snap, page) = (snap.clone(), page.clone());
        std::thread::spawn(move || handle(s, &snap, page.as_ref()));
    }
}
