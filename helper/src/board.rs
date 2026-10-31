// 主板 / BIOS / 扩展槽（SMBIOS Type 0/2/9）+ 设备树（SetupAPI 枚举 PCI / SATA / USB 设备，匹配插槽与盘符）
use crate::smbios;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::Properties::*;
use windows_sys::Win32::Foundation::DEVPROPKEY;

const PCI_FMTID: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);
const KEY_BASE_CLASS: DEVPROPKEY = DEVPROPKEY { fmtid: PCI_FMTID, pid: 3 };
const KEY_SUB_CLASS: DEVPROPKEY = DEVPROPKEY { fmtid: PCI_FMTID, pid: 4 };
const KEY_LINK_SPEED: DEVPROPKEY = DEVPROPKEY { fmtid: PCI_FMTID, pid: 9 };
const KEY_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY { fmtid: PCI_FMTID, pid: 10 };

struct Slot {
    name: String,
    kind: &'static str,
    width: Option<u32>,
    gen: Option<u32>,
    in_use: Option<bool>,
    /// (bus, device<<16 | function)，0xFF 表示 BIOS 没给
    bdf: Option<(u32, u32)>,
}

pub struct Board {
    base: Map<String, Value>,
    slots: Vec<Slot>,
}

#[derive(Default)]
struct Dev {
    id: String,
    parent: Option<String>,
    name: Option<String>,
    bus: Option<u32>,
    addr: Option<u32>,
    class: Option<u8>,
    sub: Option<u8>,
    /// 设备安装类名（MEDIA / Net ...），只对 USB 设备读取
    class_name: Option<String>,
    width: Option<u32>,
    speed: Option<u32>,
}

impl Board {
    /// SMBIOS 部分只在启动时读一次
    pub fn new() -> Board {
        let mut base = Map::new();
        let mut slots = Vec::new();
        for st in smbios::structures() {
            match st.ty {
                0 => {
                    if let Some(v) = st.s(0x05) {
                        let bios = match st.s(0x08) {
                            Some(d) => format!("{v} ({d})"),
                            None => v,
                        };
                        base.insert("bios".into(), json!(bios));
                    }
                }
                2 => {
                    if let Some(v) = st.s(0x04) {
                        base.insert("vendor".into(), json!(v));
                    }
                    if let Some(v) = st.s(0x05) {
                        base.insert("product".into(), json!(v));
                    }
                }
                9 => {
                    let name = st.s(0x04).unwrap_or_default();
                    let ty = st.b(0x05).unwrap_or(0);
                    let upper = name.to_uppercase();
                    let kind = if (0x14..=0x17).contains(&ty) || upper.contains("M.2") || upper.starts_with("M2") {
                        "M.2"
                    } else if (0xA5..=0xC8).contains(&ty) || matches!(ty, 0x1F | 0x20 | 0x24 | 0x25) || upper.contains("PCIE") || upper.contains("PCI-E") {
                        "PCIe"
                    } else {
                        "other"
                    };
                    let gen = match ty {
                        // 0xA5..=0xAA 是不带代数的通用 "PCI Express" 类型 → 未知
                        0xAB..=0xB0 | 0x1F => Some(2),
                        0xB1..=0xB6 | 0x20 => Some(3),
                        0xB7..=0xBC | 0x24 => Some(4),
                        0xBD..=0xC2 | 0x25 => Some(5),
                        0xC3..=0xC8 => Some(6),
                        _ => None,
                    };
                    let width = match st.b(0x06) {
                        Some(0x08) => Some(1),
                        Some(0x09) => Some(2),
                        Some(0x0A) => Some(4),
                        Some(0x0B) => Some(8),
                        Some(0x0C) => Some(12),
                        Some(0x0D) => Some(16),
                        Some(0x0E) => Some(32),
                        _ => None,
                    };
                    let in_use = match st.b(0x07) {
                        Some(3) => Some(true),
                        Some(4) => Some(false),
                        _ => None,
                    };
                    // 2.6+：Segment @0x0D, Bus @0x0F, Dev/Fn @0x10
                    let bdf = match (st.b(0x0F), st.b(0x10)) {
                        (Some(bus), Some(df)) if bus != 0xFF && df != 0xFF => Some((bus as u32, ((df >> 3) as u32) << 16 | (df & 7) as u32)),
                        _ => None,
                    };
                    slots.push(Slot { name, kind, width, gen, in_use, bdf });
                }
                _ => {}
            }
        }
        Board { base, slots }
    }

