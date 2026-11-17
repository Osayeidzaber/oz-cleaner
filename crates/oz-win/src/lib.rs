//! All Windows FFI lives here (A-5).
pub mod apps;
pub mod elevate;
pub mod fsops;
pub mod mft;
pub mod tweak;
pub mod volume;
pub mod walk;

use std::os::windows::ffi::OsStrExt;

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Normalizes a user-facing absolute path to `\\?\` form (A-6).
pub fn long_path(p: &str) -> String {
    let p = p.replace('/', "\\");
    if p.starts_with(r"\\?\") {
        p
    } else if let Some(unc) = p.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else {
        format!(r"\\?\{p}")
    }
}

pub fn long_wide(p: &str) -> Vec<u16> {
    wide(&long_path(p))
}

/// Current time as a FILETIME value.
pub fn now_filetime() -> i64 {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    EPOCH_DIFF + (d.as_nanos() / 100) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_paths() {
        assert_eq!(long_path("C:/a"), r"\\?\C:\a");
        assert_eq!(long_path(r"\\?\C:\a"), r"\\?\C:\a");
        assert_eq!(long_path(r"\\srv\s"), r"\\?\UNC\srv\s");
    }
}
