//! Tweak commands (D-14). Lists every catalogued tweak with its live state, applies per-user tweaks
//! in-process and machine-wide ones through the elevated helper, saving the prior state first.
use crate::App;
use oz_tweaks::{Kind, Restart, Tier, Tweak};
use oz_win::tweak::{self as w, RegVal};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tauri::State;

fn catalog() -> &'static [Tweak] {
    static C: OnceLock<Vec<Tweak>> = OnceLock::new();
    // A broken catalog shows no tweaks rather than unchecked ones.
    C.get_or_init(|| oz_tweaks::load(oz_tweaks::CATALOG).unwrap_or_default())
}

#[derive(Serialize, Deserialize, Clone)]
struct Saved {
    /// "none" (value was absent), "dword:N", "sz:TEXT", "scheme:GUID".
    prior: String,
    time: i64,
}

fn store_path(app: &App) -> std::path::PathBuf {
    std::path::Path::new(&app.data_dir).join("tweaks.json")
}
fn load_store(app: &App) -> HashMap<String, Saved> {
    std::fs::read(store_path(app))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save_store(app: &App, s: &HashMap<String, Saved>) -> Result<(), String> {
    let b = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(store_path(app), b).map_err(|e| format!("couldn't save the undo record: {e}"))
}

#[cfg(test)]
fn encode(v: &Option<RegVal>) -> String {
    match v {
        None => "none".into(),
        Some(RegVal::Dword(d)) => format!("dword:{d}"),
        Some(RegVal::Sz(s)) => format!("sz:{s}"),
    }
}
#[cfg(test)]
fn decode(s: &str) -> Option<RegVal> {
    if let Some(d) = s.strip_prefix("dword:") {
        return d.parse().ok().map(RegVal::Dword);
    }
    s.strip_prefix("sz:").map(|x| RegVal::Sz(x.to_string()))
}

fn target(t: &Tweak) -> Option<RegVal> {
    match t.kind {
        Kind::RegDword => t.on.parse().ok().map(RegVal::Dword),
        Kind::RegSz => Some(RegVal::Sz(t.on.clone())),
        _ => None,
    }
}

/// (state, current value shown to the user)
fn state(t: &Tweak) -> (&'static str, String) {
    match t.kind {
        Kind::RegDword | Kind::RegSz => {
            let (h, k, v) = (
                t.hive.as_deref().unwrap_or(""),
                t.key.as_deref().unwrap_or(""),
                t.value.as_deref().unwrap_or(""),
            );
            match w::reg_read(h, k, v) {
                Ok(None) => ("default", "Windows default".into()),
                Ok(Some(x)) => {
                    let shown = match &x {
                        RegVal::Dword(d) => d.to_string(),
                        RegVal::Sz(s) => s.clone(),
                    };
                    (if Some(x) == target(t) { "on" } else { "off" }, shown)
                }
                Err(e) => ("unknown", e),
            }
        }
        Kind::PowerPlan => match w::active_scheme() {
            Ok(g) => (
                if g.eq_ignore_ascii_case(&t.on) {
                    "on"
                } else {
                    "off"
                },
                plan_name(&g),
            ),
            Err(e) => ("unknown", e),
        },
        Kind::PowerSetting => match w::power_value(
            t.subgroup.as_deref().unwrap_or(""),
            t.setting.as_deref().unwrap_or(""),
        ) {
            Ok(v) => (
                if v.to_string() == t.on { "on" } else { "off" },
                format!("{v}%"),
            ),
            Err(e) => ("unknown", e),
        },
    }
}

fn plan_name(g: &str) -> String {
    match g {
        "381b4222-f694-41f0-9685-ff5bb260df2e" => "Balanced".into(),
        "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c" => "High performance".into(),
        "a1841308-3541-4fab-bc81-f71556f20b4a" => "Power saver".into(),
        "e9a42b02-d5df-448d-aa00-03f14749eb61" => "Ultimate performance".into(),
        _ => "Custom plan".into(),
    }
}

#[derive(Serialize)]
pub struct TweakDto {
    id: String,
    name: String,
    explain: String,
    category: String,
    tier: Tier,
    restart: Restart,
    needs_admin: bool,
    evidence: String,
    state: &'static str,
    current: String,
    applied: bool,
    locked: Option<String>,
}

fn lock_reason(t: &Tweak) -> Option<String> {
    (!t.applicable()).then(|| "Explain only".to_string())
}

/// Machine-wide tweaks go to the elevated helper by id; it returns the prior state it saw.
fn via_helper(app: &App, tag_: u8, body: &[u8]) -> Result<String, String> {
    use oz_win::elevate::{read_frame, tag, write_frame};
    if !crate::ensure_helper(app)? {
        return Err("Administrator permission was declined, so nothing changed.".into());
    }
    let mut h = app.helper.lock().map_err(|_| "helper busy")?;
    let p = h.as_mut().ok_or("Helper unavailable")?;
    write_frame(p, tag_, body).map_err(|e| e.to_string())?;
    loop {
        let (t, d) = read_frame(p).map_err(|e| e.to_string())?;
        match t {
            tag::TWEAK_DONE => return Ok(String::from_utf8_lossy(&d).into_owned()),
            tag::ERROR => return Err(String::from_utf8_lossy(&d).into_owned()),
            _ => {}
        }
    }
}

#[tauri::command]
pub fn tweaks_list(app: State<'_, Arc<App>>) -> Vec<TweakDto> {
    let store = load_store(&app);
    catalog()
        .iter()
        .map(|t| {
            let (state, current) = state(t);
            TweakDto {
                id: t.id.clone(),
                name: t.name.clone(),
                explain: t.explain.clone(),
                category: t.category.clone(),
                tier: t.tier,
                restart: t.restart,
                needs_admin: t.needs_admin,
                evidence: t.evidence.clone(),
                state,
                current,
                applied: store.contains_key(&t.id),
                locked: lock_reason(t),
            }
        })
        .collect()
}

fn find(id: &str) -> Result<&'static Tweak, String> {
    catalog()
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| "unknown tweak".to_string())
}

