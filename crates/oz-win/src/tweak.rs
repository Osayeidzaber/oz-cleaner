//! Registry and power-plan access for catalogued tweaks (D-14). Reading is free; writing is only
//! called for a tweak whose catalog entry is verified, and the caller saves the prior state first.
use crate::wide;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Power::{
    PowerGetActiveScheme, PowerReadACValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex,
};
use windows_sys::Win32::System::Registry::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegVal {
    Dword(u32),
    Sz(String),
}

fn root(hive: &str) -> Option<HKEY> {
    match hive {
        "HKCU" => Some(HKEY_CURRENT_USER),
        "HKLM" => Some(HKEY_LOCAL_MACHINE),
        _ => None,
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: handle came from RegOpenKeyExW / RegCreateKeyExW.
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(hive: &str, key: &str, access: u32) -> Result<Option<Key>, String> {
    let r = root(hive).ok_or("bad hive")?;
    let k = wide(key);
    let mut h: HKEY = std::ptr::null_mut();
    // SAFETY: valid wide string and out pointer.
    let e = unsafe { RegOpenKeyExW(r, k.as_ptr(), 0, access, &mut h) };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if e != ERROR_SUCCESS {
        return Err(format!("registry open failed ({e})"));
    }
    Ok(Some(Key(h)))
}

/// Current value, or None when the key or value doesn't exist (Windows default applies).
pub fn reg_read(hive: &str, key: &str, value: &str) -> Result<Option<RegVal>, String> {
    let Some(k) = open(hive, key, KEY_READ)? else {
        return Ok(None);
    };
    let name = wide(value);
    let mut buf = vec![0u8; 1024];
    let (mut size, mut ty) = (buf.len() as u32, 0u32);
    // SAFETY: buffer and size match.
    let e = unsafe {
        RegQueryValueExW(
            k.0,
            name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buf.as_mut_ptr(),
            &mut size,
        )
    };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if e != ERROR_SUCCESS {
        return Err(format!("registry read failed ({e})"));
    }
    match ty {
        REG_DWORD if size >= 4 => Ok(Some(RegVal::Dword(u32::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3],
        ])))),
        REG_SZ | REG_EXPAND_SZ => {
            let w: Vec<u16> = buf[..size as usize]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
            Ok(Some(RegVal::Sz(String::from_utf16_lossy(&w[..end]))))
        }
        _ => Err(format!("unexpected registry type {ty}")),
    }
}

/// Writes one value, creating the key if needed. HKCU only in-process; HKLM goes through the helper.
pub fn reg_write(hive: &str, key: &str, value: &str, v: &RegVal) -> Result<(), String> {
    if hive != "HKCU" {
        return Err("machine-wide settings are applied by the elevated helper".into());
    }
    write_any(hive, key, value, v)
}

/// Helper-only: writes HKCU or HKLM. The helper calls this only for catalogued tweak ids.
pub fn reg_write_machine(hive: &str, key: &str, value: &str, v: &RegVal) -> Result<(), String> {
    write_any(hive, key, value, v)
}

/// Helper-only counterpart of `reg_delete`.
pub fn reg_delete_machine(hive: &str, key: &str, value: &str) -> Result<(), String> {
    delete_any(hive, key, value)
}

fn write_any(hive: &str, key: &str, value: &str, v: &RegVal) -> Result<(), String> {
    let r = root(hive).ok_or("bad hive")?;
    let (k, name) = (wide(key), wide(value));
    let mut h: HKEY = std::ptr::null_mut();
    // SAFETY: valid pointers; no security attributes.
    let e = unsafe {
        RegCreateKeyExW(
            r,
            k.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut h,
            std::ptr::null_mut(),
        )
    };
    if e != ERROR_SUCCESS {
        return Err(format!("Windows refused to change this setting ({e})"));
    }
    let key = Key(h);
    let e = match v {
        RegVal::Dword(d) => {
            let b = d.to_le_bytes();
            // SAFETY: 4-byte buffer.
            unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, b.as_ptr(), 4) }
        }
        RegVal::Sz(s) => {
            let w = wide(s);
            // SAFETY: buffer includes the terminator.
            unsafe {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    w.as_ptr().cast(),
                    (w.len() * 2) as u32,
                )
            }
        }
    };
    if e != ERROR_SUCCESS {
        return Err(format!("Windows refused to change this setting ({e})"));
    }
    Ok(())
}

/// Removes a value (used to restore "not set"). Missing is success.
pub fn reg_delete(hive: &str, key: &str, value: &str) -> Result<(), String> {
    if hive != "HKCU" {
        return Err("machine-wide settings are applied by the elevated helper".into());
    }
    delete_any(hive, key, value)
}

