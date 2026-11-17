//! Executes a reviewed plan. Pre-flight runs per item right before it is touched
//! (F-37). A safer method never silently falls back to a less safe one (F-40).
use crate::guard::Guard;
use crate::ledger;
use oz_win::fsops::{self, Meta};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Permanent,
    Recycle,
    Hold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub path: String,
    pub method: Method,
    /// Delete what is inside the folder, leaving the folder.
    #[serde(default)]
    pub contents_only: bool,
    /// With contents_only: skip entries modified more recently than this.
    #[serde(default)]
    pub min_age_days: u32,
    /// File modified time from the scan. If it changed, the item is skipped.
    #[serde(default)]
    pub expect_modified: Option<i64>,
    #[serde(default)]
    pub estimated: u64,
    #[serde(default)]
    pub rule: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Done,
    Partial,
    Held,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemResult {
    pub path: String,
    pub method: Method,
    pub outcome: Outcome,
    pub reason: String,
    /// Logical bytes removed or moved, as far as is known.
    pub bytes: u64,
    /// Some items failed with access denied; offering elevation may help.
    pub denied: bool,
    #[serde(default)]
    pub held_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub results: Vec<ItemResult>,
    pub cancelled: bool,
}

const MAX_RECYCLE_PATH: usize = 259;
/// FILETIMEs exceed the range a JSON number holds exactly: the UI round trip can
/// move one by up to about 32 units. This allows for that rounding (6.4 microseconds)
/// and nothing more, so real modifications are still detected.
const TIME_TOLERANCE: i64 = 64;

/// Items the Recycle Bin cannot take; shown to the user before any change (F-40).
pub fn cannot_recycle(item: &Item) -> Option<&'static str> {
    (item.method == Method::Recycle && item.path.encode_utf16().count() > MAX_RECYCLE_PATH)
        .then_some("Path is too long for the Recycle Bin")
}

pub fn reason_text(code: u32) -> String {
    match code {
        fsops::ERR_ACCESS_DENIED => "Access denied".into(),
        fsops::ERR_SHARING | fsops::ERR_LOCK => "In use".into(),
        2 | 3 => "No longer there".into(),
        fsops::ERROR_CLOUD_FILE_PLACEHOLDER => "Cloud placeholder, left alone".into(),
        c => format!("Windows error {c}"),
    }
}

fn skip(i: &Item, reason: impl Into<String>) -> ItemResult {
    ItemResult { path: i.path.clone(), method: i.method, outcome: Outcome::Skipped, reason: reason.into(), bytes: 0, denied: false, held_at: None }
}

/// Pre-flight re-check. Returns the fresh metadata or why the item was skipped.
pub fn preflight(g: &Guard, i: &Item) -> Result<Meta, String> {
    g.check(&i.path).map_err(|r| format!("Refused: {}", r.0))?;
    let m = fsops::meta(&i.path).map_err(reason_text)?;
    if m.attrs & fsops::ATTR_REPARSE != 0 {
        return Err("Is a link or special file, left alone".into());
    }
    if m.attrs & fsops::ATTR_RECALL != 0 {
        return Err("Cloud placeholder, left alone".into());
    }
    if let Some(t) = i.expect_modified {
        if m.attrs & fsops::ATTR_DIR == 0 && (m.modified - t).abs() > TIME_TOLERANCE {
            return Err("Changed since the scan".into());
        }
    }
    if i.contents_only && m.attrs & fsops::ATTR_DIR == 0 {
        return Err("Expected a folder".into());
    }
    Ok(m)
}

