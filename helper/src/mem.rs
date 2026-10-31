// 内存：动态数值（每秒）+ SMBIOS 内存条信息（启动时读一次）
use crate::gpu::r2;
use crate::pdh::{Counter, Query, FMT_DOUBLE};
use serde_json::{json, Map, Value};
use windows_sys::Win32::System::ProcessStatus::*;
use windows_sys::Win32::System::SystemInformation::*;

const GB: f64 = 1073741824.0;

pub struct Mem {
    standby: Vec<Counter>,
    modified: Option<Counter>,
    compress: Option<f64>,
    tick: u32,
    buf: Vec<u64>,
    pub static_info: Map<String, Value>,
}

impl Mem {
    pub fn new(q: &Query) -> Mem {
        let standby = ["Core", "Normal Priority", "Reserve"]
            .iter()
            .filter_map(|s| q.add(&format!(r"\Memory\Standby Cache {s} Bytes")))
            .collect();
        Mem {
            standby,
            modified: q.add(r"\Memory\Modified Page List Bytes"),
            compress: None,
            tick: 0,
            buf: Vec::new(),
            static_info: smbios_memory(),
        }
    }

    pub fn sample(&mut self, q: &Query) -> Value {
        let mut m = self.static_info.clone();
        unsafe {
            let mut s: MEMORYSTATUSEX = std::mem::zeroed();
            s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut s) != 0 {
                m.insert("used".into(), json!(r2((s.ullTotalPhys - s.ullAvailPhys) as f64 / GB)));
                m.insert("total".into(), json!(r2(s.ullTotalPhys as f64 / GB)));
                m.insert("available".into(), json!(r2(s.ullAvailPhys as f64 / GB)));
            }
            let mut p: PERFORMANCE_INFORMATION = std::mem::zeroed();
            p.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
            if K32GetPerformanceInfo(&mut p, p.cb) != 0 {
                let pg = |n: usize| json!(r2(n as f64 * p.PageSize as f64 / GB));
                m.insert("committed".into(), pg(p.CommitTotal));
                m.insert("commitLimit".into(), pg(p.CommitLimit));
                m.insert("pagedPool".into(), pg(p.KernelPaged));
                m.insert("nonPagedPool".into(), pg(p.KernelNonpaged));
            }
        }
        // 任务管理器的「已缓存」= 备用列表 + 已修改列表
        let sb: Option<f64> = self.standby.iter().map(|c| q.value(*c, FMT_DOUBLE)).sum();
        if let (Some(sb), Some(md)) = (sb, self.modified.and_then(|c| q.value(c, FMT_DOUBLE))) {
            m.insert("cached".into(), json!(r2((sb + md) / GB)));
        }
        if let Some(c) = self.compressed() {
            m.insert("compressed".into(), json!(r2(c / GB)));
        }
        Value::Object(m)
    }

    // 压缩内存 ≈ "Memory Compression" 进程的工作集。它是受保护进程不能 OpenProcess，
    // 所以从 NtQuerySystemInformation 的进程列表读；每 5 s 刷新一次
    fn compressed(&mut self) -> Option<f64> {
        if self.tick % 5 == 0 {
            self.compress = memory_compression_ws(&mut self.buf);
        }
        self.tick = self.tick.wrapping_add(1);
        self.compress
    }
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, buf: *mut std::ffi::c_void, len: u32, ret: *mut u32) -> i32;
}

