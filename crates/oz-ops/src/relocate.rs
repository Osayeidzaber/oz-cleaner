//! Relocate (F-42): verified copy, then a junction at the old location.
//! Every failure before the junction exists leaves the source untouched.
use crate::guard::Guard;
use blake3::Hasher;
use oz_win::fsops;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Relocated {
    pub source: String,
    pub dest: String,
    pub old: String,
    pub files: u64,
    pub bytes: u64,
}

pub const OLD_SUFFIX: &str = ".ozold";

#[derive(Default)]
struct Tree {
    files: Vec<(String, u64)>,
    dirs: Vec<String>,
}

fn walk(root: &str, rel: &str, t: &mut Tree) -> Result<(), String> {
    let full = if rel.is_empty() { root.to_string() } else { format!("{root}\\{rel}") };
    for n in fsops::list_dir(&full).map_err(|e| format!("Cannot read {full}: {e}"))? {
        let r = if rel.is_empty() { n.clone() } else { format!("{rel}\\{n}") };
        let m = fsops::meta(&format!("{root}\\{r}")).map_err(|e| format!("Cannot read {r}: {e}"))?;
        if m.attrs & fsops::ATTR_RECALL != 0 {
            return Err(format!("{r} is a cloud placeholder; cloud folders can't be relocated"));
        }
        if m.attrs & fsops::ATTR_REPARSE != 0 {
            return Err(format!("{r} is a link; folders containing links can't be relocated"));
        }
        if m.attrs & fsops::ATTR_DIR != 0 {
            t.dirs.push(r.clone());
            walk(root, &r, t)?;
        } else {
            t.files.push((r, m.size));
        }
    }
    Ok(())
}

fn hash(path: &str) -> Result<blake3::Hash, String> {
    let mut f = fs::File::open(oz_win::long_path(path)).map_err(|e| format!("{path}: {e}"))?;
    let mut h = Hasher::new();
    let mut b = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut b).map_err(|e| format!("{path}: {e}"))?;
        if n == 0 {
            return Ok(h.finalize());
        }
        h.update(&b[..n]);
    }
}

/// Moves `source` to `dest_parent\<name>`. `on_progress(done_bytes, total_bytes)`.
pub fn relocate(g: &Guard, source: &str, dest_parent: &str, stop: &AtomicBool, mut on_progress: impl FnMut(u64, u64)) -> Result<Relocated, String> {
    let source = source.trim_end_matches('\\').to_string();
    let dest_parent = dest_parent.trim_end_matches('\\').to_string();
    g.check(&source).map_err(|r| format!("Refused: {}", r.0))?;
    g.check(&format!("{dest_parent}\\x")).map_err(|r| format!("Refused destination: {}", r.0))?;
    let m = fsops::meta(&source).map_err(|e| format!("Source: {}", crate::plan::reason_text(e)))?;
    if m.attrs & fsops::ATTR_DIR == 0 {
        return Err("Only folders can be relocated".into());
    }
    if m.attrs & fsops::ATTR_REPARSE != 0 {
        return Err("Source is already a link".into());
    }
    let name = source.rsplit('\\').next().unwrap_or("").to_string();
    let dest = format!("{dest_parent}\\{name}");
    let (ls, ld) = (format!("{}\\", source.to_lowercase()), format!("{}\\", dest.to_lowercase()));
    if ld.starts_with(&ls) || ls.starts_with(&ld) {
        return Err("Destination overlaps the source".into());
    }
    if fsops::exists(&dest) {
        return Err("Destination already exists".into());
    }
    let old = format!("{source}{OLD_SUFFIX}");
    if fsops::exists(&old) {
        return Err(format!("{old} already exists from an earlier move"));
    }

    let mut t = Tree::default();
    walk(&source, "", &mut t)?;
    let total: u64 = t.files.iter().map(|f| f.1).sum();
    if let Some((_, free)) = oz_win::volume::free_space(&format!("{}\\", &dest_parent[..2])) {
        if free < total.saturating_add(1 << 30) {
            return Err("Not enough free space at the destination (1 GB margin kept)".into());
        }
    }

    let result = copy_and_verify(&source, &dest, &t, total, stop, &mut on_progress);
    if let Err(e) = result {
        let _ = fsops::remove_tree(&dest, &|| false);
        return Err(e);
    }

    // Rename fails if anything inside is open: that is the "nothing is using it" check.
    if let Err(e) = fsops::rename(&source, &old) {
        let _ = fsops::remove_tree(&dest, &|| false);
        return Err(format!("The folder is in use, so nothing was changed ({})", crate::plan::reason_text(e)));
    }
    let undo_rename = |why: String| {
        let _ = fsops::remove_link(&source);
        let renamed = fsops::rename(&old, &source).is_ok();
        if renamed {
            let _ = fsops::remove_tree(&dest, &|| false);
        }
        format!("{why}. Rolled back: {}", if renamed { "source restored" } else { "RESTORE FAILED, data is safe at the .ozold folder and destination" })
    };
    if let Err(e) = fsops::create_junction(&source, &dest) {
        return Err(undo_rename(format!("Could not create junction: {e}")));
    }
    // Prove the junction resolves by reading a file through it.
    let probe_ok = match t.files.first() {
        Some((r, _)) => hash(&format!("{source}\\{r}")).is_ok(),
        None => fsops::list_dir(&source).is_ok(),
    };
    if !probe_ok {
        return Err(undo_rename("Junction did not work".into()));
    }
    Ok(Relocated { source, dest, old, files: t.files.len() as u64, bytes: total })
}

