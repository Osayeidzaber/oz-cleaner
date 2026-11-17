//! Append-only record of operations (F-39), one JSON object per line.
use crate::plan::ItemResult;
use std::io::Write;

pub fn append_line(file: &str, line: &str) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(oz_win::long_path(file))?;
    writeln!(f, "{line}")
}

/// A failed ledger write must not abort the operation already under way.
pub fn append(dir: &str, op: &str, r: &ItemResult) {
    let line = serde_json::json!({
        "time": oz_win::now_filetime(), "op": op, "path": r.path, "method": r.method,
        "outcome": r.outcome, "reason": r.reason, "bytes": r.bytes, "held_at": r.held_at,
    });
    let _ = std::fs::create_dir_all(dir);
    let _ = append_line(&format!("{dir}\\ledger.jsonl"), &line.to_string());
}

pub fn read(dir: &str) -> Vec<serde_json::Value> {
    let Ok(t) = std::fs::read_to_string(format!("{dir}\\ledger.jsonl")) else { return Vec::new() };
    t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Method, Outcome};

    #[test]
    fn appends_and_reads() {
        let d = std::env::temp_dir().join(format!("oz-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let d = d.to_str().unwrap().to_string();
        let r = ItemResult { path: "a".into(), method: Method::Hold, outcome: Outcome::Held, reason: "".into(), bytes: 5, denied: false, held_at: Some("h".into()) };
        append(&d, "o1", &r);
        append(&d, "o2", &r);
        let v = read(&d);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1]["op"], "o2");
        assert_eq!(v[0]["held_at"], "h");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
