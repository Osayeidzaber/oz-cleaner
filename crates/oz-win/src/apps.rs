//! Reads installed programs from the registry (read-only) and runs the vendor's
//! own uninstaller (F-43). Never forces or edits anything.
use crate::wide;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;

#[derive(Debug, Clone)]
pub struct App {
    pub key: String,
    pub name: String,
    pub publisher: String,
    pub location: String,
    pub uninstall: String,
    pub est_kb: u64,
}

fn read_str(k: HKEY, name: &str) -> Option<String> {
    let w = wide(name);
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let mut ty = 0u32;
    // SAFETY: buffer and size match.
    let r = unsafe { RegQueryValueExW(k, w.as_ptr(), std::ptr::null(), &mut ty, buf.as_mut_ptr().cast(), &mut size) };
    if r != ERROR_SUCCESS || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
        return None;
    }
    let n = (size as usize / 2).min(buf.len());
    let end = buf[..n].iter().position(|&c| c == 0).unwrap_or(n);
    Some(String::from_utf16_lossy(&buf[..end]))
}

fn read_dword(k: HKEY, name: &str) -> Option<u32> {
    let w = wide(name);
    let (mut v, mut size, mut ty) = (0u32, 4u32, 0u32);
    // SAFETY: out pointers valid and sized.
    let r = unsafe { RegQueryValueExW(k, w.as_ptr(), std::ptr::null(), &mut ty, (&mut v as *mut u32).cast(), &mut size) };
    (r == ERROR_SUCCESS && ty == REG_DWORD).then_some(v)
}

pub fn list() -> Vec<App> {
    const PATHS: [(HKEY, &str); 3] = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];
    let mut out: Vec<App> = Vec::new();
    for (hive, path) in PATHS {
        let w = wide(path);
        let mut root: HKEY = std::ptr::null_mut();
        // SAFETY: valid strings; root closed below.
        if unsafe { RegOpenKeyExW(hive, w.as_ptr(), 0, KEY_READ, &mut root) } != ERROR_SUCCESS {
            continue;
        }
        let mut i = 0;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            // SAFETY: buffer and length match.
            let r = unsafe {
                RegEnumKeyExW(root, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            if r != ERROR_SUCCESS {
                break;
            }
            i += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            let ws = wide(&sub);
            let mut k: HKEY = std::ptr::null_mut();
            // SAFETY: valid strings; k closed below.
            if unsafe { RegOpenKeyExW(root, ws.as_ptr(), 0, KEY_READ, &mut k) } != ERROR_SUCCESS {
                continue;
            }
            let (dn, un) = (read_str(k, "DisplayName"), read_str(k, "UninstallString"));
            let hidden = read_dword(k, "SystemComponent") == Some(1);
            if let (Some(n), Some(u), false) = (dn, un, hidden) {
                if !out.iter().any(|a| a.name == n) {
                    out.push(App {
                        key: sub,
                        name: n,
                        publisher: read_str(k, "Publisher").unwrap_or_default(),
                        location: read_str(k, "InstallLocation").unwrap_or_default(),
                        uninstall: u,
                        est_kb: read_dword(k, "EstimatedSize").unwrap_or(0) as u64,
                    });
                }
            }
            // SAFETY: open key.
            unsafe { RegCloseKey(k) };
        }
        // SAFETY: open key.
        unsafe { RegCloseKey(root) };
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Starts the registered uninstaller and waits. The vendor's UI takes over.
/// Whatever it decides, including refusing, is reported, never overridden.
pub fn run_uninstaller(cmdline: &str) -> Result<i32, String> {
    use std::os::windows::process::CommandExt;
    let s = std::process::Command::new("cmd.exe")
        .raw_arg(format!("/d /c \"{cmdline}\""))
        .status()
        .map_err(|e| e.to_string())?;
    Ok(s.code().unwrap_or(-1))
}
