// 静态信息（启动时读一次）+ 内存 / 磁盘 / 网络的轻量采样
use crate::pdh::wide;
use std::collections::HashMap;
use windows_sys::Win32::NetworkManagement::IpHelper::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Power::CallNtPowerInformation;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::SystemInformation::*;

pub fn reg_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let (k, n) = (wide(key), wide(name));
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(root, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as _, &mut len)
    };
    if r != 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize / 2)]);
    Some(s.trim_end_matches('\0').trim().to_string())
}

pub fn reg_u64(root: HKEY, key: &str, name: &str) -> Option<u64> {
    let (k, n) = (wide(key), wide(name));
    let mut buf = [0u8; 8];
    let mut len = 8u32;
    let r = unsafe { RegGetValueW(root, k.as_ptr(), n.as_ptr(), RRF_RT_ANY, std::ptr::null_mut(), buf.as_mut_ptr() as _, &mut len) };
    if r != 0 {
        return None;
    }
    Some(match len {
        4 => u32::from_le_bytes(buf[..4].try_into().unwrap()) as u64,
        8 => u64::from_le_bytes(buf),
        _ => return None,
    })
}

pub struct CpuStatic {
    pub name: Option<String>,
    /// (group, number) -> 'P' / 'E'；非混合架构为空
    pub types: HashMap<(u16, u8), char>,
    pub p_cores: u32,
    pub e_cores: u32,
    pub threads: u32,
    pub l3_mb: f64,
    pub base_ghz: Option<f64>,
}

pub fn cpu_static() -> CpuStatic {
    let name = reg_string(HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString");

    // GetSystemCpuSetInformation：每个逻辑处理器的 EfficiencyClass（数值越大越偏性能）
    let mut sets: Vec<(u16, u8, u8)> = Vec::new();
    unsafe {
        let mut len = 0u32;
        GetSystemCpuSetInformation(std::ptr::null_mut(), 0, &mut len, std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; len as usize];
        if len > 0 && GetSystemCpuSetInformation(buf.as_mut_ptr() as _, len, &mut len, std::ptr::null_mut(), 0) != 0 {
            let mut off = 0usize;
            while off + 20 <= len as usize {
                let size = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                let ty = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap());
                if ty == 0 {
                    let group = u16::from_le_bytes(buf[off + 12..off + 14].try_into().unwrap());
                    let lp = buf[off + 14];
                    let eff = buf[off + 18];
                    sets.push((group, lp, eff));
                }
                if size == 0 {
                    break;
                }
                off += size;
            }
        }
    }
    let max_eff = sets.iter().map(|s| s.2).max().unwrap_or(0);
    let min_eff = sets.iter().map(|s| s.2).min().unwrap_or(0);
    let hybrid = max_eff != min_eff;
    let types = if hybrid {
        sets.iter().map(|&(g, n, e)| ((g, n), if e == max_eff { 'P' } else { 'E' })).collect()
    } else {
        HashMap::new()
    };

    // 物理核心（按 EfficiencyClass 分 P/E）与 L3
    let (mut p_cores, mut e_cores, mut threads, mut l3) = (0u32, 0u32, 0u32, 0u64);
    unsafe {
        let mut len = 0u32;
        GetLogicalProcessorInformationEx(RelationAll, std::ptr::null_mut(), &mut len);
        let mut buf = vec![0u8; len as usize];
        if len > 0 && GetLogicalProcessorInformationEx(RelationAll, buf.as_mut_ptr() as _, &mut len) != 0 {
            let mut off = 0usize;
            while off + 8 <= len as usize {
                let rel = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
                let size = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap()) as usize;
                match rel {
                    0 => {
                        // RelationProcessorCore: Flags, EfficiencyClass
                        let smt = buf[off + 8] & 1 != 0;
                        let eff = buf[off + 9];
                        threads += if smt { 2 } else { 1 };
                        if !hybrid || eff == max_eff { p_cores += 1 } else { e_cores += 1 }
                    }
                    2 => {
                        // RelationCache: Level, Associativity, LineSize, CacheSize
                        if buf[off + 8] == 3 {
                            l3 += u32::from_le_bytes(buf[off + 12..off + 16].try_into().unwrap()) as u64;
                        }
                    }
                    _ => {}
                }
                if size == 0 {
                    break;
                }
                off += size;
            }
        }
    }
    if !sets.is_empty() {
        threads = sets.len() as u32;
    }

    // PROCESSOR_POWER_INFORMATION.MaxMhz ≈ 基础频率
    let base_ghz = unsafe {
        let n = sets.len().max(1);
        let mut buf = vec![0u32; n * 6];
        let r = CallNtPowerInformation(11, std::ptr::null(), 0, buf.as_mut_ptr() as _, (buf.len() * 4) as u32);
        (r == 0 && buf[1] > 0).then(|| buf[1] as f64 / 1000.0)
    };

    CpuStatic { name, types, p_cores, e_cores, threads, l3_mb: l3 as f64 / 1048576.0, base_ghz }
}

