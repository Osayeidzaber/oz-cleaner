//! Primitive file operations. All use `\\?\` paths. Policy (what may be touched)
//! lives in oz-ops; nothing here changes ACLs or ownership (A-7).
use crate::{long_path, long_wide, wide};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::UI::Shell::*;

pub const ATTR_DIR: u32 = 0x10;
pub const ATTR_REPARSE: u32 = 0x400;
pub const ATTR_READONLY: u32 = 0x1;
pub const ATTR_RECALL: u32 = 0x40000 | 0x400000 | 0x1000;

pub const ERR_ACCESS_DENIED: u32 = 5;
pub const ERR_SHARING: u32 = 32;
pub const ERR_LOCK: u32 = 33;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Meta {
    pub attrs: u32,
    pub size: u64,
    pub modified: i64,
}

fn last() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetLastError() }
}

/// Stat without following links.
pub fn meta(path: &str) -> Result<Meta, u32> {
    let w = long_wide(path);
    // SAFETY: zeroed POD filled by the call.
    let mut d: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    // SAFETY: valid path and out struct.
    let ok = unsafe {
        GetFileAttributesExW(
            w.as_ptr(),
            GetFileExInfoStandard,
            (&mut d as *mut WIN32_FILE_ATTRIBUTE_DATA).cast(),
        )
    };
    if ok == 0 {
        return Err(last());
    }
    Ok(Meta {
        attrs: d.dwFileAttributes,
        size: (d.nFileSizeHigh as u64) << 32 | d.nFileSizeLow as u64,
        modified: ((d.ftLastWriteTime.dwHighDateTime as u64) << 32
            | d.ftLastWriteTime.dwLowDateTime as u64) as i64,
    })
}

pub fn exists(path: &str) -> bool {
    meta(path).is_ok()
}

/// Names in a directory, not following links.
pub fn list_dir(path: &str) -> Result<Vec<String>, u32> {
    let pat = wide(&format!("{}\\*", long_path(path).trim_end_matches('\\')));
    // SAFETY: zeroed POD filled by the call.
    let mut d: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointers; handle closed below.
    let h = unsafe { FindFirstFileW(pat.as_ptr(), &mut d) };
    if h == INVALID_HANDLE_VALUE {
        return Err(last());
    }
    let mut out = Vec::new();
    loop {
        let n = d
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(d.cFileName.len());
        let name = String::from_utf16_lossy(&d.cFileName[..n]);
        if name != "." && name != ".." {
            out.push(name);
        }
        // SAFETY: open find handle.
        if unsafe { FindNextFileW(h, &mut d) } == 0 {
            break;
        }
    }
    // SAFETY: open find handle.
    unsafe { FindClose(h) };
    Ok(out)
}

pub fn delete_file(path: &str) -> Result<(), u32> {
    let w = long_wide(path);
    // SAFETY: NUL-terminated path.
    if unsafe { DeleteFileW(w.as_ptr()) } != 0 {
        return Ok(());
    }
    let e = last();
    if e == ERR_ACCESS_DENIED {
        // A read-only file in an approved plan may have that attribute cleared.
        if let Ok(m) = meta(path) {
            if m.attrs & ATTR_READONLY != 0 {
                // SAFETY: NUL-terminated path.
                unsafe {
                    SetFileAttributesW(w.as_ptr(), m.attrs & !ATTR_READONLY);
                    if DeleteFileW(w.as_ptr()) != 0 {
                        return Ok(());
                    }
                }
                return Err(last());
            }
        }
    }
    Err(e)
}