fn copy_and_verify(source: &str, dest: &str, t: &Tree, total: u64, stop: &AtomicBool, on_progress: &mut impl FnMut(u64, u64)) -> Result<(), String> {
    fsops::create_dirs(dest).map_err(|e| format!("Create destination: {e}"))?;
    for d in &t.dirs {
        fsops::create_dirs(&format!("{dest}\\{d}")).map_err(|e| format!("Create {d}: {e}"))?;
    }
    let mut done = 0u64;
    for (r, size) in &t.files {
        if stop.load(Ordering::Relaxed) {
            return Err("Cancelled; source untouched".into());
        }
        let (a, b) = (format!("{source}\\{r}"), format!("{dest}\\{r}"));
        fs::copy(oz_win::long_path(&a), oz_win::long_path(&b)).map_err(|e| format!("Copy {r}: {e}"))?;
        done += size;
        on_progress(done, total);
    }
    // Re-read everything: sizes, then content hashes.
    let mut check = Tree::default();
    walk(dest, "", &mut check)?;
    if check.files.len() != t.files.len() || check.dirs.len() != t.dirs.len() {
        return Err("Verification failed: item counts differ".into());
    }
    for (r, size) in &t.files {
        if stop.load(Ordering::Relaxed) {
            return Err("Cancelled; source untouched".into());
        }
        let (a, b) = (format!("{source}\\{r}"), format!("{dest}\\{r}"));
        if fsops::meta(&b).map(|m| m.size) != Ok(*size) {
            return Err(format!("Verification failed: size of {r}"));
        }
        if hash(&a)? != hash(&b)? {
            return Err(format!("Verification failed: contents of {r}"));
        }
    }
    Ok(())
}

/// Reverses a relocation while the original still exists as `.ozold`.
pub fn undo(g: &Guard, r: &Relocated) -> Result<(), String> {
    g.check(&r.source).map_err(|e| format!("Refused: {}", e.0))?;
    let m = fsops::meta(&r.source).map_err(|e| crate::plan::reason_text(e))?;
    if m.attrs & fsops::ATTR_REPARSE == 0 {
        return Err("Source is no longer a junction".into());
    }
    if !fsops::exists(&r.old) {
        return Err("The original copy was removed, so this can't be undone".into());
    }
    fsops::remove_link(&r.source).map_err(crate::plan::reason_text)?;
    fsops::rename(&r.old, &r.source).map_err(crate::plan::reason_text)?;
    fsops::remove_tree(&r.dest, &|| false).map_err(crate::plan::reason_text)
}

