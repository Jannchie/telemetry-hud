// 可选：从 LibreHardwareMonitor 的 Web 服务读 CPU 封装温度与封装功率
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

const NONE: u32 = u32::MAX;

pub struct Lhm {
    temp: AtomicU32,
    power: AtomicU32,
}

impl Lhm {
    pub fn temp(&self) -> Option<f64> {
        load(&self.temp)
    }
    pub fn power(&self) -> Option<f64> {
        load(&self.power)
    }
}

fn load(a: &AtomicU32) -> Option<f64> {
    let v = a.load(Ordering::Relaxed);
    (v != NONE).then(|| v as f64 / 10.0)
}

fn store(a: &AtomicU32, v: Option<f64>) {
    a.store(v.map_or(NONE, |f| (f * 10.0).round().max(0.0) as u32), Ordering::Relaxed);
}

/// 后台线程：LHM 在线时每 2 s 读一次，离线时每 15 s 探测一次
pub fn spawn(port: u16) -> Arc<Lhm> {
    let l = Arc::new(Lhm { temp: AtomicU32::new(NONE), power: AtomicU32::new(NONE) });
    let t = l.clone();
    std::thread::spawn(move || loop {
        let r = fetch(port);
        let (temp, power) = r.unwrap_or((None, None));
        store(&t.temp, temp);
        store(&t.power, power);
        std::thread::sleep(Duration::from_secs(if r.is_some() { 2 } else { 15 }));
    });
    l
}

fn fetch(port: u16) -> Option<(Option<f64>, Option<f64>)> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    s.write_all(b"GET /data.json HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n").ok()?;
    let mut body = Vec::new();
    s.take(4 << 20).read_to_end(&mut body).ok()?;
    let i = body.windows(4).position(|w| w == b"\r\n\r\n")?;
    let root: Value = serde_json::from_slice(&body[i + 4..]).ok()?;
    let (mut temp, mut power) = (None, None);
    walk(&root, false, &mut temp, &mut power);
    Some((temp.map(|b| b.1), power.map(|b| b.1)))
}

fn number(v: &str) -> Option<f64> {
    v.replace(',', ".").chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect::<String>().parse().ok()
}

// 在 CPU 硬件节点下找：温度优先 "CPU Package"（Intel）/ "Core (Tctl/Tdie)"（AMD）；功率取 "CPU Package" / "Package"
fn walk(n: &Value, in_cpu: bool, temp: &mut Option<(u8, f64)>, power: &mut Option<(u8, f64)>) {
    let text = n.get("Text").and_then(Value::as_str).unwrap_or("");
    let img = n.get("ImageURL").and_then(Value::as_str).unwrap_or("");
    let in_cpu = in_cpu || img.contains("cpu");
    if in_cpu {
        if let Some(val) = n.get("Value").and_then(Value::as_str) {
            if val.contains("°C") {
                let rank = match text {
                    "CPU Package" => 3,
                    "Core (Tctl/Tdie)" => 2,
                    "Package" | "Core Max" => 1,
                    _ => 0,
                };
                if rank > 0 && temp.map_or(true, |b| rank > b.0) {
                    if let Some(f) = number(val) {
                        *temp = Some((rank, f));
                    }
                }
            } else if val.trim_end().ends_with(" W") {
                let rank = match text {
                    "CPU Package" => 2,
                    "Package" => 1,
                    _ => 0,
                };
                if rank > 0 && power.map_or(true, |b| rank > b.0) {
                    if let Some(f) = number(val) {
                        *power = Some((rank, f));
                    }
                }
            }
        }
    }
    if let Some(ch) = n.get("Children").and_then(Value::as_array) {
        for c in ch {
            walk(c, in_cpu, temp, power);
        }
    }
}