    /// 枚举设备、匹配插槽并生成主板设备树（每 30 s 调一次）
    pub fn scan(&self) -> Value {
        let devs = enumerate();
        let idx: HashMap<&str, &Dev> = devs.iter().map(|d| (d.id.as_str(), d)).collect();
        let mut children: HashMap<&str, Vec<&Dev>> = HashMap::new();
        for d in &devs {
            if let Some(p) = &d.parent {
                children.entry(p.as_str()).or_default().push(d);
            }
        }
        let pci: Vec<&Dev> = devs.iter().filter(|d| d.id.starts_with("PCI\\")).collect();
        let disks = disk_interfaces();
        let letters = volume_letters();
        let debug = std::env::var_os("HUD_DEBUG").is_some();

        // 设备的根端口：沿父链上溯，直接挂在根复合体（ACPI PNP0A08/0A03）下的那一级；集成设备返回 None
        fn root_port<'a>(d: &'a Dev, idx: &HashMap<&str, &'a Dev>) -> Option<&'a Dev> {
            let mut cur: &Dev = d;
            for _ in 0..16 {
                let p = idx.get(cur.parent.as_deref()?)?;
                if is_root_complex(&p.id) {
                    return (cur.id != d.id).then_some(cur);
                }
                cur = p;
            }
            None
        }
        // NVMe 控制器 / SATA 盘对应的磁盘设备
        let child_disk = |d: &Dev| children.get(d.id.as_str()).and_then(|c| c.iter().find(|c| c.id.starts_with("SCSI\\DISK")).copied());
        let disk_letters = |disk: &Dev| disks.get(&disk.id).and_then(|(n, _)| letters.get(n)).map(|v| v.join(", "));
        let display_name = |d: &Dev| -> Option<String> {
            if d.class == Some(0x01) {
                if let Some(n) = child_disk(d).and_then(|c| c.name.clone()) {
                    return Some(n);
                }
            }
            d.name.clone()
        };
        let link = |d: &Dev| match (d.width, d.speed) {
            (Some(w), Some(s)) if w > 0 && s > 0 => Some(format!("x{w} Gen{s}")),
            (Some(w), _) if w > 0 => Some(format!("x{w}")),
            _ => None,
        };

        // 插槽：Type 9 的 BDF 通常是根端口
        let mut slot_of_port: HashMap<&str, &str> = HashMap::new();
        let slots: Vec<Value> = self
            .slots
            .iter()
            .map(|s| {
                let mut dev: Option<&Dev> = None;
                if let Some((bus, addr)) = s.bdf {
                    if let Some(port) = pci.iter().find(|d| d.bus == Some(bus) && d.addr == Some(addr)) {
                        if port.class == Some(0x06) {
                            slot_of_port.insert(port.id.as_str(), s.name.as_str());
                            dev = children
                                .get(port.id.as_str())
                                .and_then(|c| c.iter().filter(|c| c.id.starts_with("PCI\\")).min_by_key(|c| c.addr.unwrap_or(u32::MAX)).copied());
                        } else {
                            dev = Some(port);
                        }
                    }
                }
                json!({
                    "name": s.name,
                    "kind": s.kind,
                    "width": s.width,
                    "gen": s.gen,
                    // BIOS 的 Current Usage 常常不准：匹配到设备就以实际为准
                    "inUse": if dev.is_some() { Some(true) } else { s.in_use },
                    "device": dev.and_then(display_name),
                    "link": dev.and_then(link),
                })
            })
            .collect();

        // 设备树
        let mut devices: Vec<(u8, Value)> = Vec::new();
        for d in &pci {
            if d.class == Some(0x06) || d.class.is_none() {
                continue; // 桥 / 主桥
            }
            let class = pci_class(d);
            let port = root_port(d, &idx);
            let (where_, bus) = match port {
                None => {
                    // 芯片组集成设备：只保留网卡 / 无线网卡 / 声卡 / 显卡
                    if !matches!(class, "network" | "wireless" | "audio" | "display") {
                        continue;
                    }
                    ("Onboard".to_string(), if class == "audio" && d.sub == Some(0x03) { "HDA" } else { "PCIe" })
                }
                Some(port) => {
                    // 多功能设备的附属 function（如显卡的音频）不单列
                    let a = d.addr.unwrap_or(0);
                    if a & 0xFFFF != 0 && pci.iter().any(|o| o.bus == d.bus && o.addr == Some(a & 0xFFFF_0000)) {
                        continue;
                    }
                    let slot = slot_of_port.get(port.id.as_str()).copied();
                    let nvme = d.class == Some(0x01) && d.sub == Some(0x08);
                    // USB / 雷电控制器只在插在扩展槽上（扩展卡）时列出；非 NVMe 存储控制器的盘会作为 SATA 盘单独列
                    if slot.is_none() && (class == "usb" || (d.class == Some(0x01) && !nvme)) {
                        continue;
                    }
                    let short = short_name(port.name.as_deref().unwrap_or(""));
                    let w = match slot {
                        Some(s) => s.to_string(),
                        None if nvme => format!("M.2 · {short}"),
                        None => format!("Onboard · {short}"),
                    };
                    (w, "PCIe")
                }
            };
            let disk = if d.class == Some(0x01) { child_disk(d).and_then(disk_letters) } else { None };
            devices.push((
                rank(class),
                json!({ "name": display_name(d), "class": class, "bus": bus, "where": where_, "link": if port.is_some() { link(d) } else { None }, "disk": disk }),
            ));
        }
        // SATA 盘：父设备是 AHCI / RAID 控制器
        for d in devs.iter().filter(|d| d.id.starts_with("SCSI\\DISK")) {
            let Some(ctrl) = d.parent.as_deref().and_then(|p| idx.get(p)) else { continue };
            if !(ctrl.id.starts_with("PCI\\") && ctrl.class == Some(0x01) && matches!(ctrl.sub, Some(0x04) | Some(0x06))) {
                continue;
            }
            let port = disks.get(&d.id).and_then(|(_, p)| *p);
            let where_ = port.map_or("SATA".to_string(), |p| format!("SATA {p}"));
            devices.push((rank("storage") + 1, json!({ "name": d.name, "class": "storage", "bus": "SATA", "where": where_, "link": null, "disk": disk_letters(d) })));
        }
        // USB 声卡 / 网卡（按名称去重）
        let mut seen = std::collections::HashSet::new();
        for d in devs.iter().filter(|d| d.id.starts_with("USB\\")) {
            let class = match d.class_name.as_deref() {
                Some("MEDIA") => "audio",
                Some("Net") => "network",
                _ => continue,
            };
            let Some(name) = d.name.clone() else { continue };
            if seen.insert(name.clone()) {
                devices.push((rank(class), json!({ "name": name, "class": class, "bus": "USB", "where": "USB", "link": null, "disk": null })));
            }
        }
        devices.sort_by_key(|d| d.0);

        if debug {
            for (id, (n, p)) in &disks {
                eprintln!("disk #{n} pathId={p:?} {id}");
            }
            for d in &pci {
                let a = d.addr.unwrap_or(0);
                eprintln!("pci {:02x}:{:02x}.{} class={:?}/{:?} {:?} parent={:?}", d.bus.unwrap_or(0), a >> 16, a & 0xffff, d.class, d.sub, d.name, d.parent);
            }
        }

        let mut b = self.base.clone();
        b.insert("slots".into(), Value::Array(slots));
        b.insert("devices".into(), Value::Array(devices.into_iter().map(|d| d.1).collect()));
        Value::Object(b)
    }
}

