//! Backup, removal and restore of a leftover HKCU registry key (AGENTS.md section 6b).
//! A key is always exported in full before it is deleted, and the export restores it exactly.
use crate::wide;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::*;

/// One value: name, registry type, raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub name: String,
    pub ty: u32,
    pub data: Vec<u8>,
}

/// A key and everything below it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    pub values: Vec<Value>,
    pub keys: Vec<(String, Tree)>,
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: handle from RegOpenKeyExW / RegCreateKeyExW.
        unsafe { RegCloseKey(self.0) };
    }
}

fn hkcu_only(path: &str) -> Result<(), String> {
    let p = path.to_ascii_lowercase();
    // Leftover removal is limited to per-user software keys of a named vendor or app.
    if !p.starts_with("software\\") || p.matches('\\').count() < 1 || p.contains("..") {
        return Err("only per-user Software keys can be removed".into());
    }
    for bad in [
        "software\\microsoft",
        "software\\classes",
        "software\\policies",
        "software\\wow6432node",
    ] {
        if p == bad || p.starts_with(&format!("{bad}\\")) {
            return Err("that registry area is protected".into());
        }
    }
    Ok(())
}

fn open(parent: HKEY, sub: &str, access: u32) -> Result<Option<Key>, String> {
    let w = wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    // SAFETY: valid string and out pointer.
    let e = unsafe { RegOpenKeyExW(parent, w.as_ptr(), 0, access, &mut h) };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if e != ERROR_SUCCESS {
        return Err(format!("registry open failed ({e})"));
    }
    Ok(Some(Key(h)))
}

/// Names of the direct subkeys of an HKCU key (read-only).
pub fn subkeys(path: &str) -> Vec<String> {
    let Ok(Some(k)) = open(HKEY_CURRENT_USER, path, KEY_READ) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut j = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut nlen = name.len() as u32;
        // SAFETY: buffer and length match.
        let e = unsafe {
            RegEnumKeyExW(
                k.0,
                j,
                name.as_mut_ptr(),
                &mut nlen,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if e != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..nlen as usize]));
        j += 1;
    }
    out
}

pub fn exists(path: &str) -> bool {
    matches!(open(HKEY_CURRENT_USER, path, KEY_READ), Ok(Some(_)))
}

fn read_tree(k: HKEY, depth: u32) -> Result<Tree, String> {
    if depth > 32 {
        return Err("registry key nests too deeply".into());
    }
    let mut t = Tree::default();
    let mut i = 0u32;
    loop {
        let mut name = vec![0u16; 16384];
        let mut nlen = name.len() as u32;
        let mut data = vec![0u8; 1 << 16];
        let mut dlen = data.len() as u32;
        let mut ty = 0u32;
        // SAFETY: buffers and lengths match.
        let e = unsafe {
            RegEnumValueW(
                k,
                i,
                name.as_mut_ptr(),
                &mut nlen,
                std::ptr::null(),
                &mut ty,
                data.as_mut_ptr(),
                &mut dlen,
            )
        };
        if e == ERROR_NO_MORE_ITEMS {
            break;
        }
        if e != ERROR_SUCCESS {
            return Err(format!(
                "couldn't read a registry value ({e}); nothing was removed"
            ));
        }
        data.truncate(dlen as usize);
        t.values.push(Value {
            name: String::from_utf16_lossy(&name[..nlen as usize]),
            ty,
            data,
        });
        i += 1;
    }
    let mut j = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut nlen = name.len() as u32;
        // SAFETY: buffer and length match.
        let e = unsafe {
            RegEnumKeyExW(
                k,
                j,
                name.as_mut_ptr(),
                &mut nlen,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if e == ERROR_NO_MORE_ITEMS {
            break;
        }
        if e != ERROR_SUCCESS {
            return Err(format!(
                "couldn't read a registry key ({e}); nothing was removed"
            ));
        }
        let sub = String::from_utf16_lossy(&name[..nlen as usize]);
        let child = open(k, &sub, KEY_READ)?.ok_or("key vanished while reading")?;
        t.keys.push((sub, read_tree(child.0, depth + 1)?));
        j += 1;
    }
    Ok(t)
}

/// Full copy of an HKCU key, or None if it doesn't exist.
pub fn export(path: &str) -> Result<Option<Tree>, String> {
    hkcu_only(path)?;
    match open(HKEY_CURRENT_USER, path, KEY_READ)? {
        None => Ok(None),
        Some(k) => read_tree(k.0, 0).map(Some),
    }
}

/// Deletes an HKCU key and everything below it. Callers export first.
pub fn delete(path: &str) -> Result<(), String> {
    hkcu_only(path)?;
    let w = wide(path);
    // SAFETY: valid string.
    let e = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, w.as_ptr()) };
    if e != ERROR_SUCCESS && e != ERROR_FILE_NOT_FOUND {
        return Err(format!("Windows refused to remove the key ({e})"));
    }
    // RegDeleteTreeW leaves the key itself; remove it too.
    // SAFETY: valid string.
    let e = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, w.as_ptr()) };
    if e != ERROR_SUCCESS && e != ERROR_FILE_NOT_FOUND {
        return Err(format!("Windows refused to remove the key ({e})"));
    }
    Ok(())
}