#[tauri::command]
pub async fn tweak_apply(id: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = find(&id)?;
        if let Some(r) = lock_reason(t) {
            return Err(r);
        }
        let mut store = load_store(&app);
        if t.per_user() {
            // Record the prior state before touching anything; never overwrite an earlier record.
            let prior = oz_win::tweak_exec::prior(t)?;
            store.entry(id.clone()).or_insert(Saved {
                prior,
                time: oz_win::now_filetime(),
            });
            save_store(&app, &store)?;
            oz_win::tweak_exec::apply(t, false)
        } else {
            let prior = via_helper(&app, oz_win::elevate::tag::TWEAK_APPLY, id.as_bytes())?;
            store.entry(id.clone()).or_insert(Saved {
                prior,
                time: oz_win::now_filetime(),
            });
            save_store(&app, &store)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn tweak_revert(id: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let t = find(&id)?;
        let mut store = load_store(&app);
        let Some(saved) = store.get(&id).cloned() else {
            return Err("Nothing to undo: Oz didn't change this setting.".into());
        };
        if t.per_user() {
            oz_win::tweak_exec::revert(t, &saved.prior, false)?;
        } else {
            via_helper(
                &app,
                oz_win::elevate::tag::TWEAK_REVERT,
                format!("{id}\n{}", saved.prior).as_bytes(),
            )?;
        }
        store.remove(&id);
        save_store(&app, &store)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prior_round_trip() {
        for v in [
            None,
            Some(RegVal::Dword(0)),
            Some(RegVal::Dword(4294967295)),
            Some(RegVal::Sz("400".into())),
            Some(RegVal::Sz(String::new())),
        ] {
            let e = encode(&v);
            assert_eq!(if e == "none" { None } else { decode(&e) }, v);
        }
    }
    #[test]
    fn every_tweak_reads() {
        // Reading live state must never panic, whatever the machine.
        for t in catalog() {
            let _ = state(t);
        }
        assert!(catalog().len() >= 20);
    }
}