fn is_root_complex(id: &str) -> bool {
    id.starts_with("ACPI\\PNP0A08") || id.starts_with("ACPI\\PNP0A03")
}

fn pci_class(d: &Dev) -> &'static str {
    let name = d.name.as_deref().unwrap_or("").to_lowercase();
    match (d.class, d.sub) {
        (Some(0x01), _) => "storage",
        (Some(0x02), sub) if sub == Some(0x80) || ["wi-fi", "wireless", "wlan", "802.11"].iter().any(|k| name.contains(k)) => "wireless",
        (Some(0x02), _) => "network",
        (Some(0x03), _) => "display",
        (Some(0x04), _) => "audio",
        (Some(0x0C), Some(0x03)) | (Some(0x0C), Some(0x0A)) => "usb",
        _ => "other",
    }
}

fn rank(class: &str) -> u8 {
    match class {
        "display" => 0,
        "storage" => 10,
        "network" => 20,
        "wireless" => 30,
        "audio" => 40,
        "usb" => 50,
        _ => 60,
    }
}

/// "Intel(R) PCI Express Root Port #3 - 7ABA" → "Root Port #3"，"Intel(R) PEG60 - 464D" → "PEG60"
fn short_name(s: &str) -> String {
    let mut s = s.replace("Intel(R) ", "").replace("AMD ", "").replace("PCI Express ", "");
    if let Some(i) = s.rfind(" - ") {
        if s[i + 3..].chars().all(|c| c.is_ascii_hexdigit()) {
            s.truncate(i);
        }
    }
    s.trim().to_string()
}

/// 物理盘号 → 盘符列表
fn volume_letters() -> HashMap<u32, Vec<String>> {
    let mut m: HashMap<u32, Vec<String>> = HashMap::new();
    for (v, _, _) in crate::sysinfo::fixed_volumes() {
        if let Some(n) = crate::sysinfo::disk_number(&v) {
            m.entry(n).or_default().push(v);
        }
    }
    m
}