fn write_tree(parent: HKEY, sub: &str, t: &Tree) -> Result<(), String> {
    let w = wide(sub);
    let mut h: HKEY = std::ptr::null_mut();
    // SAFETY: valid pointers.
    let e = unsafe {
        RegCreateKeyExW(
            parent,
            w.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_ALL_ACCESS,
            std::ptr::null(),
            &mut h,
            std::ptr::null_mut(),
        )
    };
    if e != ERROR_SUCCESS {
        return Err(format!("couldn't recreate the key ({e})"));
    }
    let k = Key(h);
    for v in &t.values {
        let n = wide(&v.name);
        // SAFETY: data buffer and length match.
        let e = unsafe {
            RegSetValueExW(
                k.0,
                n.as_ptr(),
                0,
                v.ty,
                v.data.as_ptr(),
                v.data.len() as u32,
            )
        };
        if e != ERROR_SUCCESS {
            return Err(format!("couldn't restore a value ({e})"));
        }
    }
    for (name, child) in &t.keys {
        write_tree(k.0, name, child)?;
    }
    Ok(())
}

/// Recreates an exported key exactly.
pub fn restore(path: &str, t: &Tree) -> Result<(), String> {
    hkcu_only(path)?;
    write_tree(HKEY_CURRENT_USER, path, t)
}

/// Compact text form for the backup file: one line per value, `key\tname\ttype\thex`.
pub fn to_lines(t: &Tree, prefix: &str, out: &mut Vec<String>) {
    out.push(format!("K\t{prefix}"));
    for v in &t.values {
        let hex: String = v.data.iter().map(|b| format!("{b:02x}")).collect();
        out.push(format!(
            "V\t{prefix}\t{}\t{}\t{hex}",
            v.name.replace(['\t', '\n'], " "),
            v.ty
        ));
    }
    for (n, c) in &t.keys {
        to_lines(c, &format!("{prefix}\\{n}"), out);
    }
}

/// Parses `to_lines` output back into a tree rooted at `root`.
pub fn from_lines(lines: &[String], root: &str) -> Option<Tree> {
    fn node<'a>(t: &'a mut Tree, rel: &str) -> &'a mut Tree {
        if rel.is_empty() {
            return t;
        }
        let (head, rest) = rel.split_once('\\').unwrap_or((rel, ""));
        let idx = match t.keys.iter().position(|(n, _)| n == head) {
            Some(i) => i,
            None => {
                t.keys.push((head.to_string(), Tree::default()));
                t.keys.len() - 1
            }
        };
        node(&mut t.keys[idx].1, rest)
    }
    let mut t = Tree::default();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        let rel = |p: &str| {
            p.strip_prefix(root)
                .map(|r| r.trim_start_matches('\\').to_string())
        };
        match f.as_slice() {
            ["K", p] => {
                node(&mut t, &rel(p)?);
            }
            ["V", p, name, ty, hex] => {
                let data = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                    .collect::<Option<Vec<u8>>>()?;
                node(&mut t, &rel(p)?).values.push(Value {
                    name: name.to_string(),
                    ty: ty.parse().ok()?,
                    data,
                });
            }
            _ => return None,
        }
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_areas_refused() {
        for p in [
            r"Software\Microsoft\Windows",
            r"Software\Classes\CLSID",
            r"Software\Policies\X",
            r"System\X",
            r"Software",
            r"Software\..\X",
        ] {
            assert!(hkcu_only(p).is_err(), "{p}");
        }
        assert!(hkcu_only(r"Software\SomeVendor\SomeApp").is_ok());
    }

    #[test]
    fn lines_round_trip() {
        let t = Tree {
            values: vec![
                Value {
                    name: "a".into(),
                    ty: REG_DWORD,
                    data: vec![1, 0, 0, 0],
                },
                Value {
                    name: "".into(),
                    ty: REG_SZ,
                    data: vec![104, 0, 0, 0],
                },
            ],
            keys: vec![(
                "sub".into(),
                Tree {
                    values: vec![Value {
                        name: "b".into(),
                        ty: REG_BINARY,
                        data: vec![],
                    }],
                    keys: vec![("deep".into(), Tree::default())],
                },
            )],
        };
        let mut lines = Vec::new();
        to_lines(&t, r"Software\V\A", &mut lines);
        assert_eq!(from_lines(&lines, r"Software\V\A").unwrap(), t);
    }

    /// Uses only the app's own scratch key. Run explicitly: `cargo test -p oz-win -- --ignored regtree`.
    #[test]
    #[ignore]
    fn regtree_scratch_round_trip() {
        let p = r"Software\OzCleanerScratch\Leftover";
        let t = Tree {
            values: vec![Value {
                name: "x".into(),
                ty: REG_DWORD,
                data: vec![7, 0, 0, 0],
            }],
            keys: vec![(
                "child".into(),
                Tree {
                    values: vec![Value {
                        name: "s".into(),
                        ty: REG_SZ,
                        data: vec![65, 0, 0, 0],
                    }],
                    keys: vec![],
                },
            )],
        };
        restore(p, &t).unwrap();
        let got = export(p).unwrap().unwrap();
        assert_eq!(got, t);
        delete(p).unwrap();
        assert!(!exists(p));
        restore(p, &got).unwrap();
        assert_eq!(export(p).unwrap().unwrap(), t);
        delete(p).unwrap();
        delete(r"Software\OzCleanerScratch").unwrap();
    }
}
