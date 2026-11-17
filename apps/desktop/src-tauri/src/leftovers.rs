//! Uninstall A to Z (AGENTS.md section 6b). After a program's own uninstaller runs, find what it
//! left behind, say why each item matched, and remove only what the user ticks: files go to the
//! Holding Area through the normal plan executor, registry keys are exported before deletion.
use crate::App;
use oz_win::regtree;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tauri::State;

#[derive(Serialize, Clone, Debug)]
pub struct Leftover {
    pub path: String,
    /// "folder", "shortcut" or "registry".
    pub kind: &'static str,
    /// "Certain", "Likely", "Name only", "Shared" or "Keep".
    pub confidence: &'static str,
    pub reason: String,
    pub bytes: u64,
    pub preselect: bool,
}

/// Lowercase letters and digits only.
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Name variants worth matching: words before the first version-like token, with and without the
/// publisher's first word. "Microsoft Visual Studio Code (User)" -> ["microsoftvisualstudiocode", "visualstudiocode"].
pub fn app_keys(name: &str, publisher: &str) -> Vec<String> {
    let words: Vec<&str> = name
        .split_whitespace()
        .take_while(|w| {
            !w.starts_with(|c: char| {
                c.is_ascii_digit()
                    || c == '('
                    || c == '-'
                    || c == 'v' && w[1..].starts_with(|d: char| d.is_ascii_digit())
            })
        })
        .filter(|w| {
            !matches!(
                w.to_ascii_lowercase().as_str(),
                "x64" | "x86" | "64-bit" | "32-bit"
            )
        })
        .collect();
    let mut out = vec![norm(&words.join(" "))];
    let pub_first = publisher
        .split_whitespace()
        .next()
        .map(norm)
        .unwrap_or_default();
    if words.len() > 1 && norm(words[0]) == pub_first {
        out.push(norm(&words[1..].join(" ")));
    }
    out.retain(|k| k.len() >= 4);
    out.dedup();
    out
}

/// Publisher folder name: drops legal suffixes. "Freebuff, Inc." -> "freebuff".
pub fn publisher_key(publisher: &str) -> String {
    let words: Vec<&str> = publisher
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .filter(|w| {
            !matches!(
                norm(w).as_str(),
                "inc"
                    | "llc"
                    | "ltd"
                    | "limited"
                    | "corp"
                    | "corporation"
                    | "co"
                    | "gmbh"
                    | "team"
                    | "software"
                    | "the"
            )
        })
        .collect();
    norm(&words.join(""))
}

fn dir_bytes(p: &Path, budget: &mut u32) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    let mut n = 0;
    for e in rd.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let Ok(m) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        if m.file_type().is_symlink() {
            continue;
        }
        n += if m.is_dir() {
            dir_bytes(&e.path(), budget)
        } else {
            m.len()
        };
    }
    n
}