fn delete_any(hive: &str, key: &str, value: &str) -> Result<(), String> {
    let Some(k) = open(hive, key, KEY_SET_VALUE)? else {
        return Ok(());
    };
    let name = wide(value);
    // SAFETY: valid key and name.
    let e = unsafe { RegDeleteValueW(k.0, name.as_ptr()) };
    if e != ERROR_SUCCESS && e != ERROR_FILE_NOT_FOUND {
        return Err(format!("registry delete failed ({e})"));
    }
    Ok(())
}

pub fn parse_guid(s: &str) -> Option<GUID> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
        .map(GUID::from_u128)
}

pub fn guid_string(g: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

pub fn active_scheme() -> Result<String, String> {
    let mut p: *mut GUID = std::ptr::null_mut();
    // SAFETY: out pointer; Windows allocates, we free with LocalFree.
    let e = unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut p) };
    if e != ERROR_SUCCESS || p.is_null() {
        return Err(format!("couldn't read the power plan ({e})"));
    }
    // SAFETY: p is a valid GUID from Windows.
    let s = guid_string(unsafe { &*p });
    // SAFETY: allocated by PowerGetActiveScheme.
    unsafe { LocalFree(p.cast()) };
    Ok(s)
}

pub fn set_active_scheme(guid: &str) -> Result<(), String> {
    let g = parse_guid(guid).ok_or("bad GUID")?;
    // SAFETY: valid GUID pointer.
    let e = unsafe { PowerSetActiveScheme(std::ptr::null_mut(), &g) };
    if e != ERROR_SUCCESS {
        return Err(format!("Windows refused to change the power plan ({e})"));
    }
    Ok(())
}

/// AC value of a power setting in the active scheme.
pub fn power_value(subgroup: &str, setting: &str) -> Result<u32, String> {
    let scheme = parse_guid(&active_scheme()?).ok_or("bad scheme")?;
    let (sg, st) = (
        parse_guid(subgroup).ok_or("bad GUID")?,
        parse_guid(setting).ok_or("bad GUID")?,
    );
    let mut v = 0u32;
    // SAFETY: valid GUID pointers and out value.
    let e = unsafe { PowerReadACValueIndex(std::ptr::null_mut(), &scheme, &sg, &st, &mut v) };
    if e != ERROR_SUCCESS {
        return Err(format!("couldn't read the power setting ({e})"));
    }
    Ok(v)
}

/// Helper-only: sets the AC value of a power setting in the active scheme and re-applies the scheme.
pub fn set_power_value(subgroup: &str, setting: &str, value: u32) -> Result<(), String> {
    let scheme = parse_guid(&active_scheme()?).ok_or("bad scheme")?;
    let (sg, st) = (
        parse_guid(subgroup).ok_or("bad GUID")?,
        parse_guid(setting).ok_or("bad GUID")?,
    );
    // SAFETY: valid GUID pointers.
    let e = unsafe { PowerWriteACValueIndex(std::ptr::null_mut(), &scheme, &sg, &st, value) };
    if e != ERROR_SUCCESS {
        return Err(format!("Windows refused to change the power setting ({e})"));
    }
    // SAFETY: valid GUID pointer; re-activating applies the new value.
    let e = unsafe { PowerSetActiveScheme(std::ptr::null_mut(), &scheme) };
    if e != ERROR_SUCCESS {
        return Err(format!("Windows refused to apply the power setting ({e})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_round_trip() {
        let s = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
        assert_eq!(guid_string(&parse_guid(s).unwrap()), s);
        assert!(parse_guid("nope").is_none());
    }

    #[test]
    fn reads_are_harmless() {
        // A key that never exists reads as "not set".
        assert_eq!(
            reg_read("HKCU", r"Software\OzCleaner\DoesNotExist\Ever", "x").unwrap(),
            None
        );
        assert!(active_scheme().is_ok());
    }

    #[test]
    fn hklm_writes_refused_in_process() {
        assert!(reg_write("HKLM", r"Software\X", "y", &RegVal::Dword(1)).is_err());
        assert!(reg_delete("HKLM", r"Software\X", "y").is_err());
    }

    /// Writes only under the app's own scratch key. Ignored by default; run in Windows Sandbox:
    /// `cargo test -p oz-win -- --ignored scratch_round_trip`.
    #[test]
    #[ignore]
    fn scratch_round_trip() {
        let k = r"Software\OzCleaner\TweakScratch";
        reg_write("HKCU", k, "d", &RegVal::Dword(7)).unwrap();
        assert_eq!(reg_read("HKCU", k, "d").unwrap(), Some(RegVal::Dword(7)));
        reg_write("HKCU", k, "s", &RegVal::Sz("100".into())).unwrap();
        assert_eq!(
            reg_read("HKCU", k, "s").unwrap(),
            Some(RegVal::Sz("100".into()))
        );
        reg_delete("HKCU", k, "d").unwrap();
        reg_delete("HKCU", k, "s").unwrap();
        assert_eq!(reg_read("HKCU", k, "d").unwrap(), None);
    }
}
