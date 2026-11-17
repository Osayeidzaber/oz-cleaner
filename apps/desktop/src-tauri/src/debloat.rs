//! Remove built-in apps (D-14). Only catalogued, removable Microsoft Store apps, for the current
//! user. Each removal is logged; "Get it back" opens the app's Store page.
use crate::App;
use oz_tweaks::bloat::{self, safe_package_name};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct BloatApp {
    family: String,
    name: String,
    explain: String,
    recommended: bool,
    installed: bool,
    removable: bool,
}

#[tauri::command]
pub async fn bloat_list() -> Result<Vec<BloatApp>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let pkgs = oz_win::appx::list()?;
        Ok(bloat::BLOAT
            .iter()
            .map(|b| {
                let p = pkgs
                    .iter()
                    .find(|p| format!("{}_", p.family.split('_').next().unwrap_or("")) == b.family);
                BloatApp {
                    family: p
                        .map(|p| p.family.clone())
                        .unwrap_or_else(|| b.family.trim_end_matches('_').to_string()),
                    name: b.name.into(),
                    explain: b.explain.into(),
                    recommended: b.recommended,
                    installed: p.is_some(),
                    removable: p.is_some_and(|p| !p.non_removable),
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn bloat_remove(family: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let b = bloat::find(&format!("{family}_"))
            .or_else(|| bloat::find(&family))
            .ok_or("not an app Oz offers to remove")?;
        // Re-read the package list; the UI only names the family.
        let pkgs = oz_win::appx::list()?;
        let p = pkgs
            .iter()
            .find(|p| p.family == family)
            .ok_or("that app isn't installed")?;
        if p.non_removable {
            return Err("Windows marks this app as non-removable, so Oz leaves it.".into());
        }
        if !safe_package_name(&p.full_name) {
            return Err("unexpected package name; nothing removed".into());
        }
        oz_win::appx::remove(&p.full_name)?;
        crate::append_action(
            &app,
            "debloat",
            b.name,
            "Removed built-in app",
            &p.full_name,
        );
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_store(family: String) -> Result<(), String> {
    if bloat::find(&format!("{family}_")).is_none() || !safe_package_name(&family) {
        return Err("not an app Oz offers".into());
    }
    oz_win::fsops::open_uri(&format!("ms-windows-store://pdp/?PFN={family}"))
}