pub fn uptime() -> u64 {
    unsafe { GetTickCount64() / 1000 }
}

/// 固定盘：[(盘符 "C:", 已用 GB, 总 GB)]
pub fn fixed_volumes() -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            continue; // DRIVE_FIXED
        }
        let (mut _avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut _avail, &mut total, &mut free) } == 0 || total == 0 {
            continue;
        }
        let gb = 1073741824.0;
        out.push((format!("{letter}:"), (total - free) as f64 / gb, total as f64 / gb));
    }
    out
}

/// 物理网卡累计字节数 {luid: (in, out)}
pub fn net_octets() -> HashMap<u64, (u64, u64)> {
    let mut out = HashMap::new();
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t) != 0 || t.is_null() {
            return out;
        }
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
        for r in rows {
            let f = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = f & 1 != 0;
            let filter = f & 2 != 0;
            // OperStatus 1 = IfOperStatusUp
            if hardware && !filter && r.OperStatus == 1 {
                out.insert(r.InterfaceLuid.Value, (r.InOctets, r.OutOctets));
            }
        }
        FreeMibTable(t as _);
    }
    out
}

pub fn hostname() -> Option<String> {
    let mut buf = [0u16; 256];
    let mut n = buf.len() as u32;
    let ok = unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut n) };
    (ok != 0 && n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

// 卷 → 物理盘号 → 总线类型 / 寻道惩罚，判断 NVMe / SSD / HDD（与 MSFT_PhysicalDisk 同源，无需管理员）
pub(crate) unsafe fn ioctl(path: &str, code: u32, inp: &[u8], out: &mut [u8]) -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let w = wide(path);
    let h = CreateFileW(w.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
    if h == INVALID_HANDLE_VALUE {
        if std::env::var_os("HUD_DEBUG").is_some() {
            eprintln!("open {path}: {}", std::io::Error::last_os_error());
        }
        return None;
    }
    let mut n = 0u32;
    let ok = DeviceIoControl(h, code, inp.as_ptr() as _, inp.len() as u32, out.as_mut_ptr() as _, out.len() as u32, &mut n, std::ptr::null_mut());
    if ok == 0 && std::env::var_os("HUD_DEBUG").is_some() {
        eprintln!("ioctl {path} {code:#x}: {}", std::io::Error::last_os_error());
    }
    CloseHandle(h);
    (ok != 0).then_some(n)
}

pub(crate) const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;

/// 卷（"C:"）→ 所在物理盘号（跨盘卷取第一块）
pub fn disk_number(volume: &str) -> Option<u32> {
    const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
    let mut ext = [0u8; 256];
    unsafe { ioctl(&format!("\\\\.\\{volume}"), IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut ext)? };
    if u32::from_le_bytes(ext[..4].try_into().unwrap()) == 0 {
        return None;
    }
    Some(u32::from_le_bytes(ext[8..12].try_into().unwrap()))
}

pub fn disk_type(volume: &str) -> Option<&'static str> {
    unsafe {
        let disk = disk_number(volume)?;
        let drive = format!("\\\\.\\PhysicalDrive{disk}");
        // STORAGE_PROPERTY_QUERY { PropertyId, QueryType = 0 }
        let q = |id: u32| {
            let mut b = [0u8; 12];
            b[..4].copy_from_slice(&id.to_le_bytes());
            b
        };
        let mut dev = [0u8; 1024];
        if ioctl(&drive, IOCTL_STORAGE_QUERY_PROPERTY, &q(0), &mut dev).is_some() {
            // STORAGE_DEVICE_DESCRIPTOR.BusType @28；17 = BusTypeNvme
            if u32::from_le_bytes(dev[28..32].try_into().unwrap()) == 17 {
                return Some("NVMe");
            }
        }
        let mut sp = [0u8; 12];
        // StorageDeviceSeekPenaltyProperty = 7 → DEVICE_SEEK_PENALTY_DESCRIPTOR.IncursSeekPenalty @8
        ioctl(&drive, IOCTL_STORAGE_QUERY_PROPERTY, &q(7), &mut sp)?;
        Some(if sp[8] != 0 { "HDD" } else { "SSD" })
    }
}

