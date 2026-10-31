// NVML 动态加载（无 NVIDIA 驱动时自动跳过），以及注册表里的显卡静态信息兜底
use crate::pdh::wide;
use serde_json::{json, Map, Value};
use std::ffi::c_void;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;

type Dev = *mut c_void;
type FnDevU32 = unsafe extern "C" fn(Dev, *mut u32) -> i32;
type FnDevArgU32 = unsafe extern "C" fn(Dev, u32, *mut u32) -> i32;

#[repr(C)]
#[derive(Default)]
struct Util {
    gpu: u32,
    mem: u32,
}
#[repr(C)]
#[derive(Default)]
struct MemInfo {
    total: u64,
    free: u64,
    used: u64,
}

pub struct Nvml {
    dev: Dev,
    util: unsafe extern "C" fn(Dev, *mut Util) -> i32,
    mem: unsafe extern "C" fn(Dev, *mut MemInfo) -> i32,
    temp: FnDevArgU32,
    power: FnDevU32,
    clock: FnDevArgU32,
    pub static_info: Map<String, Value>,
    pub spec: Map<String, Value>,
}
unsafe impl Send for Nvml {}

unsafe fn sym<T>(lib: *mut c_void, name: &str) -> Option<T> {
    let c = std::ffi::CString::new(name).ok()?;
    let p = GetProcAddress(lib, c.as_ptr() as _)?;
    Some(std::mem::transmute_copy(&p))
}

fn cstr(buf: &[u8]) -> String {
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).trim().to_string()
}

impl Nvml {
    pub fn load() -> Option<Nvml> {
        unsafe {
            let mut lib = std::ptr::null_mut();
            for p in ["nvml.dll", r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"] {
                lib = LoadLibraryW(wide(p).as_ptr());
                if !lib.is_null() {
                    break;
                }
            }
            if lib.is_null() {
                return None;
            }
            let init: unsafe extern "C" fn() -> i32 = sym(lib, "nvmlInit_v2")?;
            if init() != 0 {
                return None;
            }
            let get_handle: unsafe extern "C" fn(u32, *mut Dev) -> i32 = sym(lib, "nvmlDeviceGetHandleByIndex_v2")?;
            let mut dev: Dev = std::ptr::null_mut();
            if get_handle(0, &mut dev) != 0 {
                return None;
            }
            let mut info = Map::new();
            let mut spec = Map::new();
            let mut buf = [0u8; 96];
            if let Some(f) = sym::<unsafe extern "C" fn(Dev, *mut u8, u32) -> i32>(lib, "nvmlDeviceGetName") {
                if f(dev, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                    info.insert("name".into(), json!(cstr(&buf)));
                }
            }
            if let Some(f) = sym::<unsafe extern "C" fn(*mut u8, u32) -> i32>(lib, "nvmlSystemGetDriverVersion") {
                buf.fill(0);
                if f(buf.as_mut_ptr(), buf.len() as u32) == 0 {
                    spec.insert("driver".into(), json!(cstr(&buf)));
                }
            }
            let mut v = 0u32;
            if let Some(f) = sym::<FnDevU32>(lib, "nvmlDeviceGetMemoryBusWidth") {
                if f(dev, &mut v) == 0 && v > 0 {
                    spec.insert("busWidth".into(), json!(v));
                }
            }
            if let Some(f) = sym::<FnDevArgU32>(lib, "nvmlDeviceGetMaxClockInfo") {
                if f(dev, 0, &mut v) == 0 && v > 0 {
                    spec.insert("boostClock".into(), json!(r2(v as f64 / 1000.0)));
                }
            }
            if let Some(f) = sym::<FnDevU32>(lib, "nvmlDeviceGetPowerManagementDefaultLimit") {
                if f(dev, &mut v) == 0 && v > 0 {
                    spec.insert("tdp".into(), json!((v as f64 / 1000.0).round() as u64));
                }
            }
            Some(Nvml {
                dev,
                util: sym(lib, "nvmlDeviceGetUtilizationRates")?,
                mem: sym(lib, "nvmlDeviceGetMemoryInfo")?,
                temp: sym(lib, "nvmlDeviceGetTemperature")?,
                power: sym(lib, "nvmlDeviceGetPowerUsage")?,
                clock: sym(lib, "nvmlDeviceGetClockInfo")?,
                static_info: info,
                spec,
            })
        }
    }

    pub fn sample(&self, out: &mut Map<String, Value>) {
        unsafe {
            let mut u = Util::default();
            if (self.util)(self.dev, &mut u) == 0 {
                out.insert("usage".into(), json!(u.gpu));
            }
            let mut m = MemInfo::default();
            if (self.mem)(self.dev, &mut m) == 0 && m.total > 0 {
                out.insert("vramUsed".into(), json!(r2(m.used as f64 / 1073741824.0)));
                out.insert("vramTotal".into(), json!(r2(m.total as f64 / 1073741824.0)));
            }
            let mut v = 0u32;
            if (self.temp)(self.dev, 0, &mut v) == 0 {
                out.insert("temp".into(), json!(v));
            }
            if (self.power)(self.dev, &mut v) == 0 {
                out.insert("power".into(), json!(r1(v as f64 / 1000.0)));
            }
            if (self.clock)(self.dev, 0, &mut v) == 0 {
                out.insert("clock".into(), json!(r2(v as f64 / 1000.0)));
            }
        }
    }
}

/// 非 NVIDIA 兜底：从显示适配器类注册表取名称 / 显存总量 / 驱动版本（取显存最大的那块）
pub fn registry_gpu() -> (Map<String, Value>, Map<String, Value>) {
    let mut best: Option<(u64, String, Option<String>)> = None;
    for i in 0..10 {
        let key = format!(r"SYSTEM\CurrentControlSet\Control\Class\{{4d36e968-e325-11ce-bfc1-08002be10318}}\{i:04}");
        let Some(desc) = crate::sysinfo::reg_string(HKEY_LOCAL_MACHINE, &key, "DriverDesc") else { continue };
        let mem = crate::sysinfo::reg_u64(HKEY_LOCAL_MACHINE, &key, "HardwareInformation.qwMemorySize").unwrap_or(0);
        let drv = crate::sysinfo::reg_string(HKEY_LOCAL_MACHINE, &key, "DriverVersion");
        if best.as_ref().map_or(true, |b| mem > b.0) {
            best = Some((mem, desc, drv));
        }
    }
    let (mut info, mut spec) = (Map::new(), Map::new());
    if let Some((mem, desc, drv)) = best {
        info.insert("name".into(), json!(desc));
        if mem > 0 {
            info.insert("vramTotal".into(), json!(r2(mem as f64 / 1073741824.0)));
        }
        if let Some(d) = drv {
            spec.insert("driver".into(), json!(d));
        }
    }
    (info, spec)
}

pub fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
pub fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
