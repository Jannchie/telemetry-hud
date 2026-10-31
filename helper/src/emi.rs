// Windows Energy Meter Interface（EMI，emi.h）：读能量计数器，用两次采样的差值算功率（无需驱动 / 管理员）
// 通道名含 PKG / PACKAGE → CPU 封装功率；含 PSYS / PLATFORM / SYSTEM → 整机功率
use crate::pdh::wide;
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::IO::DeviceIoControl;

const GUID_DEVICE_ENERGY_METER: GUID = GUID::from_u128(0x45bd8344_7ed6_49cf_a440_c276c933b053);
const IOCTL_EMI_GET_VERSION: u32 = 0x0022_4000;
const IOCTL_EMI_GET_METADATA_SIZE: u32 = 0x0022_4004;
const IOCTL_EMI_GET_METADATA: u32 = 0x0022_4008;
const IOCTL_EMI_GET_MEASUREMENT: u32 = 0x0022_400C;
const PWH_TO_J: f64 = 3.6e-9; // EmiMeasurementUnitPicowattHours

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Cpu,
    System,
    Other,
}

struct Meter {
    h: HANDLE,
    channels: Vec<Kind>,
    prev: Option<Vec<(u64, u64)>>, // (能量 pWh, 时间 100ns)
}

pub struct Emi {
    meters: Vec<Meter>,
}
unsafe impl Send for Emi {}

fn debug() -> bool {
    std::env::var_os("HUD_DEBUG").is_some()
}

unsafe fn io(h: HANDLE, code: u32, out: &mut [u8]) -> Option<usize> {
    let mut n = 0u32;
    let ok = DeviceIoControl(h, code, std::ptr::null(), 0, out.as_mut_ptr() as _, out.len() as u32, &mut n, std::ptr::null_mut());
    (ok != 0).then_some(n as usize)
}

fn classify(name: &str) -> Kind {
    let u = name.to_uppercase();
    if u.contains("PKG") || u.contains("PACKAGE") {
        Kind::Cpu
    } else if u.contains("PSYS") || u.contains("PLATFORM") || u.contains("SYSTEM") {
        Kind::System
    } else {
        Kind::Other
    }
}

fn utf16(b: &[u8]) -> String {
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&w)
}

