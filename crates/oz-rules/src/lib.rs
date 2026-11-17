//! Rule catalog loader and matcher (D-12, A-9). Pure: environment values are passed in.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    Rebuildable,
    Caution,
    Review,
    Protected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    DeleteContents,
    RunTool,
    OpenSettings,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub explain: String,
    #[serde(default)]
    pub after: String,
    pub tier: Tier,
    pub category: String,
    pub paths: Vec<String>,
    pub action: Action,
    #[serde(default)]
    pub min_age_days: u32,
    #[serde(default)]
    pub needs_admin: bool,
    #[serde(default)]
    pub settings_uri: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    pub verified: String,
}

#[derive(Deserialize)]
struct Catalog {
    rule: Vec<Rule>,
}

pub const CATALOG: &str = include_str!("../catalog.toml");
pub const ALLOWED_URIS: [&str; 1] = ["ms-settings:storagesense"];

pub fn load(text: &str) -> Result<Vec<Rule>, String> {
    let c: Catalog = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut ids = std::collections::HashSet::new();
    for r in &c.rule {
        if !ids.insert(r.id.clone()) {
            return Err(format!("duplicate rule {}", r.id));
        }
        if r.verified.trim().is_empty() {
            return Err(format!("rule {} has no verification record", r.id));
        }
        if r.paths.is_empty() {
            return Err(format!("rule {} has no paths", r.id));
        }
        if r.tier == Tier::Protected && r.action == Action::DeleteContents {
            return Err(format!("protected rule {} may not delete", r.id));
        }
        if r.tier != Tier::Protected && r.after.is_empty() {
            return Err(format!("rule {} must say what happens afterwards", r.id));
        }
        if let Some(u) = &r.settings_uri {
            if !ALLOWED_URIS.contains(&u.as_str()) {
                return Err(format!("rule {} uses a URI outside the allow list", r.id));
            }
        }
    }
    Ok(c.rule)
}

/// Expands `%VAR%` tokens. None if a variable is unknown.
pub fn expand(pattern: &str, env: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        let b = after.find('%')?;
        out.push_str(env.get(&after[..b].to_ascii_uppercase())?);
        rest = &after[b + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn segments(p: &str) -> Vec<String> {
    p.trim_end_matches('\\')
        .split('\\')
        .map(|s| s.to_lowercase())
        .collect()
}

/// True if `path` equals the pattern or lies beneath it. `*` matches one segment.
pub fn matches(pattern: &str, path: &str) -> bool {
    let (pa, pt) = (segments(pattern), segments(path));
    pt.len() >= pa.len() && pa.iter().zip(&pt).all(|(a, b)| a == "*" || a == b)
}

pub struct Matcher {
    pub rules: Vec<Rule>,
    expanded: Vec<(usize, String)>,
}

impl Matcher {
    pub fn new(rules: Vec<Rule>, env: &HashMap<String, String>) -> Self {
        let mut expanded = Vec::new();
        for (i, r) in rules.iter().enumerate() {
            for p in &r.paths {
                if let Some(e) = expand(p, env) {
                    expanded.push((i, e));
                }
            }
        }
        // Most specific first.
        expanded.sort_by_key(|(_, p)| std::cmp::Reverse(p.len()));
        Self { rules, expanded }
    }

    pub fn find(&self, path: &str) -> Option<&Rule> {
        self.expanded
            .iter()
            .find(|(_, p)| matches(p, path))
            .map(|(i, _)| &self.rules[*i])
    }

    /// Expanded, wildcard-free roots of a rule.
    pub fn roots(&self, rule_id: &str) -> Vec<String> {
        self.expanded
            .iter()
            .filter(|(i, p)| self.rules[*i].id == rule_id && !p.contains('*'))
            .map(|(_, p)| p.clone())
            .collect()
    }
}

/// Suggests a likely owner from a folder name. Always shown as a guess (F-31).
pub fn guess_owner(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    for marker in [
        "\\appdata\\local\\",
        "\\appdata\\roaming\\",
        "\\appdata\\locallow\\",
        "\\program files\\",
        "\\program files (x86)\\",
        "\\programdata\\",
    ] {
        if let Some(i) = lower.find(marker) {
            let rest = &path[i + marker.len()..];
            let seg = rest.split('\\').next().unwrap_or("");
            if !seg.is_empty() {
                return Some(seg.to_string());
            }
        }
    }
    None
}

/// Project-marker check for Review candidates such as `node_modules` (F-33).
pub fn dev_folder_marker(name: &str) -> Option<&'static [&'static str]> {
    match name.to_ascii_lowercase().as_str() {
        "node_modules" => Some(&["package.json"]),
        ".venv" | "venv" => Some(&["pyvenv.cfg"]),
        "target" => Some(&["Cargo.toml"]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HashMap<String, String> {
        [
            ("LOCALAPPDATA", r"C:\Users\z\AppData\Local"),
            ("WINDIR", r"C:\Windows"),
            ("SYSTEMDRIVE", "C:"),
            ("PROGRAMDATA", r"C:\ProgramData"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    #[test]
    fn real_catalog_loads() {
        let r = load(CATALOG).unwrap();
        assert!(r.len() > 10);
        for rule in r.iter().filter(|r| r.action == Action::DeleteContents) {
            assert!(rule.tier != Tier::Protected);
            assert!(rule.verified.contains("2026"), "{}", rule.id);
        }
    }

    #[test]
    fn rejects_unverified_and_bad_protected() {
        let base =
            "[[rule]]\nid='a'\nname='a'\nexplain='a'\nafter='x'\ncategory='c'\npaths=['C:\\\\x']\n";
        assert!(load(&format!(
            "{base}tier='Caution'\naction='delete_contents'\nverified=''"
        ))
        .is_err());
        assert!(load(&format!(
            "{base}tier='Protected'\naction='delete_contents'\nverified='y'"
        ))
        .is_err());
        assert!(load(&format!(
            "{base}tier='Caution'\naction='delete_contents'\nverified='y'"
        ))
        .is_ok());
    }

    #[test]
    fn matching() {
        let m = Matcher::new(load(CATALOG).unwrap(), &env());
        assert_eq!(
            m.find(r"C:\Users\z\AppData\Local\Temp\x\y.tmp").unwrap().id,
            "user-temp"
        );
        assert_eq!(m.find(r"c:\windows\winsxs\foo").unwrap().id, "winsxs");
        assert!(m.find(r"C:\Users\z\Documents\report.docx").is_none());
        assert!(m.find(r"C:\Users\z\AppData\Local\Temporary").is_none());
        assert_eq!(
            m.find(r"C:\Users\z\AppData\Local\Packages\Canonical\LocalState\ext4.vhdx")
                .unwrap()
                .id,
            "wsl-docker"
        );
        let npm = m.roots("npm-cache");
        assert!(npm.contains(&r"C:\Users\z\AppData\Local\npm-cache".to_string()));
        assert!(npm.contains(
            &r"C:\Users\z\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\npm-cache"
                .to_string()
        ));
        assert_eq!(m.find(r"C:\Users\z\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\npm-cache\_cacache\x").unwrap().id, "npm-cache");
        // Claude Code's own history and config are never matched by a cleaning rule.
        assert!(m.find(r"C:\Users\z\.claude\projects\x.jsonl").is_none());
    }

    #[test]
    fn misc() {
        assert_eq!(
            guess_owner(r"C:\Users\z\AppData\Local\Slack\x").as_deref(),
            Some("Slack")
        );
        assert!(expand("%NOPE%\\x", &env()).is_none());
        assert!(dev_folder_marker("node_modules").is_some());
    }
}
