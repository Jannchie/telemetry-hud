// PDH 的极简封装：一个 query 里挂多个计数器，每秒 collect 一次
use windows_sys::Win32::System::Performance::*;

pub const FMT_DOUBLE: u32 = PDH_FMT_DOUBLE;
pub const FMT_NOCAP: u32 = PDH_FMT_DOUBLE | 0x8000; // PDH_FMT_NOCAP100

pub struct Query {
    h: PDH_HQUERY,
    buf: std::cell::RefCell<Vec<u64>>,
}

#[derive(Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Query {
    pub fn new() -> Option<Self> {
        let mut h: PDH_HQUERY = std::ptr::null_mut();
        let r = unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut h) };
        (r == 0).then(|| Query { h, buf: Default::default() })
    }

    pub fn add(&self, path: &str) -> Option<Counter> {
        let mut c: PDH_HCOUNTER = std::ptr::null_mut();
        let w = wide(path);
        let r = unsafe { PdhAddEnglishCounterW(self.h, w.as_ptr(), 0, &mut c) };
        (r == 0).then_some(Counter(c))
    }

    pub fn collect(&self) -> bool {
        unsafe { PdhCollectQueryData(self.h) == 0 }
    }

    pub fn value(&self, c: Counter, fmt: u32) -> Option<f64> {
        unsafe {
            let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
            let r = PdhGetFormattedCounterValue(c.0, fmt, std::ptr::null_mut(), &mut v);
            (r == 0 && (v.CStatus == 0 || v.CStatus == 1)).then(|| v.Anonymous.doubleValue)
        }
    }

    /// 通配实例计数器 → [(实例名, 值)]
    pub fn array(&self, c: Counter, fmt: u32) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        let mut buf = self.buf.borrow_mut();
        unsafe {
            for _ in 0..3 {
                let mut size = (buf.len() * 8) as u32;
                let mut count = 0u32;
                let p = if buf.is_empty() { std::ptr::null_mut() } else { buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W };
                let r = PdhGetFormattedCounterArrayW(c.0, fmt, &mut size, &mut count, p);
                if r == PDH_MORE_DATA {
                    buf.resize((size as usize + 7) / 8 + 64, 0);
                    continue;
                }
                if r != 0 {
                    return out;
                }
                let items = std::slice::from_raw_parts(p, count as usize);
                for it in items {
                    if it.FmtValue.CStatus != 0 && it.FmtValue.CStatus != 1 {
                        continue;
                    }
                    let mut n = 0;
                    while *it.szName.add(n) != 0 {
                        n += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, n));
                    out.push((name, it.FmtValue.Anonymous.doubleValue));
                }
                return out;
            }
        }
        out
    }
}

// PDH 句柄只在采样线程里用
unsafe impl Send for Query {}
unsafe impl Send for Counter {}