fn interface_paths(guid: &GUID) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == -1isize as HDEVINFO {
            return out;
        }
        for i in 0.. {
            let mut ifd: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
            ifd.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, std::ptr::null(), guid, i, &mut ifd) == 0 {
                break;
            }
            let mut detail = [0u16; 1024];
            (detail.as_mut_ptr() as *mut u32).write_unaligned(8);
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, detail.as_mut_ptr() as _, (detail.len() * 2) as u32, std::ptr::null_mut(), std::ptr::null_mut()) != 0 {
                let p = &detail[2..];
                out.push(String::from_utf16_lossy(&p[..p.iter().position(|&c| c == 0).unwrap_or(p.len())]));
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

impl Emi {
    pub fn new() -> Emi {
        let mut meters = Vec::new();
        for path in interface_paths(&GUID_DEVICE_ENERGY_METER) {
            unsafe {
                let h = CreateFileW(wide(&path).as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
                if h == INVALID_HANDLE_VALUE {
                    if debug() {
                        eprintln!("emi open {path}: {}", std::io::Error::last_os_error());
                    }
                    continue;
                }
                let mut v = [0u8; 2];
                let mut sz = [0u8; 4];
                if io(h, IOCTL_EMI_GET_VERSION, &mut v).is_none() || io(h, IOCTL_EMI_GET_METADATA_SIZE, &mut sz).is_none() {
                    CloseHandle(h);
                    continue;
                }
                let version = u16::from_le_bytes(v);
                let mut md = vec![0u8; u32::from_le_bytes(sz) as usize];
                if io(h, IOCTL_EMI_GET_METADATA, &mut md).is_none() {
                    CloseHandle(h);
                    continue;
                }
                let mut names = Vec::new();
                let mut units = Vec::new();
                if version == 1 && md.len() >= 70 {
                    // V1：Unit u32, OEM[16], Model[16], Revision u16, MeteredHardwareName[]
                    units.push(u32::from_le_bytes(md[0..4].try_into().unwrap()));
                    names.push(utf16(&md[70..]));
                } else if version >= 2 && md.len() >= 68 {
                    // V2：OEM[16], Model[16], Revision u16, ChannelCount u16, 之后每个通道 { Unit u32, NameSize u16, Name[NameSize] }
                    let count = u16::from_le_bytes([md[66], md[67]]) as usize;
                    let mut off = 68;
                    for _ in 0..count {
                        if off + 6 > md.len() {
                            break;
                        }
                        let unit = u32::from_le_bytes(md[off..off + 4].try_into().unwrap());
                        let ns = u16::from_le_bytes([md[off + 4], md[off + 5]]) as usize;
                        let end = (off + 6 + ns).min(md.len());
                        units.push(unit);
                        names.push(utf16(&md[off + 6..end]));
                        off = end;
                    }
                }
                // 只认 picowatt-hours（unit 0），其他单位不猜
                let channels: Vec<Kind> = names.iter().zip(&units).map(|(n, &u)| if u == 0 { classify(n) } else { Kind::Other }).collect();
                if debug() {
                    eprintln!("emi v{version} {path}: {:?}", names.iter().zip(&units).collect::<Vec<_>>());
                }
                if channels.iter().any(|k| *k != Kind::Other) {
                    meters.push(Meter { h, channels, prev: None });
                } else {
                    CloseHandle(h);
                }
            }
        }
        Emi { meters }
    }

    /// (CPU 封装 W, 整机 W)；第一次调用只记录基准，返回 None
    pub fn sample(&mut self) -> (Option<f64>, Option<f64>) {
        let (mut cpu, mut sys) = (None, None);
        for m in &mut self.meters {
            let mut buf = vec![0u8; m.channels.len() * 16];
            if unsafe { io(m.h, IOCTL_EMI_GET_MEASUREMENT, &mut buf) }.is_none() {
                continue;
            }
            let now: Vec<(u64, u64)> = buf.chunks_exact(16).map(|c| (u64::from_le_bytes(c[0..8].try_into().unwrap()), u64::from_le_bytes(c[8..16].try_into().unwrap()))).collect();
            if let Some(prev) = &m.prev {
                for (i, k) in m.channels.iter().enumerate() {
                    let (Some(a), Some(b)) = (prev.get(i), now.get(i)) else { continue };
                    let dt = b.1.saturating_sub(a.1) as f64 * 1e-7;
                    if dt <= 0.0 || b.0 < a.0 {
                        continue;
                    }
                    let w = (b.0 - a.0) as f64 * PWH_TO_J / dt;
                    let w = (w * 10.0).round() / 10.0;
                    match k {
                        Kind::Cpu if cpu.is_none() => cpu = Some(w),
                        Kind::System if sys.is_none() => sys = Some(w),
                        _ => {}
                    }
                }
            }
            m.prev = Some(now);
        }
        (cpu, sys)
    }
}

/// 放电功率（W）：仅在用电池供电时有意义；IOCTL_BATTERY_QUERY_STATUS 的 Rate（mW，放电为负）
pub fn battery_discharge_w() -> Option<f64> {
    const GUID_DEVCLASS_BATTERY: GUID = GUID::from_u128(0x72631e54_78a4_11d0_bcf7_00aa00b7b32a);
    const IOCTL_BATTERY_QUERY_TAG: u32 = 0x0029_4040;
    const IOCTL_BATTERY_QUERY_STATUS: u32 = 0x0029_404C;
    let mut total = 0f64;
    let mut any = false;
    for path in interface_paths(&GUID_DEVCLASS_BATTERY) {
        unsafe {
            let h = CreateFileW(wide(&path).as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
            if h == INVALID_HANDLE_VALUE {
                continue;
            }
            let wait = 0u32.to_le_bytes();
            let mut tag = [0u8; 4];
            let mut n = 0u32;
            let ok = DeviceIoControl(h, IOCTL_BATTERY_QUERY_TAG, wait.as_ptr() as _, 4, tag.as_mut_ptr() as _, 4, &mut n, std::ptr::null_mut());
            if ok != 0 && u32::from_le_bytes(tag) != 0 {
                // BATTERY_WAIT_STATUS { BatteryTag, Timeout, PowerState, LowCapacity, HighCapacity }
                let mut ws = [0u8; 20];
                ws[0..4].copy_from_slice(&tag);
                // BATTERY_STATUS { PowerState, Capacity, Voltage, Rate(i32) }
                let mut st = [0u8; 16];
                if DeviceIoControl(h, IOCTL_BATTERY_QUERY_STATUS, ws.as_ptr() as _, 20, st.as_mut_ptr() as _, 16, &mut n, std::ptr::null_mut()) != 0 {
                    let rate = i32::from_le_bytes(st[12..16].try_into().unwrap());
                    // 0x80000000 = BATTERY_UNKNOWN_RATE
                    if rate != i32::MIN && rate < 0 {
                        total += -rate as f64 / 1000.0;
                        any = true;
                    }
                }
            }
            CloseHandle(h);
        }
    }
    any.then(|| (total * 10.0).round() / 10.0)
}