/// 磁盘设备实例 ID → (物理盘号, SCSI PathId = SATA 端口号)
fn disk_interfaces() -> HashMap<String, (u32, Option<u8>)> {
    const GUID_DEVINTERFACE_DISK: GUID = GUID::from_u128(0x53f56307_b6bf_11d0_94f2_00a0c91efb8b);
    const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;
    const IOCTL_SCSI_GET_ADDRESS: u32 = 0x0004_1018;
    let mut out = HashMap::new();
    unsafe {
        let set = SetupDiGetClassDevsW(&GUID_DEVINTERFACE_DISK, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == -1isize as HDEVINFO {
            return out;
        }
        let mut buf = [0u8; 1024];
        for i in 0.. {
            let mut ifd: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
            ifd.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &GUID_DEVINTERFACE_DISK, i, &mut ifd) == 0 {
                break;
            }
            // SP_DEVICE_INTERFACE_DETAIL_DATA_W：cbSize 固定为 8（x64），路径从偏移 4 开始
            let mut detail = [0u16; 1024];
            (detail.as_mut_ptr() as *mut u32).write_unaligned(8);
            let mut info: SP_DEVINFO_DATA = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, detail.as_mut_ptr() as _, (detail.len() * 2) as u32, std::ptr::null_mut(), &mut info) == 0 {
                continue;
            }
            let p = &detail[2..];
            let path = String::from_utf16_lossy(&p[..p.iter().position(|&c| c == 0).unwrap_or(p.len())]);
            let Some(id) = prop_str(set, &info, &DEVPKEY_Device_InstanceId, &mut buf).map(|s| s.to_uppercase()) else { continue };
            let mut num = [0u8; 12];
            if crate::sysinfo::ioctl(&path, IOCTL_STORAGE_GET_DEVICE_NUMBER, &[], &mut num).is_none() {
                continue;
            }
            let n = u32::from_le_bytes(num[4..8].try_into().unwrap());
            let mut addr = [0u8; 8];
            let path_id = crate::sysinfo::ioctl(&path, IOCTL_SCSI_GET_ADDRESS, &[], &mut addr).map(|_| addr[5]);
            out.insert(id, (n, path_id));
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

unsafe fn prop(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<(u32, usize)> {
    let (mut ty, mut need) = (0u32, 0u32);
    let ok = SetupDiGetDevicePropertyW(set, info, key, &mut ty, buf.as_mut_ptr(), buf.len() as u32, &mut need, 0);
    (ok != 0).then_some((ty, need as usize))
}

unsafe fn prop_str(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<String> {
    let (ty, n) = prop(set, info, key, buf)?;
    if ty != DEVPROP_TYPE_STRING {
        return None;
    }
    let w: Vec<u16> = buf[..n.min(buf.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&w).trim_end_matches(char::from(0)).trim().to_string()).filter(|s| !s.is_empty())
}

unsafe fn prop_u32(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<u32> {
    match prop(set, info, key, buf)? {
        (DEVPROP_TYPE_UINT32, 4) => Some(u32::from_le_bytes(buf[..4].try_into().unwrap())),
        (DEVPROP_TYPE_BYTE, 1) => Some(buf[0] as u32),
        _ => None,
    }
}

fn enumerate() -> Vec<Dev> {
    let mut out = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_ALLCLASSES);
        if set == -1isize as HDEVINFO {
            return out;
        }
        let mut buf = [0u8; 1024];
        let mut i = 0;
        loop {
            let mut info: SP_DEVINFO_DATA = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiEnumDeviceInfo(set, i, &mut info) == 0 {
                break;
            }
            i += 1;
            // 实例 ID 大小写不统一（Parent 属性里常见小写），统一转大写再比较
            let Some(id) = prop_str(set, &info, &DEVPKEY_Device_InstanceId, &mut buf).map(|s| s.to_uppercase()) else { continue };
            let mut d = Dev { parent: prop_str(set, &info, &DEVPKEY_Device_Parent, &mut buf).map(|s| s.to_uppercase()), ..Default::default() };
            d.name = prop_str(set, &info, &DEVPKEY_Device_FriendlyName, &mut buf).or_else(|| prop_str(set, &info, &DEVPKEY_Device_DeviceDesc, &mut buf));
            if id.starts_with("USB\\") {
                d.class_name = prop_str(set, &info, &DEVPKEY_Device_Class, &mut buf);
            }
            if id.starts_with("PCI\\") {
                d.bus = prop_u32(set, &info, &DEVPKEY_Device_BusNumber, &mut buf);
                d.addr = prop_u32(set, &info, &DEVPKEY_Device_Address, &mut buf);
                d.class = prop_u32(set, &info, &KEY_BASE_CLASS, &mut buf).map(|v| v as u8);
                d.sub = prop_u32(set, &info, &KEY_SUB_CLASS, &mut buf).map(|v| v as u8);
                d.width = prop_u32(set, &info, &KEY_LINK_WIDTH, &mut buf);
                d.speed = prop_u32(set, &info, &KEY_LINK_SPEED, &mut buf);
            }
            d.id = id;
            out.push(d);
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}
