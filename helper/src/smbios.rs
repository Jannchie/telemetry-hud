// SMBIOS 原始表解析（GetSystemFirmwareTable('RSMB')，无需管理员）
use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;

pub struct Structure {
    pub ty: u8,
    pub f: Vec<u8>,
    strs: Vec<String>,
}

impl Structure {
    pub fn b(&self, i: usize) -> Option<u8> {
        self.f.get(i).copied()
    }
    pub fn w(&self, i: usize) -> Option<u16> {
        (i + 2 <= self.f.len()).then(|| u16::from_le_bytes([self.f[i], self.f[i + 1]]))
    }
    pub fn d(&self, i: usize) -> Option<u32> {
        (i + 4 <= self.f.len()).then(|| u32::from_le_bytes(self.f[i..i + 4].try_into().unwrap()))
    }
    /// 字符串字段（偏移处是 1 起的字符串编号），空串视为缺失
    pub fn s(&self, i: usize) -> Option<String> {
        self.b(i).filter(|&k| k > 0).and_then(|k| self.strs.get(k as usize - 1)).filter(|v| !v.is_empty()).cloned()
    }
}

pub fn structures() -> Vec<Structure> {
    let mut out = Vec::new();
    let raw = unsafe {
        let sig = u32::from_be_bytes(*b"RSMB");
        let n = GetSystemFirmwareTable(sig, 0, std::ptr::null_mut(), 0);
        if n == 0 {
            return out;
        }
        let mut b = vec![0u8; n as usize];
        if GetSystemFirmwareTable(sig, 0, b.as_mut_ptr(), n) != n {
            return out;
        }
        b
    };
    if raw.len() < 8 {
        return out;
    }
    let len = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    let t = &raw[8..(8 + len).min(raw.len())];
    let mut off = 0usize;
    while off + 4 <= t.len() {
        let (ty, flen) = (t[off], t[off + 1] as usize);
        if flen < 4 || off + flen > t.len() {
            break;
        }
        // 字符串区：formatted 部分之后，以双 0 结束
        let mut strs = Vec::new();
        let mut p = off + flen;
        let mut cur = Vec::new();
        while p < t.len() {
            if t[p] == 0 {
                if cur.is_empty() {
                    break;
                }
                strs.push(String::from_utf8_lossy(&cur).trim().to_string());
                cur.clear();
            } else {
                cur.push(t[p]);
            }
            p += 1;
        }
        out.push(Structure { ty, f: t[off..off + flen].to_vec(), strs: strs.clone() });
        if ty == 127 {
            break;
        }
        off = if strs.is_empty() { off + flen + 2 } else { p + 1 };
    }
    out
}