fn size(p: &Path) -> u64 {
    let mut budget = 200_000;
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => dir_bytes(p, &mut budget),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

/// Finds leftovers for a program that has just been uninstalled.
pub fn find(
    app: &App,
    name: &str,
    publisher: &str,
    location: &str,
    others: &[oz_win::apps::App],
) -> Vec<Leftover> {
    let keys = app_keys(name, publisher);
    let pkey = publisher_key(publisher);
    let ms = pkey.starts_with("microsoft");
    let shared_pub = !pkey.is_empty() && others.iter().any(|o| publisher_key(&o.publisher) == pkey);
    let guard = app.guard.as_ref().ok();
    let safe = |p: &str| guard.is_some_and(|g| g.check(p).is_ok());
    let mut out: Vec<Leftover> = Vec::new();
    let mut push = |l: Leftover| {
        if !out.iter().any(|x| x.path.eq_ignore_ascii_case(&l.path)) {
            out.push(l);
        }
    };

    // 1. The recorded install folder, if the uninstaller left it.
    let loc = location.trim().trim_matches('"').trim_end_matches('\\');
    if loc.len() > 3
        && Path::new(loc).is_dir()
        && safe(loc)
        && !others
            .iter()
            .any(|o| o.location.trim_end_matches('\\').eq_ignore_ascii_case(loc))
    {
        push(Leftover {
            path: loc.into(),
            kind: "folder",
            confidence: "Certain",
            reason: "The program's recorded install folder".into(),
            bytes: size(Path::new(loc)),
            preselect: true,
        });
    }

    // 2. App data folders.
    let mut bases: Vec<String> = ["APPDATA", "LOCALAPPDATA", "PROGRAMDATA"]
        .iter()
        .filter_map(|k| env(k))
        .collect();
    if let Some(l) = env("LOCALAPPDATA") {
        bases.push(format!(
            "{}\\LocalLow",
            l.trim_end_matches("\\Local").trim_end_matches("Local")
        ));
    }
    for base in &bases {
        let Ok(rd) = std::fs::read_dir(base) else {
            continue;
        };
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let fname = e.file_name().to_string_lossy().to_string();
            let n = norm(&fname);
            let path = e.path().to_string_lossy().to_string();
            if keys.contains(&n) && safe(&path) {
                push(Leftover {
                    path: path.clone(),
                    kind: "folder",
                    confidence: "Name only",
                    reason: format!("Folder named like the program in {}", short_base(base)),
                    bytes: size(&e.path()),
                    preselect: false,
                });
            } else if !ms && !pkey.is_empty() && n == pkey && safe(&path) {
                // Publisher folder: look inside for the app, else offer the whole folder only if no other app shares it.
                let mut found = false;
                for c in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
                    let cn = norm(&c.file_name().to_string_lossy());
                    let cp = c.path().to_string_lossy().to_string();
                    if c.path().is_dir() && keys.contains(&cn) && safe(&cp) {
                        found = true;
                        push(Leftover {
                            path: cp,
                            kind: "folder",
                            confidence: "Likely",
                            reason: "Publisher and program name match".into(),
                            bytes: size(&c.path()),
                            preselect: true,
                        });
                    }
                }
                if !found {
                    push(Leftover {
                        path: path.clone(),
                        kind: "folder",
                        confidence: if shared_pub { "Shared" } else { "Likely" },
                        reason: if shared_pub {
                            format!(
                                "Publisher folder; another {publisher} program is still installed"
                            )
                        } else {
                            "Publisher folder; no other program from this publisher is installed"
                                .into()
                        },
                        bytes: size(&e.path()),
                        preselect: !shared_pub,
                    });
                }
            }
        }
    }
    // Upgrade a name-only match that sits right under a publisher match: keep the stronger one.
    // 3. Shortcuts in Start menu and on the desktop.
    let starts = [
        env("APPDATA").map(|a| format!(r"{a}\Microsoft\Windows\Start Menu\Programs")),
        env("PROGRAMDATA").map(|a| format!(r"{a}\Microsoft\Windows\Start Menu\Programs")),
        env("USERPROFILE").map(|u| format!(r"{u}\Desktop")),
        env("PUBLIC").map(|u| format!(r"{u}\Desktop")),
    ];
    for base in starts.into_iter().flatten() {
        for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            let stem = fname.strip_suffix(".lnk").unwrap_or(&fname);
            let p = e.path().to_string_lossy().to_string();
            if keys.contains(&norm(stem)) && safe(&p) {
                push(Leftover {
                    path: p,
                    kind: "shortcut",
                    confidence: "Likely",
                    reason: "Shortcut named like the program".into(),
                    bytes: size(&e.path()),
                    preselect: true,
                });
            }
        }
    }
    // 4. Documents: the user's own files. Shown, never preselected.
    if let Some(u) = env("USERPROFILE") {
        let docs = format!(r"{u}\Documents");
        for e in std::fs::read_dir(&docs).into_iter().flatten().flatten() {
            if e.path().is_dir() && keys.contains(&norm(&e.file_name().to_string_lossy())) {
                push(Leftover {
                    path: e.path().to_string_lossy().to_string(),
                    kind: "folder",
                    confidence: "Keep",
                    reason: "In Documents: likely your own saves, projects or exports".into(),
                    bytes: size(&e.path()),
                    preselect: false,
                });
            }
        }
    }
    // 5. Per-user registry keys under Software.
    for top in regtree::subkeys("Software") {
        let n = norm(&top);
        let path = format!(r"Software\{top}");
        if keys.contains(&n) {
            push(Leftover {
                path: format!(r"HKCU\{path}"),
                kind: "registry",
                confidence: "Name only",
                reason: "Settings key named like the program".into(),
                bytes: 0,
                preselect: false,
            });
        } else if !ms && !pkey.is_empty() && n == pkey {
            for sub in regtree::subkeys(&path) {
                if keys.contains(&norm(&sub)) {
                    push(Leftover {
                        path: format!(r"HKCU\{path}\{sub}"),
                        kind: "registry",
                        confidence: "Likely",
                        reason: "Publisher and program name match".into(),
                        bytes: 0,
                        preselect: true,
                    });
                }
            }
        }
    }
    out.sort_by_key(|l| (rank(l.confidence), std::cmp::Reverse(l.bytes)));
    out
}