fn memory_compression_ws(buf: &mut Vec<u64>) -> Option<f64> {
    let name: Vec<u16> = "Memory Compression".encode_utf16().collect();
    unsafe {
        for _ in 0..4 {
            let mut need = 0u32;
            let r = NtQuerySystemInformation(5, buf.as_mut_ptr() as _, (buf.len() * 8) as u32, &mut need);
            if r == 0xC000_0004u32 as i32 {
                // STATUS_INFO_LENGTH_MISMATCH：扩容后重试
                buf.resize(need as usize / 8 + 8192, 0);
                continue;
            }
            if r != 0 {
                return None;
            }
            let base = buf.as_ptr() as *const u8;
            let mut off = 0usize;
            loop {
                let e = base.add(off);
                // SYSTEM_PROCESS_INFORMATION (x64)：ImageName.Length @0x38，ImageName.Buffer @0x40，WorkingSetSize @0x90
                let len = (e.add(0x38) as *const u16).read_unaligned() as usize / 2;
                let ptr = (e.add(0x40) as *const *const u16).read_unaligned();
                if len == name.len() && !ptr.is_null() && std::slice::from_raw_parts(ptr, len) == &name[..] {
                    return Some((e.add(0x90) as *const usize).read_unaligned() as f64);
                }
                let next = (e as *const u32).read_unaligned() as usize;
                if next == 0 {
                    return None;
                }
                off += next;
            }
        }
        None
    }
}

/// 解析 SMBIOS 原始表：Type 16 (Physical Memory Array) 取插槽数，Type 17 (Memory Device) 取每条内存
/// （Win32_PhysicalMemory / Win32_PhysicalMemoryArray 读的就是这份数据，无需 COM / 管理员）
pub fn smbios_memory() -> Map<String, Value> {
    let mut out = Map::new();
    let (mut slots, mut modules) = (0u32, Vec::new());
    for st in crate::smbios::structures() {
        let ty = st.ty;
        let b = |i: usize| st.b(i);
        let w = |i: usize| st.w(i);
        let d = |i: usize| st.d(i);
        let s = |i: usize| st.s(i);
        match ty {
            16 if b(5) == Some(3) => slots += w(0x0D).unwrap_or(0) as u32, // Use = System memory
            17 => {
                let size_mb = match w(0x0C) {
                    None | Some(0) | Some(0xFFFF) => None,
                    Some(0x7FFF) => d(0x1C).map(|v| (v & 0x7FFF_FFFF) as u64),
                    Some(v) if v & 0x8000 != 0 => Some(((v & 0x7FFF) as u64) / 1024),
                    Some(v) => Some(v as u64),
                };
                let Some(size_mb) = size_mb else { continue }; // 空插槽
                let mut m = Map::new();
                let mut put = |k: &str, v: Option<Value>| {
                    if let Some(v) = v {
                        m.insert(k.into(), v);
                    }
                };
                put("slot", s(0x10).map(Value::from));
                put("bank", s(0x11).map(Value::from));
                put("capacity", Some(json!(r2(size_mb as f64 / 1024.0))));
                put(
                    "type",
                    match b(0x12) {
                        Some(0x18) => Some("DDR3"),
                        Some(0x1A) => Some("DDR4"),
                        Some(0x1E) => Some("LPDDR4"),
                        Some(0x22) => Some("DDR5"),
                        Some(0x23) => Some("LPDDR5"),
                        _ => None,
                    }
                    .map(Value::from),
                );
                // Speed 为 0xFFFF 时看 3.3 版的 Extended Speed
                let speed = |i: usize, ext: usize| match w(i) {
                    Some(0xFFFF) => d(ext).filter(|&v| v > 0).map(Value::from),
                    Some(v) if v > 0 => Some(json!(v)),
                    _ => None,
                };
                put("speed", speed(0x15, 0x54));
                put("configuredSpeed", speed(0x20, 0x58));
                put("voltage", w(0x26).filter(|&v| v > 0).map(|v| json!(v as f64 / 1000.0)));
                put(
                    "formFactor",
                    match b(0x0E) {
                        Some(0x09) => Some("DIMM"),
                        Some(0x0D) => Some("SODIMM"),
                        _ => None,
                    }
                    .map(Value::from),
                );
                put("manufacturer", s(0x17).map(Value::from));
                put("partNumber", s(0x1A).map(Value::from));
                put("serial", s(0x18).map(Value::from));
                modules.push(Value::Object(m));
            }
            _ => {}
        }
    }
    if slots > 0 {
        out.insert("slots".into(), json!(slots));
    }
    if !modules.is_empty() {
        out.insert("modules".into(), Value::Array(modules));
    }
    out
}