pub fn remove_empty_dir(path: &str) -> Result<(), u32> {
    let w = long_wide(path);
    // SAFETY: NUL-terminated path.
    if unsafe { RemoveDirectoryW(w.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(last())
    }
}

/// Removes a junction or symlink itself; the target is untouched.
pub fn remove_link(path: &str) -> Result<(), u32> {
    let m = meta(path)?;
    if m.attrs & ATTR_REPARSE == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if m.attrs & ATTR_DIR != 0 {
        remove_empty_dir(path)
    } else {
        delete_file(path)
    }
}

/// Removes a tree depth-first. Links are removed, never entered. Stops with the
/// first error from `stop` or a failure to enumerate; individual failures are
/// returned by `remove_tree_report`.
pub fn remove_tree(path: &str, stop: &dyn Fn() -> bool) -> Result<(), u32> {
    let mut fails = Vec::new();
    remove_tree_report(path, stop, &mut |_| {}, &mut fails);
    match fails.first() {
        Some((_, e)) => Err(*e),
        None => Ok(()),
    }
}

/// Deletes what it can. `on_removed` receives bytes for each deleted file.
pub fn remove_tree_report(
    path: &str,
    stop: &dyn Fn() -> bool,
    on_removed: &mut dyn FnMut(u64),
    fails: &mut Vec<(String, u32)>,
) {
    if stop() {
        return;
    }
    let m = match meta(path) {
        Ok(m) => m,
        Err(e) => {
            fails.push((path.to_string(), e));
            return;
        }
    };
    if m.attrs & ATTR_REPARSE != 0 && m.attrs & ATTR_RECALL == 0 && m.attrs & ATTR_DIR != 0 {
        if let Err(e) = remove_empty_dir(path) {
            fails.push((path.to_string(), e));
        }
        return;
    }
    if m.attrs & ATTR_RECALL != 0 {
        // Cloud placeholder: deleting could delete the cloud copy (A-7).
        fails.push((path.to_string(), ERROR_CLOUD_FILE_PLACEHOLDER));
        return;
    }
    if m.attrs & ATTR_DIR == 0 {
        match delete_file(path) {
            Ok(()) => on_removed(m.size),
            Err(e) => fails.push((path.to_string(), e)),
        }
        return;
    }
    let names = match list_dir(path) {
        Ok(n) => n,
        Err(e) => {
            fails.push((path.to_string(), e));
            return;
        }
    };
    for n in names {
        remove_tree_report(
            &format!("{}\\{n}", path.trim_end_matches('\\')),
            stop,
            on_removed,
            fails,
        );
    }
    if !stop() {
        if let Err(e) = remove_empty_dir(path) {
            // Non-empty after a child failure is already reported by that child.
            if e != ERROR_DIR_NOT_EMPTY {
                fails.push((path.to_string(), e));
            }
        }
    }
}

/// A distinctive internal code for placeholders refused by policy.
pub const ERROR_CLOUD_FILE_PLACEHOLDER: u32 = 0x2000_0001;

/// Same-volume rename; fails rather than copies across volumes.
pub fn rename(from: &str, to: &str) -> Result<(), u32> {
    let (f, t) = (long_wide(from), long_wide(to));
    // SAFETY: NUL-terminated paths; flags 0 forbids overwrite and cross-volume copy.
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(last())
    }
}

pub fn create_dirs(path: &str) -> Result<(), u32> {
    std::fs::create_dir_all(long_path(path)).map_err(|e| e.raw_os_error().unwrap_or(0) as u32)
}

/// Sends one path to the Recycle Bin. Callers must refuse paths of 260 chars or
/// more first: the legacy shell call is documented to fail on them.
pub fn recycle(path: &str) -> Result<(), u32> {
    let mut from = wide(path);
    from.push(0); // double NUL terminated
                  // SAFETY: zeroed POD; `from` outlives the call.
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags =
        (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_NO_UI) as u16;
    // SAFETY: op is fully initialized for FO_DELETE.
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 {
        return Err(r as u32);
    }
    if op.fAnyOperationsAborted != 0 {
        return Err(ERROR_CANCELLED);
    }
    Ok(())
}