pub fn hold_root(path: &str, op: &str) -> Option<String> {
    let p = path.strip_prefix(r"\\?\").unwrap_or(path);
    let b = p.as_bytes();
    if b.len() < 3 || b[1] != b':' {
        return None;
    }
    let clean: String = op.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    Some(format!("{}:\\OzHold\\{clean}", b[0] as char))
}

/// Runs a plan. `ledger_dir` receives an append-only record before and after.
pub fn execute(plan: &Plan, g: &Guard, ledger_dir: Option<&str>, stop: &AtomicBool, mut on_result: impl FnMut(&ItemResult)) -> Report {
    let mut results = Vec::new();
    let mut hold_seq = 0u32;
    let mut cancelled = false;
    for item in &plan.items {
        if stop.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let r = run_item(plan, item, g, stop, &mut hold_seq);
        if let Some(d) = ledger_dir {
            ledger::append(d, &plan.id, &r);
        }
        on_result(&r);
        results.push(r);
    }
    Report { id: plan.id.clone(), results, cancelled }
}

fn run_item(plan: &Plan, item: &Item, g: &Guard, stop: &AtomicBool, seq: &mut u32) -> ItemResult {
    let meta = match preflight(g, item) {
        Ok(m) => m,
        Err(e) => return skip(item, e),
    };
    if let Some(why) = cannot_recycle(item) {
        return skip(item, why);
    }
    match item.method {
        Method::Permanent => permanent(item, g, stop, meta),
        Method::Recycle => {
            if item.contents_only {
                return skip(item, "Recycle applies to whole items only");
            }
            match fsops::recycle(&item.path) {
                Ok(()) if !fsops::exists(&item.path) => done(item, meta.size, Outcome::Done),
                Ok(()) => skip(item, "Windows did not recycle it"),
                Err(e) => {
                    let mut r = skip(item, format!("Could not recycle ({e}); nothing was deleted"));
                    r.denied = e == fsops::ERR_ACCESS_DENIED;
                    r
                }
            }
        }
        Method::Hold => hold(plan, item, seq, meta),
    }
}

fn done(i: &Item, bytes: u64, o: Outcome) -> ItemResult {
    ItemResult { path: i.path.clone(), method: i.method, outcome: o, reason: String::new(), bytes, denied: false, held_at: None }
}

fn permanent(item: &Item, g: &Guard, stop: &AtomicBool, m: Meta) -> ItemResult {
    let halted = || stop.load(Ordering::Relaxed);
    let mut bytes = 0u64;
    let mut fails = Vec::new();
    let mut young = 0u32;
    if item.contents_only {
        let names = match fsops::list_dir(&item.path) {
            Ok(n) => n,
            Err(e) => return skip(item, reason_text(e)),
        };
        let cutoff = oz_win::now_filetime() - item.min_age_days as i64 * 864_000_000_000;
        for n in names {
            let child = format!("{}\\{n}", item.path.trim_end_matches('\\'));
            if g.check(&child).is_err() {
                continue;
            }
            if item.min_age_days > 0 {
                if newest(&child, cutoff) {
                    young += 1;
                    continue;
                }
            }
            fsops::remove_tree_report(&child, &halted, &mut |b| bytes += b, &mut fails);
        }
    } else {
        let _ = m;
        fsops::remove_tree_report(&item.path, &halted, &mut |b| bytes += b, &mut fails);
    }
    let denied = fails.iter().any(|f| f.1 == fsops::ERR_ACCESS_DENIED);
    let mut r = done(item, bytes, if fails.is_empty() { Outcome::Done } else { Outcome::Partial });
    r.denied = denied;
    let mut notes = Vec::new();
    if !fails.is_empty() {
        let in_use = fails.iter().filter(|f| matches!(f.1, fsops::ERR_SHARING | fsops::ERR_LOCK)).count();
        let den = fails.iter().filter(|f| f.1 == fsops::ERR_ACCESS_DENIED).count();
        if in_use > 0 {
            notes.push(format!("{in_use} in use"));
        }
        if den > 0 {
            notes.push(format!("{den} access denied"));
        }
        let other = fails.len() - in_use - den;
        if other > 0 {
            notes.push(format!("{other} other: {}", reason_text(fails.iter().find(|f| !matches!(f.1, 5 | 32 | 33)).map(|f| f.1).unwrap_or(0))));
        }
    }
    if young > 0 {
        notes.push(format!("{young} too recent"));
    }
    r.reason = notes.join(", ");
    r
}

/// True if the item or anything beneath it was modified after `cutoff`.
/// Unreadable items count as recent, which keeps them.
fn newest(path: &str, cutoff: i64) -> bool {
    let Ok(m) = fsops::meta(path) else { return true };
    if m.modified > cutoff {
        return true;
    }
    if m.attrs & fsops::ATTR_DIR == 0 || m.attrs & fsops::ATTR_REPARSE != 0 {
        return false;
    }
    let Ok(names) = fsops::list_dir(path) else { return true };
    names.iter().any(|n| newest(&format!("{}\\{n}", path.trim_end_matches('\\')), cutoff))
}

fn hold(plan: &Plan, item: &Item, seq: &mut u32, m: Meta) -> ItemResult {
    if item.contents_only {
        return skip(item, "Holding Area applies to whole items only");
    }
    let Some(root) = hold_root(&item.path, &plan.id) else { return skip(item, "Not a local path") };
    if let Err(e) = fsops::create_dirs(&root) {
        return skip(item, format!("Could not create Holding Area: {}", reason_text(e)));
    }
    *seq += 1;
    let dest = format!("{root}\\{:06}", *seq);
    // Manifest entry first: an interrupted move can still be traced.
    let line = serde_json::json!({"from": item.path, "to": dest, "bytes": item.estimated}).to_string();
    if ledger::append_line(&format!("{root}\\manifest.jsonl"), &line).is_err() {
        return skip(item, "Could not write Holding Area manifest");
    }
    match fsops::rename(&item.path, &dest) {
        Ok(()) => {
            let mut r = done(item, item.estimated.max(m.size), Outcome::Held);
            r.held_at = Some(dest);
            r
        }
        Err(e) => {
            let mut r = skip(item, reason_text(e));
            r.denied = e == fsops::ERR_ACCESS_DENIED;
            r
        }
    }
}

/// Moves a held item back. Refuses if something now occupies the original place.
pub fn restore(from_hold: &str, original: &str) -> Result<(), String> {
    if !from_hold.contains("\\OzHold\\") {
        return Err("Not in a Holding Area".into());
    }
    if fsops::exists(original) {
        return Err("Something else is now at the original location".into());
    }
    if let Some((parent, _)) = original.rsplit_once('\\') {
        fsops::create_dirs(parent).map_err(reason_text)?;
    }
    fsops::rename(from_hold, original).map_err(reason_text)
}

/// Permanently deletes one Holding Area operation folder.
pub fn purge_hold(root: &str) -> Result<(), String> {
    let p = root.trim_end_matches('\\');
    let b = p.as_bytes();
    let shape = b.len() > 11 && b[1] == b':' && p[2..].starts_with("\\OzHold\\") && !p[10..].contains('\\') && !p.contains("..");
    if !shape {
        return Err("Not a Holding Area operation folder".into());
    }
    fsops::remove_tree(p, &|| false).map_err(reason_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(String);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("oz-plan-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p.to_str().unwrap().to_string())
        }
        fn p(&self, n: &str) -> String {
            format!("{}\\{n}", self.0)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fsops::remove_tree(&self.0, &|| false);
        }
    }

    /// A guard that treats the temp folder as ordinary user space.
    fn guard() -> Guard {
        Guard::new(r"C:\Windows", r"C:\NoSuchUsers\me", &[r"C:\Program Files"], r"C:\ProgramData")
    }

    fn item(path: &str, method: Method) -> Item {
        Item { path: path.into(), method, contents_only: false, min_age_days: 0, expect_modified: None, estimated: 0, rule: None }
    }

    fn run(items: Vec<Item>) -> Report {
        execute(&Plan { id: "t".into(), items }, &guard(), None, &AtomicBool::new(false), |_| {})
    }

    #[test]
    fn refuses_protected_in_execute() {
        let r = run(vec![item(r"C:\Windows\System32\drivers\etc\hosts", Method::Permanent)]);
        assert_eq!(r.results[0].outcome, Outcome::Skipped);
        assert!(r.results[0].reason.starts_with("Refused"));
        assert!(fsops::exists(r"C:\Windows\System32\drivers\etc\hosts"));
    }

    #[test]
    fn reparse_item_is_refused_and_target_kept() {
        let s = Scratch::new("rp");
        let t = Scratch::new("rp-t");
        fs::write(t.p("keep"), b"k").unwrap();
        fsops::create_junction(&s.p("link"), &t.0).unwrap();
        let r = run(vec![item(&s.p("link"), Method::Permanent)]);
        assert_eq!(r.results[0].outcome, Outcome::Skipped);
        assert!(fsops::exists(&t.p("keep")));
        fsops::remove_link(&s.p("link")).unwrap();
    }

    #[test]
    fn nested_junction_survives_folder_delete() {
        let s = Scratch::new("nj");
        let t = Scratch::new("nj-t");
        fs::write(t.p("keep"), b"k").unwrap();
        fs::create_dir(s.p("d")).unwrap();
        fsops::create_junction(&s.p("d\\link"), &t.0).unwrap();
        let r = run(vec![item(&s.p("d"), Method::Permanent)]);
        assert_eq!(r.results[0].outcome, Outcome::Done);
        assert!(!fsops::exists(&s.p("d")));
        assert!(fsops::exists(&t.p("keep")));
    }

    #[test]
    fn changed_file_is_skipped() {
        let s = Scratch::new("ch");
        fs::write(s.p("f"), b"1").unwrap();
        let mut i = item(&s.p("f"), Method::Permanent);
        i.expect_modified = Some(fsops::meta(&s.p("f")).unwrap().modified - 10_000);
        let r = run(vec![i]);
        assert_eq!(r.results[0].reason, "Changed since the scan");
        assert!(fsops::exists(&s.p("f")));
    }

    #[test]
    fn timestamp_survives_json_number_round_trip() {
        let s = Scratch::new("js");
        fs::write(s.p("f"), b"1").unwrap();
        let t = fsops::meta(&s.p("f")).unwrap().modified;
        let mut i = item(&s.p("f"), Method::Permanent);
        // What JavaScript does to the value.
        i.expect_modified = Some((t as f64) as i64);
        assert!(preflight(&guard(), &i).is_ok());
        i.expect_modified = Some(t - 10_000_000); // one second earlier: a real change
        assert_eq!(preflight(&guard(), &i).unwrap_err(), "Changed since the scan");
    }

    #[test]
    fn missing_item_reported() {
        let s = Scratch::new("ms");
        let r = run(vec![item(&s.p("gone"), Method::Permanent)]);
        assert_eq!(r.results[0].reason, "No longer there");
    }

    #[test]
    fn long_path_recycle_refused_with_no_fallback() {
        let s = Scratch::new("lr");
        let mut d = s.0.clone();
        for _ in 0..30 {
            d.push_str("\\0123456789");
        }
        fsops::create_dirs(&d).unwrap();
        let f = format!("{d}\\f.txt");
        fs::write(oz_win::long_path(&f), b"x").unwrap();
        let r = run(vec![item(&f, Method::Recycle)]);
        assert_eq!(r.results[0].outcome, Outcome::Skipped);
        assert!(fsops::exists(&f), "must not fall back to deleting");
    }

    #[test]
    fn long_path_permanent_works() {
        let s = Scratch::new("lp");
        let mut d = s.p("top");
        for _ in 0..30 {
            d.push_str("\\0123456789");
        }
        fsops::create_dirs(&d).unwrap();
        fs::write(oz_win::long_path(&format!("{d}\\f.txt")), vec![0u8; 10]).unwrap();
        let r = run(vec![item(&s.p("top"), Method::Permanent)]);
        assert_eq!(r.results[0].outcome, Outcome::Done);
        assert_eq!(r.results[0].bytes, 10);
        assert!(!fsops::exists(&s.p("top")));
    }

    #[test]
    fn contents_only_respects_age_and_keeps_folder() {
        let s = Scratch::new("age");
        fs::write(s.p("fresh"), b"1").unwrap();
        let mut i = item(&s.0, Method::Permanent);
        i.contents_only = true;
        i.min_age_days = 7;
        let r = run(vec![i.clone()]);
        assert!(fsops::exists(&s.p("fresh")));
        assert!(r.results[0].reason.contains("1 too recent"));
        i.min_age_days = 0;
        run(vec![i]);
        assert!(!fsops::exists(&s.p("fresh")));
        assert!(fsops::exists(&s.0));
    }

    #[test]
    fn locked_file_is_partial() {
        use std::os::windows::fs::OpenOptionsExt;
        let s = Scratch::new("lk");
        fs::write(s.p("free"), b"1").unwrap();
        let h = fs::OpenOptions::new().create(true).truncate(true).write(true).share_mode(0).open(s.p("lock")).unwrap();
        let mut i = item(&s.0, Method::Permanent);
        i.contents_only = true;
        let r = run(vec![i]);
        assert_eq!(r.results[0].outcome, Outcome::Partial);
        assert!(r.results[0].reason.contains("1 in use"));
        assert!(!fsops::exists(&s.p("free")));
        drop(h);
    }

    #[test]
    fn stop_flag_stops() {
        let s = Scratch::new("st");
        fs::write(s.p("f"), b"1").unwrap();
        let stop = AtomicBool::new(true);
        let r = execute(&Plan { id: "t".into(), items: vec![item(&s.p("f"), Method::Permanent)] }, &guard(), None, &stop, |_| {});
        assert!(r.cancelled);
        assert!(fsops::exists(&s.p("f")));
    }

    #[test]
    fn hold_root_and_purge_shape() {
        assert_eq!(hold_root(r"E:\a\b", "op-1").unwrap(), r"E:\OzHold\op-1");
        assert_eq!(hold_root(r"\\?\E:\a", "o/../p").unwrap(), r"E:\OzHold\op");
        for bad in [r"E:\OzHold", r"E:\OzHold\", r"E:\Other\x", r"E:\OzHold\a\b", r"E:\OzHold\..\Windows"] {
            assert!(purge_hold(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn restore_refuses_occupied_and_non_hold() {
        let s = Scratch::new("rs");
        fs::write(s.p("f"), b"1").unwrap();
        assert!(restore(&s.p("x"), &s.p("y")).is_err());
        assert!(restore(r"E:\OzHold\o\000001", &s.p("f")).is_err());
    }

    #[test]
    fn hold_moves_then_restore_roundtrip() {
        // Uses a scratch-local rename to prove Holding Area semantics without
        // writing to a real volume root: restore() on the moved path.
        let s = Scratch::new("hr");
        let hold_dir = s.p("OzHold\\op");
        fsops::create_dirs(&hold_dir).unwrap();
        fs::write(s.p("data"), b"hello").unwrap();
        let held = format!("{hold_dir}\\000001");
        fsops::rename(&s.p("data"), &held).unwrap();
        assert!(!fsops::exists(&s.p("data")));
        restore(&held, &s.p("data")).unwrap();
        assert_eq!(fs::read(s.p("data")).unwrap(), b"hello");
    }
}