/// 交流电 / 电池状态（GetSystemPowerStatus，开销极小，每秒采样）
pub fn power() -> serde_json::Value {
    use serde_json::{json, Value};
    let mut s: windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut s) } == 0 {
        let (plan, mode) = power_plan();
        return json!({ "ac": null, "battery": null, "plan": plan, "mode": mode });
    }
    let ac = match s.ACLineStatus {
        0 => Value::Bool(false),
        1 => Value::Bool(true),
        _ => Value::Null, // 255 = 未知
    };
    // BatteryFlag：128 = 没有系统电池，255 = 状态未知
    let battery = if s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 {
        Value::Null
    } else {
        json!({
            "percent": if s.BatteryLifePercent <= 100 { json!(s.BatteryLifePercent) } else { Value::Null },
            "charging": s.BatteryFlag & 8 != 0,
            "remainingMin": if s.BatteryLifeTime == u32::MAX { Value::Null } else { json!(s.BatteryLifeTime / 60) },
        })
    };
    let (plan, mode) = power_plan();
    json!({ "ac": ac, "battery": battery, "plan": plan, "mode": mode })
}

/// 电源计划名称 + Windows 11 电源模式（叠加方案），缓存 10 s
fn power_plan() -> (serde_json::Value, serde_json::Value) {
    use serde_json::Value;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Value, Value)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    if let Some((t, p, m)) = c.as_ref() {
        if t.elapsed() < Duration::from_secs(10) {
            return (p.clone(), m.clone());
        }
    }
    let plan = active_scheme_name().map_or(Value::Null, Value::from);
    let mode = overlay_mode().map_or(Value::Null, Value::from);
    *c = Some((Instant::now(), plan.clone(), mode.clone()));
    (plan, mode)
}

fn active_scheme_name() -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::System::Power::{PowerGetActiveScheme, PowerReadFriendlyName};
    unsafe {
        let mut g: *mut windows_sys::core::GUID = std::ptr::null_mut();
        if PowerGetActiveScheme(std::ptr::null_mut(), &mut g) != 0 || g.is_null() {
            return None;
        }
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        let r = PowerReadFriendlyName(std::ptr::null_mut(), g, std::ptr::null(), std::ptr::null(), buf.as_mut_ptr() as *mut u8, &mut size);
        LocalFree(g as _);
        if r != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.is_empty())
    }
}

/// PowerGetEffectiveOverlayScheme（旧系统没有，动态加载；失败再试 PowerGetActualOverlayScheme）
fn overlay_mode() -> Option<&'static str> {
    use windows_sys::core::GUID;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type F = unsafe extern "system" fn(*mut GUID) -> u32;
    unsafe {
        let lib = LoadLibraryW(wide("powrprof.dll").as_ptr());
        if lib.is_null() {
            return None;
        }
        let mut g: GUID = std::mem::zeroed();
        let ok = [c"PowerGetEffectiveOverlayScheme", c"PowerGetActualOverlayScheme"].iter().any(|name| match GetProcAddress(lib, name.as_ptr() as _) {
            Some(p) => {
                let f: F = std::mem::transmute(p);
                f(&mut g) == 0
            }
            None => false,
        });
        if !ok {
            return None;
        }
        let v = (g.data1 as u128) << 96 | (g.data2 as u128) << 80 | (g.data3 as u128) << 64 | u64::from_be_bytes(g.data4) as u128;
        if std::env::var_os("HUD_DEBUG").is_some() {
            eprintln!("overlay scheme = {v:032x}");
        }
        match v {
            0x961cc777_2547_4f9d_8174_7d86181b8a7a => Some("best-efficiency"),
            0x00000000_0000_0000_0000_000000000000 => Some("balanced"),
            0xded574b5_45a0_4f42_8737_46345c09c238 => Some("best-performance"),
            _ => None,
        }
    }
}