/// Removes the `.ozold` original, which frees the space. Not reversible.
pub fn finish(g: &Guard, r: &Relocated) -> Result<(), String> {
    g.check(&r.old).map_err(|e| format!("Refused: {}", e.0))?;
    if !r.old.ends_with(OLD_SUFFIX) {
        return Err("Unexpected path".into());
    }
    let m = fsops::meta(&r.source).map_err(crate::plan::reason_text)?;
    if m.attrs & fsops::ATTR_REPARSE == 0 {
        return Err("Source is not a junction; keeping the original".into());
    }
    fsops::remove_tree(&r.old, &|| false).map_err(crate::plan::reason_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct S(String);
    impl S {
        fn new(t: &str) -> Self {
            let p = std::env::temp_dir().join(format!("oz-rel-{t}-{}", std::process::id()));
            let _ = fsops::remove_tree(p.to_str().unwrap(), &|| false);
            fs::create_dir_all(&p).unwrap();
            Self(p.to_str().unwrap().to_string())
        }
    }
    impl Drop for S {
        fn drop(&mut self) {
            let _ = fsops::remove_link(&format!("{}\\data", self.0));
            let _ = fsops::remove_tree(&self.0, &|| false);
        }
    }
    fn g() -> Guard {
        Guard::new(r"C:\Windows", r"C:\NoSuchUsers\me", &[r"C:\Program Files"], r"C:\ProgramData")
    }
    fn fill(root: &str) -> String {
        let src = format!("{root}\\data");
        let mut deep = src.clone();
        for _ in 0..30 {
            deep.push_str("\\0123456789");
        }
        fsops::create_dirs(&deep).unwrap();
        fs::write(oz_win::long_path(&format!("{deep}\\deep.bin")), vec![7u8; 5000]).unwrap();
        fs::write(format!("{src}\\a.txt"), b"hello").unwrap();
        src
    }
    const NO: AtomicBool = AtomicBool::new(false);

    #[test]
    fn roundtrip_with_long_paths() {
        let (a, b) = (S::new("a"), S::new("b"));
        let src = fill(&a.0);
        let r = relocate(&g(), &src, &b.0, &NO, |_, _| {}).unwrap();
        assert_eq!(r.files, 2);
        assert_eq!(fs::read(format!("{src}\\a.txt")).unwrap(), b"hello");
        assert!(fsops::meta(&src).unwrap().attrs & fsops::ATTR_REPARSE != 0);
        assert!(fsops::exists(&r.old));
        undo(&g(), &r).unwrap();
        assert!(fsops::meta(&src).unwrap().attrs & fsops::ATTR_REPARSE == 0);
        assert_eq!(fs::read(format!("{src}\\a.txt")).unwrap(), b"hello");
        assert!(!fsops::exists(&r.dest));
    }

    #[test]
    fn finish_frees_old() {
        let (a, b) = (S::new("fa"), S::new("fb"));
        let src = fill(&a.0);
        let r = relocate(&g(), &src, &b.0, &NO, |_, _| {}).unwrap();
        finish(&g(), &r).unwrap();
        assert!(!fsops::exists(&r.old));
        assert_eq!(fs::read(format!("{src}\\a.txt")).unwrap(), b"hello");
        assert!(undo(&g(), &r).is_err());
    }

    #[test]
    fn open_file_rolls_back() {
        use std::os::windows::fs::OpenOptionsExt;
        let (a, b) = (S::new("oa"), S::new("ob"));
        let src = fill(&a.0);
        let h = fs::OpenOptions::new().read(true).share_mode(1).open(format!("{src}\\a.txt")).unwrap();
        let e = relocate(&g(), &src, &b.0, &NO, |_, _| {}).unwrap_err();
        assert!(e.contains("in use"), "{e}");
        drop(h);
        assert!(!fsops::exists(&format!("{}\\data", b.0)));
        assert_eq!(fs::read(format!("{src}\\a.txt")).unwrap(), b"hello");
        assert!(fsops::meta(&src).unwrap().attrs & fsops::ATTR_REPARSE == 0);
    }

    #[test]
    fn cancel_leaves_source_and_no_dest() {
        let (a, b) = (S::new("ca"), S::new("cb"));
        let src = fill(&a.0);
        let stop = AtomicBool::new(true);
        assert!(relocate(&g(), &src, &b.0, &stop, |_, _| {}).is_err());
        assert!(!fsops::exists(&format!("{}\\data", b.0)));
        assert!(fsops::exists(&format!("{src}\\a.txt")));
    }

    #[test]
    fn refusals() {
        let (a, b) = (S::new("ra"), S::new("rb"));
        let src = fill(&a.0);
        assert!(relocate(&g(), r"C:\Windows\Temp", &b.0, &NO, |_, _| {}).is_err());
        assert!(relocate(&g(), &src, r"C:\Windows", &NO, |_, _| {}).is_err());
        assert!(relocate(&g(), &src, &format!("{src}\\sub"), &NO, |_, _| {}).is_err());
        assert!(relocate(&g(), &format!("{src}\\a.txt"), &b.0, &NO, |_, _| {}).is_err());
        // Contains a link
        let t = S::new("rt");
        fsops::create_junction(&format!("{src}\\lnk"), &t.0).unwrap();
        let e = relocate(&g(), &src, &b.0, &NO, |_, _| {}).unwrap_err();
        assert!(e.contains("link"));
        fsops::remove_link(&format!("{src}\\lnk")).unwrap();
        // Occupied destination
        fs::create_dir(format!("{}\\data", b.0)).unwrap();
        assert!(relocate(&g(), &src, &b.0, &NO, |_, _| {}).is_err());
        fs::remove_dir(format!("{}\\data", b.0)).unwrap();
    }
}
