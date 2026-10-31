// 磁盘寿命：IOCTL_STORAGE_QUERY_PROPERTY + StorageDeviceProtocolSpecificProperty（以 0 访问权限打开，无需管理员）
//   NVMe：SMART / Health Information 日志（Log 0x02）
//   SATA：ATA Device Statistics 日志（Log 0x04，第 1 页通电时间 / 写入扇区，第 7 页耐久度百分比）
// 读不到就返回 None（页面显示 NO SIGNAL），不做任何推测
use crate::sysinfo::{ioctl, IOCTL_STORAGE_QUERY_PROPERTY};
use serde_json::{json, Map, Value};

const PROTOCOL_ATA: u32 = 2;
const PROTOCOL_NVME: u32 = 3;
const DATA_TYPE_LOG_PAGE: u32 = 2; // NVMeDataTypeLogPage / AtaDataTypeLogPage

/// 读一个协议日志页，返回 512 字节数据
fn log_page(disk: u32, protocol: u32, log: u32, page: u32) -> Option<Vec<u8>> {
    const HDR: usize = 8; // STORAGE_PROPERTY_QUERY.PropertyId + QueryType
    const SPEC: usize = 40; // STORAGE_PROTOCOL_SPECIFIC_DATA
    const LEN: usize = 512;
    let mut q = vec![0u8; HDR + SPEC + LEN];
    let put = |b: &mut [u8], off: usize, v: u32| b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut q, 0, 50); // StorageDeviceProtocolSpecificProperty
    put(&mut q, 4, 0); // PropertyStandardQuery
    put(&mut q, HDR, protocol);
    put(&mut q, HDR + 4, DATA_TYPE_LOG_PAGE);
    put(&mut q, HDR + 8, log);
    put(&mut q, HDR + 12, page);
    put(&mut q, HDR + 16, SPEC as u32); // ProtocolDataOffset（相对 ProtocolSpecificData 起点）
    put(&mut q, HDR + 20, LEN as u32);
    let mut out = vec![0u8; q.len()];
    unsafe { ioctl(&format!("\\\\.\\PhysicalDrive{disk}"), IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut out)? };
    // STORAGE_PROTOCOL_DATA_DESCRIPTOR { Version, Size, ProtocolSpecificData }
    let off = u32::from_le_bytes(out[HDR + 16..HDR + 20].try_into().unwrap()) as usize;
    let len = u32::from_le_bytes(out[HDR + 20..HDR + 24].try_into().unwrap()) as usize;
    let start = HDR + off;
    if len < LEN || start + LEN > out.len() {
        return None;
    }
    Some(out[start..start + LEN].to_vec())
}

fn u128_le(b: &[u8]) -> u128 {
    u128::from_le_bytes(b[..16].try_into().unwrap())
}

fn nvme(disk: u32) -> Option<Map<String, Value>> {
    let d = log_page(disk, PROTOCOL_NVME, 0x02, 0)?;
    let mut m = Map::new();
    m.insert("lifeUsed".into(), json!(d[5]));
    m.insert("spare".into(), json!(d[3]));
    m.insert("powerOnHours".into(), json!(u128_le(&d[128..144]) as u64));
    // Data Units Written：单位 1000 × 512 B
    let tb = u128_le(&d[48..64]) as f64 * 512_000.0 / 1e12;
    m.insert("tbw".into(), json!((tb * 10.0).round() / 10.0));
    let temp_k = u16::from_le_bytes([d[1], d[2]]);
    if temp_k > 0 {
        m.insert("temp".into(), json!(temp_k as i32 - 273));
    }
    Some(m)
}

/// ATA Device Statistics 的一个条目：bit63 supported，bit62 valid
fn ata_stat(page: &[u8], off: usize) -> Option<u64> {
    let q = u64::from_le_bytes(page.get(off..off + 8)?.try_into().ok()?);
    (q >> 63 & 1 == 1 && q >> 62 & 1 == 1).then_some(q & 0x0000_FFFF_FFFF_FFFF)
}

fn ata(disk: u32, ssd: bool) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    // 页头 bits 16..23 是页号，用来确认数据确实是那一页
    let page_ok = |p: &[u8], n: u8| p.len() >= 8 && p[2] == n;
    if let Some(p) = log_page(disk, PROTOCOL_ATA, 0x04, 1).filter(|p| page_ok(p, 1)) {
        if let Some(h) = ata_stat(&p, 16) {
            m.insert("powerOnHours".into(), json!(h & 0xFFFF_FFFF));
        }
        if ssd {
            if let Some(sectors) = ata_stat(&p, 24) {
                m.insert("tbw".into(), json!(((sectors as f64 * 512.0 / 1e12) * 10.0).round() / 10.0));
            }
        }
    }
    if ssd {
        if let Some(p) = log_page(disk, PROTOCOL_ATA, 0x04, 7).filter(|p| page_ok(p, 7)) {
            if let Some(v) = ata_stat(&p, 8) {
                m.insert("lifeUsed".into(), json!(v & 0xFF));
            }
        }
    }
    (!m.is_empty()).then_some(m)
}

/// kind 为 disk_type 的结果（"NVMe" / "SSD" / "HDD"）
pub fn read(volume: &str, kind: Option<&str>) -> Option<Value> {
    let disk = crate::sysinfo::disk_number(volume)?;
    let m = match kind {
        Some("NVMe") => nvme(disk),
        Some("SSD") => ata(disk, true),
        Some("HDD") => ata(disk, false),
        _ => None,
    }?;
    Some(Value::Object(m))
}
