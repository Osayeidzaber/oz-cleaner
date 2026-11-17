use crate::wide;
use windows_sys::Win32::Storage::FileSystem::*;

#[derive(Debug, Clone)]
pub struct Volume {
    pub root: String,
    pub label: String,
    pub fs: String,
    pub kind: &'static str,
    pub total: u64,
    pub free: u64,
}

pub fn free_space(root: &str) -> Option<(u64, u64)> {
    let w = wide(root);
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: valid NUL-terminated path and out pointers for the call duration.
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0).then_some((total, free))
}

pub fn list() -> Vec<Volume> {
    let mut out = Vec::new();
    // SAFETY: plain call without pointers.
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w = wide(&root);
        // SAFETY: NUL-terminated string.
        let t = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(w.as_ptr()) };
        let kind = match t {
            2 => "removable",
            3 => "fixed",
            4 => "network",
            _ => continue,
        };
        let mut label = [0u16; 261];
        let mut fsn = [0u16; 64];
        // SAFETY: buffers and their lengths match.
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fsn.as_mut_ptr(),
                fsn.len() as u32,
            )
        };
        if ok == 0 {
            continue;
        }
        let Some((total, free)) = free_space(&root) else { continue };
        let s = |b: &[u16]| {
            let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            String::from_utf16_lossy(&b[..n])
        };
        out.push(Volume { root, label: s(&label), fs: s(&fsn), kind, total, free });
    }
    out
}