fn rank(c: &str) -> u8 {
    match c {
        "Certain" => 0,
        "Likely" => 1,
        "Name only" => 2,
        "Shared" => 3,
        _ => 4,
    }
}

fn short_base(b: &str) -> &'static str {
    let l = b.to_ascii_lowercase();
    if l.ends_with("roaming") {
        "AppData\\Roaming"
    } else if l.ends_with("locallow") {
        "AppData\\LocalLow"
    } else if l.ends_with("local") {
        "AppData\\Local"
    } else {
        "ProgramData"
    }
}

#[tauri::command]
pub fn remove_registry_leftovers(
    keys: Vec<String>,
    app: State<'_, Arc<App>>,
) -> Result<String, String> {
    // Export everything first; delete nothing if any export fails.
    let mut lines = Vec::new();
    let mut paths = Vec::new();
    for k in &keys {
        let rel = k
            .strip_prefix(r"HKCU\")
            .ok_or("only per-user keys can be removed")?;
        if let Some(t) = regtree::export(rel)? {
            lines.push(format!("R\t{rel}"));
            regtree::to_lines(&t, rel, &mut lines);
            paths.push(rel.to_string());
        }
    }
    if paths.is_empty() {
        return Ok(String::new());
    }
    let dir = format!(r"{}\registry-backups", app.data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = format!(r"{dir}\{}.txt", oz_win::now_filetime());
    std::fs::write(&file, lines.join("\n"))
        .map_err(|e| format!("couldn't save the backup, nothing removed: {e}"))?;
    for p in &paths {
        regtree::delete(p)?;
    }
    crate::append_action(
        &app,
        "uninstall",
        &paths.join(", "),
        "Removed registry leftovers",
        &format!("backup {file}"),
    );
    Ok(file)
}

#[tauri::command]
pub fn restore_registry_backup(file: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    let dir = format!(r"{}\registry-backups", app.data_dir);
    if !file.starts_with(&dir) || file.contains("..") {
        return Err("not an Oz backup".into());
    }
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for l in text.lines() {
        if let Some(r) = l.strip_prefix("R\t") {
            groups.push((r.to_string(), Vec::new()));
        } else if let Some(g) = groups.last_mut() {
            g.1.push(l.to_string());
        }
    }
    for (root, lines) in &groups {
        let t = regtree::from_lines(lines, root).ok_or("backup file is damaged")?;
        regtree::restore(root, &t)?;
    }
    crate::append_action(&app, "uninstall", &file, "Restored registry leftovers", "");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert_eq!(app_keys("ZCode 3.17.2", "ZCode"), vec!["zcode"]);
        assert_eq!(
            app_keys(
                "Microsoft Visual Studio Code (User)",
                "Microsoft Corporation"
            ),
            vec!["microsoftvisualstudiocode", "visualstudiocode"]
        );
        assert_eq!(app_keys("Hydra", "Los Broxas"), vec!["hydra"]);
        assert!(
            app_keys("Go 1.22", "Google").is_empty(),
            "too short to match safely"
        );
        assert_eq!(publisher_key("Freebuff, Inc."), "freebuff");
        assert_eq!(publisher_key("Moonsworth LLC"), "moonsworth");
        assert_eq!(publisher_key("NordPass Team"), "nordpass");
    }
}