/// Creates a junction at `link` (a new empty folder) pointing to absolute `target`.
pub fn create_junction(link: &str, target: &str) -> Result<(), String> {
    create_dirs(link).map_err(|e| format!("create folder: {e}"))?;
    let w = long_wide(link);
    // SAFETY: NUL-terminated path; handle closed below.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        let e = last();
        let _ = remove_empty_dir(link);
        return Err(format!("open: {e}"));
    }
    let clean = target.trim_start_matches(r"\\?\").trim_end_matches('\\');
    let sub: Vec<u16> = format!(r"\??\{clean}").encode_utf16().collect();
    let print: Vec<u16> = clean.encode_utf16().collect();
    let path_len = (sub.len() + print.len() + 2) * 2;
    let data_len = 8 + path_len;
    let mut buf = Vec::with_capacity(8 + data_len);
    buf.extend_from_slice(&0xA000_0003u32.to_le_bytes());
    buf.extend_from_slice(&(data_len as u16).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&((sub.len() * 2) as u16).to_le_bytes());
    buf.extend_from_slice(&(((sub.len() + 1) * 2) as u16).to_le_bytes());
    buf.extend_from_slice(&((print.len() * 2) as u16).to_le_bytes());
    for c in sub.iter().chain(&[0u16]).chain(print.iter()).chain(&[0u16]) {
        buf.extend_from_slice(&c.to_le_bytes());
    }
    let mut ret = 0u32;
    // SAFETY: buffer valid for its length; handle open.
    let ok = unsafe {
        DeviceIoControl(
            h,
            FSCTL_SET_REPARSE_POINT,
            buf.as_ptr().cast(),
            buf.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut ret,
            std::ptr::null_mut(),
        )
    };
    let e = last();
    // SAFETY: open handle.
    unsafe { CloseHandle(h) };
    if ok == 0 {
        let _ = remove_empty_dir(link);
        return Err(format!("set reparse point: {e}"));
    }
    Ok(())
}

/// Opens Explorer with the item selected.
pub fn reveal(path: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.replace('"', "")))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens a shell target such as an `ms-settings:` URI. Only called with
/// fixed strings from the app, never with data from disk.
pub fn open_uri(uri: &str) -> Result<(), String> {
    let (op, u) = (wide("open"), wide(uri));
    // SAFETY: NUL-terminated strings.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            u.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if r as isize > 32 {
        Ok(())
    } else {
        Err(format!("shell error {}", r as isize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> String {
        let p = std::env::temp_dir().join(format!("oz-fsops-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn long_path_tree_removed() {
        let r = scratch("long");
        let mut d = r.clone();
        for _ in 0..40 {
            d.push_str("\\abcdefghij");
        }
        assert!(d.len() > 400);
        create_dirs(&d).unwrap();
        fs::write(long_path(&format!("{d}\\f.txt")), b"x").unwrap();
        assert!(exists(&format!("{d}\\f.txt")));
        remove_tree(&r, &|| false).unwrap();
        assert!(!exists(&r));
    }

    #[test]
    fn tree_delete_does_not_traverse_junction() {
        let r = scratch("junc");
        let t = scratch("junc-target");
        fs::write(format!("{t}\\keep.txt"), b"precious").unwrap();
        create_junction(&format!("{r}\\link"), &t).unwrap();
        assert!(exists(&format!("{r}\\link\\keep.txt")), "junction works");
        remove_tree(&r, &|| false).unwrap();
        assert!(!exists(&r));
        assert_eq!(fs::read(format!("{t}\\keep.txt")).unwrap(), b"precious");
        remove_tree(&t, &|| false).unwrap();
    }

    #[test]
    fn read_only_file_deleted_and_locked_reported() {
        let r = scratch("ro");
        let f = format!("{r}\\ro.txt");
        fs::write(&f, b"x").unwrap();
        let mut p = fs::metadata(&f).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(&f, p).unwrap();
        delete_file(&f).unwrap();

        let locked = format!("{r}\\locked.txt");
        use std::os::windows::fs::OpenOptionsExt;
        let h = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .share_mode(0)
            .open(&locked)
            .unwrap();
        let mut fails = Vec::new();
        remove_tree_report(&r, &|| false, &mut |_| {}, &mut fails);
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0].1, ERR_SHARING);
        drop(h);
        remove_tree(&r, &|| false).unwrap();
    }

    #[test]
    fn rename_refuses_overwrite() {
        let r = scratch("mv");
        fs::write(format!("{r}\\a"), b"a").unwrap();
        fs::write(format!("{r}\\b"), b"b").unwrap();
        assert!(rename(&format!("{r}\\a"), &format!("{r}\\b")).is_err());
        rename(&format!("{r}\\a"), &format!("{r}\\c")).unwrap();
        remove_tree(&r, &|| false).unwrap();
    }
}
