#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Tauri core. Runs unelevated (A-3). The UI can only call the commands below.
mod analysis;
mod debloat;
mod leftovers;
mod tweaks;

use analysis::*;
use oz_layout as layout;
use oz_model::{Index, IndexBuilder, NodeId, ROOT};
use oz_ops::{execute, guard::Guard, ledger, plan, relocate, Plan, Report};
use oz_rules::{Action, Matcher, Tier};
use oz_win::elevate::{launch_helper, read_frame, tag, write_frame, LaunchError, Pipe};
use oz_win::{fsops, mft, now_filetime, volume, walk};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, Response};
use tauri::State;

#[derive(Default)]
struct Scan {
    index: Option<Arc<Index>>,
    marks: Arc<Vec<bool>>,
    root: String,
    unread: Vec<String>,
    mode: String,
    complete: bool,
}

struct App {
    scan: Mutex<Scan>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    generation: AtomicU64,
    helper: Arc<Mutex<Option<Pipe>>>,
    matcher: Matcher,
    guard: Result<Guard, String>,
    data_dir: String,
    op_stop: Arc<AtomicBool>,
}

fn env_map() -> HashMap<String, String> {
    std::env::vars()
        .map(|(k, v)| (k.to_ascii_uppercase(), v))
        .collect()
}

fn index(app: &App) -> Result<Arc<Index>, String> {
    app.scan
        .lock()
        .unwrap()
        .index
        .clone()
        .ok_or_else(|| "No scan yet".to_string())
}

fn guard(app: &App) -> Result<&Guard, String> {
    app.guard.as_ref().map_err(|e| e.clone())
}

// ---------------------------------------------------------------- volumes

#[derive(Serialize)]
struct VolumeDto {
    root: String,
    label: String,
    fs: String,
    kind: String,
    total: u64,
    free: u64,
    fast: bool,
}

#[tauri::command]
fn volumes() -> Vec<VolumeDto> {
    volume::list()
        .into_iter()
        .map(|v| VolumeDto {
            fast: v.fs.eq_ignore_ascii_case("NTFS") && v.kind != "network",
            root: v.root,
            label: v.label,
            fs: v.fs,
            kind: v.kind.to_string(),
            total: v.total,
            free: v.free,
        })
        .collect()
}

// ---------------------------------------------------------------- scanning

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ScanEvent {
    Progress {
        items: u64,
        bytes: u64,
        ms: u64,
    },
    Partial {
        items: u64,
        bytes: u64,
        ms: u64,
    },
    Done {
        items: u64,
        ms: u64,
        partial: bool,
        mode: String,
    },
    Declined,
    Failed {
        message: String,
    },
}

/// Marks everything beneath cleanable rule roots for the Reclaimable lens.
fn compute_marks(ix: &Index, m: &Matcher) -> Vec<bool> {
    let mut marks = vec![false; ix.len()];
    for r in m
        .rules
        .iter()
        .filter(|r| r.action == Action::DeleteContents)
    {
        for root in m.roots(&r.id) {
            if let Some(id) = find_path(ix, &root) {
                let mut st = vec![id];
                while let Some(n) = st.pop() {
                    marks[n as usize] = true;
                    st.extend_from_slice(ix.children(n));
                }
            }
        }
    }
    marks
}

#[tauri::command]
fn start_scan(
    root: String,
    fast: bool,
    on_event: Channel<ScanEvent>,
    app: State<'_, Arc<App>>,
) -> Result<(), String> {
    if let Some(c) = app.cancel.lock().unwrap().take() {
        c.store(true, Ordering::Relaxed);
    }
    let gen = app.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    *app.cancel.lock().unwrap() = Some(cancel.clone());
    {
        let mut s = app.scan.lock().unwrap();
        *s = Scan {
            root: root.clone(),
            mode: if fast { "fast" } else { "standard" }.into(),
            ..Default::default()
        };
    }
    let app: Arc<App> = app.inner().clone();
    std::thread::spawn(move || {
        if fast {
            fast_scan(&app, gen, &root, cancel, &on_event);
        } else {
            standard_scan(&app, gen, &root, cancel, &on_event);
        }
    });
    Ok(())
}

fn publish(app: &App, gen: u64, ix: Index, unread: Vec<String>, complete: bool) -> bool {
    if app.generation.load(Ordering::SeqCst) != gen {
        return false;
    }
    let marks = compute_marks(&ix, &app.matcher);
    let mut s = app.scan.lock().unwrap();
    s.index = Some(Arc::new(ix));
    s.marks = Arc::new(marks);
    s.unread = unread;
    s.complete = complete;
    true
}

fn standard_scan(
    app: &Arc<App>,
    gen: u64,
    root: &str,
    cancel: Arc<AtomicBool>,
    ev: &Channel<ScanEvent>,
) {
    let start = std::time::Instant::now();
    let shared = walk::Shared::new(root);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    let (s2, r2) = (shared.clone(), root.to_string());
    let handle = std::thread::spawn(move || walk::run(s2, &r2, workers));

    let mut next_snapshot = std::time::Duration::from_millis(600);
    let mut last_snapshot = start.elapsed();
    while !handle.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if cancel.load(Ordering::Relaxed) {
            shared.cancel.store(true, Ordering::Relaxed);
        }
        let (items, bytes) = (
            shared.items.load(Ordering::Relaxed),
            shared.bytes.load(Ordering::Relaxed),
        );
        let ms = start.elapsed().as_millis() as u64;
        let _ = ev.send(ScanEvent::Progress { items, bytes, ms });
        // Snapshots cost time, so they get rarer as the tree grows.
        if start.elapsed() - last_snapshot >= next_snapshot {
            let t = std::time::Instant::now();
            let (ix, unread) = {
                let st = shared.builder.lock().unwrap();
                (st.b.build(), st.unread.clone())
            };
            if !publish(app, gen, ix, unread, false) {
                shared.cancel.store(true, Ordering::Relaxed);
                continue;
            }
            let _ = ev.send(ScanEvent::Partial { items, bytes, ms });
            next_snapshot = (t.elapsed() * 6).max(std::time::Duration::from_millis(600));
            last_snapshot = start.elapsed();
        }
    }
    let _ = handle.join();
    let partial = cancel.load(Ordering::Relaxed);
    let st = shared.builder.lock().unwrap();
    let items = st.b.len() as u64;
    if publish(app, gen, st.b.build(), st.unread.clone(), !partial) {
        let _ = ev.send(ScanEvent::Done {
            items,
            ms: start.elapsed().as_millis() as u64,
            partial,
            mode: "standard".into(),
        });
    }
}

fn helper_exe() -> Result<String, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let p = me.parent().ok_or("no exe folder")?.join("oz-helper.exe");
    if !p.exists() {
        return Err("oz-helper.exe was not found next to the app".into());
    }
    Ok(p.to_string_lossy().into_owned())
}

/// Launches the helper once per session; returns Ok(false) if UAC was declined.
pub(crate) fn ensure_helper(app: &App) -> Result<bool, String> {
    let mut h = app.helper.lock().unwrap();
    if h.is_some() {
        return Ok(true);
    }
    match launch_helper(&helper_exe()?) {
        Ok(p) => {
            *h = Some(p);
            Ok(true)
        }
        Err(LaunchError::Declined) => Ok(false),
        Err(LaunchError::Failed(e)) => Err(e),
    }
}

fn fast_scan(
    app: &Arc<App>,
    gen: u64,
    root: &str,
    cancel: Arc<AtomicBool>,
    ev: &Channel<ScanEvent>,
) {
    let start = std::time::Instant::now();
    match ensure_helper(app) {
        Ok(true) => {}
        Ok(false) => {
            let _ = ev.send(ScanEvent::Declined);
            return;
        }
        Err(message) => {
            let _ = ev.send(ScanEvent::Failed { message });
            return;
        }
    }
    let letter = root.as_bytes().first().copied().unwrap_or(b'C');
    let mut guard_pipe = app.helper.lock().unwrap();
    let Some(pipe) = guard_pipe.as_mut() else {
        return;
    };
    if write_frame(pipe, tag::SCAN, &[letter]).is_err() {
        *guard_pipe = None;
        let _ = ev.send(ScanEvent::Failed {
            message: "The helper stopped unexpectedly".into(),
        });
        return;
    }
    let mut asm = mft::Assembler::default();
    let mut bytes = 0u64;
    let mut failure = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            // Closing the pipe stops the helper; the next Fast scan prompts again.
            *guard_pipe = None;
            let _ = ev.send(ScanEvent::Done {
                items: 0,
                ms: start.elapsed().as_millis() as u64,
                partial: true,
                mode: "fast".into(),
            });
            return;
        }
        match read_frame(pipe) {
            Ok((tag::RECORDS, data)) => {
                let mut o = 0;
                while let Some(r) = oz_ntfs::decode_record(&data, &mut o) {
                    bytes += r.allocated;
                    asm.add(r);
                }
                let _ = ev.send(ScanEvent::Progress {
                    items: asm.count,
                    bytes,
                    ms: start.elapsed().as_millis() as u64,
                });
            }
            Ok((tag::SCAN_DONE, _)) => break,
            Ok((tag::ERROR, m)) => {
                failure = Some(String::from_utf8_lossy(&m).into_owned());
                break;
            }
            Ok(_) => {}
            Err(e) => {
                *guard_pipe = None;
                failure = Some(format!("The helper stopped unexpectedly: {e}"));
                break;
            }
        }
    }
    drop(guard_pipe);
    if let Some(message) = failure {
        let _ = ev.send(ScanEvent::Failed { message });
        return;
    }
    let b: IndexBuilder = asm.build(root);
    let items = b.len() as u64;
    if publish(app, gen, b.build(), Vec::new(), true) {
        let _ = ev.send(ScanEvent::Done {
            items,
            ms: start.elapsed().as_millis() as u64,
            partial: false,
            mode: "fast".into(),
        });
    }
}

#[tauri::command]
fn cancel_scan(app: State<'_, Arc<App>>) {
    if let Some(c) = app.cancel.lock().unwrap().as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- reading

#[tauri::command]
fn tree_page(
    root: u32,
    expanded: Vec<u32>,
    sort: String,
    offset: u32,
    limit: u32,
    app: State<'_, Arc<App>>,
) -> Result<Page, String> {
    let ix = index(&app)?;
    if !ix.contains(root) {
        return Err("Unknown item".into());
    }
    let set: HashSet<u32> = expanded.into_iter().collect();
    Ok(page(&ix, root, &set, &sort, offset, limit.min(2000)))
}

#[derive(Serialize)]
struct RuleInfo {
    id: String,
    name: String,
    explain: String,
    after: String,
    tier: Tier,
    action: Action,
    settings_uri: Option<String>,
    tool: Option<String>,
}

#[derive(Serialize)]
struct Details {
    row: Row,
    path: String,
    parent: u32,
    ancestors: Vec<(u32, String)>,
    rule: Option<RuleInfo>,
    guess: Option<String>,
    deletable: Option<String>,
}

#[tauri::command]
fn details(id: u32, app: State<'_, Arc<App>>) -> Result<Details, String> {
    let ix = index(&app)?;
    if !ix.contains(id) {
        return Err("Unknown item".into());
    }
    let path = ix.path(id);
    let rule = app.matcher.find(&path).map(|r| RuleInfo {
        id: r.id.clone(),
        name: r.name.clone(),
        explain: r.explain.clone(),
        after: r.after.clone(),
        tier: r.tier,
        action: r.action,
        settings_uri: r.settings_uri.clone(),
        tool: r.tool.clone(),
    });
    let guess = if rule.is_none() {
        oz_rules::guess_owner(&path)
    } else {
        None
    };
    let mut anc = Vec::new();
    let mut cur = id;
    while cur != oz_model::NONE {
        anc.push((cur, ix.name(cur).to_string()));
        if cur == ROOT {
            break;
        }
        cur = ix.parent(cur);
    }
    anc.reverse();
    let refused = guard(&app)?.check(&path).err().map(|r| r.0);
    let protected_rule = rule.as_ref().is_some_and(|r| r.tier == Tier::Protected);
    let deletable = refused.or(if protected_rule {
        Some("Managed by Windows or its own tool".into())
    } else {
        None
    });
    Ok(Details {
        row: row(&ix, id, 0, false),
        parent: ix.parent(id),
        path,
        ancestors: anc,
        rule,
        guess,
        deletable,
    })
}

#[tauri::command]
fn layout_map(
    node: u32,
    width: f32,
    height: f32,
    mode: String,
    logical: bool,
    app: State<'_, Arc<App>>,
) -> Result<Response, String> {
    let (ix, marks) = {
        let s = app.scan.lock().unwrap();
        (s.index.clone().ok_or("No scan yet")?, s.marks.clone())
    };
    let now = now_filetime();
    let mut v = if mode == "strata" {
        layout::strata(&ix, node, width, height, 8, now, logical)
    } else {
        layout::treemap(&ix, node, width, height, 12, now, logical)
    };
    for i in &mut v {
        if marks.get(i.id as usize).copied().unwrap_or(false) {
            i.kind |= 8;
        }
    }
    Ok(Response::new(layout::encode(&v)))
}

/// Names and sizes for the map's label overlay.
#[tauri::command]
fn labels(ids: Vec<u32>, app: State<'_, Arc<App>>) -> Result<Vec<(String, u64)>, String> {
    let ix = index(&app)?;
    Ok(ids
        .into_iter()
        .take(1000)
        .map(|i| {
            if ix.contains(i) {
                (ix.name(i).to_string(), ix.allocated(i))
            } else {
                (String::new(), 0)
            }
        })
        .collect())
}

/// Row number of `id` in the flattened tree, for scrolling to a selection.
#[tauri::command]
fn tree_locate(
    id: u32,
    expanded: Vec<u32>,
    sort: String,
    app: State<'_, Arc<App>>,
) -> Result<Option<u32>, String> {
    let ix = index(&app)?;
    let set: HashSet<u32> = expanded.into_iter().collect();
    let mut n = 0u32;
    let mut stack: Vec<NodeId> = sorted_children(&ix, ROOT, &sort)
        .into_iter()
        .rev()
        .collect();
    while let Some(c) = stack.pop() {
        if c == id {
            return Ok(Some(n));
        }
        n += 1;
        if set.contains(&c) {
            stack.extend(sorted_children(&ix, c, &sort).into_iter().rev());
        }
    }
    Ok(None)
}

#[derive(Serialize)]
struct Hover {
    name: String,
    alloc: u64,
    files: u32,
    dir: bool,
    modified: i64,
    path: String,
}

#[tauri::command]
fn hover_info(id: u32, app: State<'_, Arc<App>>) -> Result<Hover, String> {
    let ix = index(&app)?;
    if !ix.contains(id) {
        return Err("Unknown item".into());
    }
    Ok(Hover {
        name: ix.name(id).into(),
        alloc: ix.allocated(id),
        files: ix.files(id),
        dir: ix.is_dir(id),
        modified: ix.modified(id),
        path: ix.path(id),
    })
}

#[tauri::command]
fn search_index(
    query: String,
    limit: usize,
    app: State<'_, Arc<App>>,
) -> Result<SearchResult, String> {
    Ok(search(
        &*index(&app)?,
        &query,
        limit.min(1000),
        now_filetime(),
    ))
}

#[derive(Serialize)]
struct Tops {
    files: Vec<TopEntry>,
    folders: Vec<TopEntry>,
    types: Vec<TypeEntry>,
}

#[tauri::command]
fn tops(app: State<'_, Arc<App>>) -> Result<Tops, String> {
    let ix = index(&app)?;
    Ok(Tops {
        files: top_files(&ix, 200),
        folders: top_folders(&ix, 200),
        types: top_types(&ix, 200),
    })
}

#[tauri::command]
fn timeline_weeks(weeks: usize, app: State<'_, Arc<App>>) -> Result<Vec<Week>, String> {
    Ok(timeline(
        &*index(&app)?,
        weeks.clamp(4, 1040),
        now_filetime(),
    ))
}

#[derive(Serialize)]
struct Accounting {
    root: String,
    whole_volume: bool,
    scanned: u64,
    used: u64,
    unaccounted: i64,
    unread: Vec<String>,
    orphan: u64,
    mode: String,
    complete: bool,
    free: u64,
    total: u64,
}

#[tauri::command]
fn accounting(app: State<'_, Arc<App>>) -> Result<Accounting, String> {
    let s = app.scan.lock().unwrap();
    let ix = s.index.clone().ok_or("No scan yet")?;
    let drive = s
        .root
        .get(..2)
        .map(|d| format!("{d}\\"))
        .unwrap_or_default();
    let (total, free) = volume::free_space(&drive).unwrap_or((0, 0));
    let used = total.saturating_sub(free);
    let scanned = ix.allocated(ROOT);
    Ok(Accounting {
        whole_volume: s.root.trim_end_matches('\\').len() == 2,
        root: s.root.clone(),
        scanned,
        used,
        unaccounted: used as i64 - scanned as i64,
        unread: s.unread.iter().take(500).cloned().collect(),
        orphan: ix.orphan_allocated,
        mode: s.mode.clone(),
        complete: s.complete,
        free,
        total,
    })
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    fsops::reveal(&path)
}

#[tauri::command]
fn open_settings(uri: String) -> Result<(), String> {
    if !oz_rules::ALLOWED_URIS.contains(&uri.as_str()) {
        return Err("Not an allowed target".into());
    }
    fsops::open_uri(&uri)
}

// ---------------------------------------------------------------- reclaim

#[derive(Serialize)]
struct Candidate {
    rule: String,
    name: String,
    explain: String,
    after: String,
    tier: Tier,
    category: String,
    action: Action,
    path: String,
    bytes: u64,
    id: Option<u32>,
    min_age_days: u32,
    needs_admin: bool,
    settings_uri: Option<String>,
    tool: Option<String>,
    modified: i64,
}

#[tauri::command]
fn reclaim_candidates(app: State<'_, Arc<App>>) -> Result<Vec<Candidate>, String> {
    let ix = index(&app)?;
    let mut out = Vec::new();
    for r in &app.matcher.rules {
        for root in app.matcher.roots(&r.id) {
            let Some(id) = find_path(&ix, &root) else {
                continue;
            };
            let bytes = ix.allocated(id);
            if bytes == 0 {
                continue;
            }
            out.push(Candidate {
                rule: r.id.clone(),
                name: r.name.clone(),
                explain: r.explain.clone(),
                after: r.after.clone(),
                tier: r.tier,
                category: r.category.clone(),
                action: r.action,
                path: root,
                bytes,
                id: Some(id),
                min_age_days: r.min_age_days,
                needs_admin: r.needs_admin,
                settings_uri: r.settings_uri.clone(),
                tool: r.tool.clone(),
                modified: ix.modified(id),
            });
        }
    }
    let g = guard(&app)?;
    for d in dev_folders(&ix, 100) {
        // Never suggest what the guard would refuse, or what lives in a temp rule.
        if g.check(&d.path).is_err() || app.matcher.find(&d.path).is_some() {
            continue;
        }
        out.push(Candidate {
            rule: "dev-folder".into(),
            name: d.name.clone(),
            explain: "Looks like rebuildable project dependencies or build output, because a project file sits beside it. Only you know if the project still matters.".into(),
            after: "Rebuild or reinstall in the project to get it back.".into(),
            tier: Tier::Review,
            category: "Developer".into(),
            action: Action::DeleteContents,
            path: d.path,
            bytes: d.alloc,
            id: Some(d.id),
            min_age_days: 0,
            needs_admin: false,
            settings_uri: None,
            tool: None,
            modified: d.modified,
        });
    }
    Ok(out)
}

#[derive(Serialize)]
struct Preview {
    path: String,
    problem: Option<String>,
}

/// Checks a plan with no changes, so problems surface before confirmation (F-40).
#[tauri::command]
fn preview_plan(plan: Plan, app: State<'_, Arc<App>>) -> Result<Vec<Preview>, String> {
    let g = guard(&app)?;
    Ok(plan
        .items
        .iter()
        .map(|i| Preview {
            path: i.path.clone(),
            problem: plan::preflight(g, i)
                .err()
                .or_else(|| plan::cannot_recycle(i).map(String::from)),
        })
        .collect())
}

#[derive(Serialize)]
struct ExecDone {
    report: Report,
    freed: i64,
}

#[tauri::command]
async fn execute_plan(
    plan: Plan,
    elevated: bool,
    on_item: Channel<plan::ItemResult>,
    app: State<'_, Arc<App>>,
) -> Result<ExecDone, String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let g = guard(&app)?.clone();
        let drives: HashSet<String> = plan
            .items
            .iter()
            .filter_map(|i| i.path.get(..2))
            .map(|d| format!("{d}\\"))
            .collect();
        let free = |ds: &HashSet<String>| {
            ds.iter()
                .filter_map(|d| volume::free_space(d))
                .map(|x| x.1 as i64)
                .sum::<i64>()
        };
        let before = free(&drives);
        app.op_stop.store(false, Ordering::Relaxed);
        // The core's own check runs first even for elevated plans.
        let report = if elevated {
            for i in &plan.items {
                g.check(&i.path)
                    .map_err(|r| format!("Refused {}: {}", i.path, r.0))?;
            }
            if !ensure_helper(&app)? {
                return Err(
                    "Administrator permission was declined, so nothing changed.".to_string()
                );
            }
            let mut h = app.helper.lock().unwrap();
            let p = h.as_mut().ok_or("Helper unavailable")?;
            write_frame(
                p,
                tag::PLAN,
                &serde_json::to_vec(&plan).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let report: Report = loop {
                let (t, d) = read_frame(p).map_err(|e| e.to_string())?;
                match t {
                    tag::REPORT => break serde_json::from_slice(&d).map_err(|e| e.to_string())?,
                    tag::ERROR => return Err(String::from_utf8_lossy(&d).into_owned()),
                    _ => {}
                }
            };
            for r in &report.results {
                ledger::append(&app.data_dir, &plan.id, r);
                let _ = on_item.send(r.clone());
            }
            report
        } else {
            execute(&plan, &g, Some(&app.data_dir), &app.op_stop, |r| {
                let _ = on_item.send(r.clone());
            })
        };
        Ok(ExecDone {
            report,
            freed: free(&drives) - before,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop_operation(app: State<'_, Arc<App>>) {
    app.op_stop.store(true, Ordering::Relaxed);
}

#[tauri::command]
async fn component_cleanup(app: State<'_, Arc<App>>) -> Result<String, String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !ensure_helper(&app)? {
            return Err("Administrator permission was declined, so nothing changed.".to_string());
        }
        let mut h = app.helper.lock().unwrap();
        let p = h.as_mut().ok_or("Helper unavailable")?;
        write_frame(p, tag::COMPONENT_CLEANUP, &[]).map_err(|e| e.to_string())?;
        loop {
            let (t, d) = read_frame(p).map_err(|e| e.to_string())?;
            if t == tag::TOOL_DONE {
                return Ok(String::from_utf8_lossy(&d).into_owned());
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn ledger_entries(app: State<'_, Arc<App>>) -> Vec<serde_json::Value> {
    let mut v = ledger::read(&app.data_dir);
    v.reverse();
    v.truncate(2000);
    v
}

pub(crate) fn append_action(app: &App, op: &str, path: &str, action: &str, reason: &str) {
    let r = plan::ItemResult {
        path: path.to_string(),
        method: plan::Method::Hold,
        outcome: plan::Outcome::Done,
        reason: format!(
            "{action}{}{reason}",
            if reason.is_empty() { "" } else { ": " }
        ),
        bytes: 0,
        denied: false,
        held_at: None,
    };
    ledger::append(&app.data_dir, op, &r);
}

#[tauri::command]
fn restore_held(
    held_at: String,
    original: String,
    op: String,
    app: State<'_, Arc<App>>,
) -> Result<(), String> {
    guard(&app)?
        .check(&original)
        .map_err(|r| format!("Refused: {}", r.0))?;
    plan::restore(&held_at, &original)?;
    append_action(&app, &op, &original, "Restored", "");
    Ok(())
}

#[tauri::command]
fn purge_held(root: String, op: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    plan::purge_hold(&root)?;
    append_action(&app, &op, &root, "Purged", "");
    Ok(())
}

// ---------------------------------------------------------------- relocate

fn relocations_file(app: &App) -> String {
    format!("{}\\relocations.json", app.data_dir)
}

fn read_relocations(app: &App) -> Vec<relocate::Relocated> {
    std::fs::read(relocations_file(app))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_relocations(app: &App, v: &[relocate::Relocated]) {
    let _ = std::fs::create_dir_all(&app.data_dir);
    if let Ok(b) = serde_json::to_vec_pretty(v) {
        let _ = std::fs::write(relocations_file(app), b);
    }
}

#[derive(Serialize, Clone)]
struct RelocateProgress {
    done: u64,
    total: u64,
}

#[tauri::command]
async fn relocate_folder(
    source: String,
    dest_parent: String,
    on_progress: Channel<RelocateProgress>,
    app: State<'_, Arc<App>>,
) -> Result<relocate::Relocated, String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let g = guard(&app)?.clone();
        app.op_stop.store(false, Ordering::Relaxed);
        let mut last = 0u64;
        let r = relocate::relocate(&g, &source, &dest_parent, &app.op_stop, |done, total| {
            if done - last > 1 << 24 || done == total {
                last = done;
                let _ = on_progress.send(RelocateProgress { done, total });
            }
        })?;
        let mut v = read_relocations(&app);
        v.push(r.clone());
        write_relocations(&app, &v);
        append_action(
            &app,
            "relocate",
            &r.source,
            "Relocated",
            &format!("to {}", r.dest),
        );
        Ok(r)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn relocations(app: State<'_, Arc<App>>) -> Vec<relocate::Relocated> {
    read_relocations(&app)
}

#[tauri::command]
fn relocate_undo(source: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    let mut v = read_relocations(&app);
    let i = v
        .iter()
        .position(|r| r.source == source)
        .ok_or("Unknown relocation")?;
    relocate::undo(guard(&app)?, &v[i])?;
    append_action(&app, "relocate", &source, "Undid relocation", "");
    v.remove(i);
    write_relocations(&app, &v);
    Ok(())
}

#[tauri::command]
fn relocate_finish(source: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    let mut v = read_relocations(&app);
    let i = v
        .iter()
        .position(|r| r.source == source)
        .ok_or("Unknown relocation")?;
    relocate::finish(guard(&app)?, &v[i])?;
    append_action(
        &app,
        "relocate",
        &source,
        "Removed original after relocation",
        "",
    );
    v.remove(i);
    write_relocations(&app, &v);
    Ok(())
}

// ---------------------------------------------------------------- apps

#[derive(Serialize)]
struct AppDto {
    key: String,
    name: String,
    publisher: String,
    location: String,
    bytes: u64,
    measured: bool,
}

#[tauri::command]
fn installed_apps(app: State<'_, Arc<App>>) -> Vec<AppDto> {
    let ix = app.scan.lock().unwrap().index.clone();
    let mut out: Vec<AppDto> = oz_win::apps::list()
        .into_iter()
        .map(|a| {
            let measured = ix.as_ref().and_then(|i| {
                if a.location.len() > 3 {
                    find_path(i, &a.location).map(|id| i.allocated(id))
                } else {
                    None
                }
            });
            AppDto {
                bytes: measured.unwrap_or(a.est_kb * 1024),
                measured: measured.is_some(),
                key: a.key,
                name: a.name,
                publisher: a.publisher,
                location: a.location,
            }
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.bytes));
    out
}

#[derive(Serialize)]
struct UninstallResult {
    exit_code: i32,
    still_installed: bool,
    leftovers: Vec<leftovers::Leftover>,
}

#[tauri::command]
async fn run_uninstaller(key: String, app: State<'_, Arc<App>>) -> Result<UninstallResult, String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // The command comes from the registry by key; the UI never supplies it.
        let a = oz_win::apps::list()
            .into_iter()
            .find(|a| a.key == key)
            .ok_or("That program is no longer listed")?;
        let exit_code = oz_win::apps::run_uninstaller(&a.uninstall)?;
        let still_installed = oz_win::apps::list().iter().any(|x| x.key == key);
        append_action(
            &app,
            "uninstall",
            &a.name,
            "Ran uninstaller",
            &format!("exit {exit_code}, still listed: {still_installed}"),
        );
        // Only look for leftovers once the program is really gone.
        let others = oz_win::apps::list();
        let leftovers = if still_installed {
            Vec::new()
        } else {
            leftovers::find(&app, &a.name, &a.publisher, &a.location, &others)
        };
        Ok(UninstallResult {
            exit_code,
            still_installed,
            leftovers,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------- snapshots

fn snapshots_dir(app: &App) -> String {
    format!("{}\\snapshots", app.data_dir)
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    time: i64,
    root: String,
    total: u64,
    folders: Vec<(String, u64)>,
}

#[derive(Serialize)]
struct SnapshotMeta {
    file: String,
    time: i64,
    root: String,
    total: u64,
}

#[tauri::command]
fn snapshot_save(app: State<'_, Arc<App>>) -> Result<(), String> {
    let (ix, root) = {
        let s = app.scan.lock().unwrap();
        if !s.complete {
            return Err("Finish a full scan first".into());
        }
        (s.index.clone().ok_or("No scan yet")?, s.root.clone())
    };
    let snap = Snapshot {
        time: now_filetime(),
        root,
        total: ix.allocated(ROOT),
        folders: folder_sizes(&ix, 6, 16 << 20),
    };
    let d = snapshots_dir(&app);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    std::fs::write(
        format!("{d}\\{}.json", snap.time),
        serde_json::to_vec(&snap).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn snapshot_list(app: State<'_, Arc<App>>) -> Vec<SnapshotMeta> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(snapshots_dir(&app)) {
        for e in rd.flatten() {
            let p = e.path();
            if let Some(s) = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Snapshot>(&b).ok())
            {
                v.push(SnapshotMeta {
                    file: p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    time: s.time,
                    root: s.root,
                    total: s.total,
                });
            }
        }
    }
    v.sort_by_key(|m| std::cmp::Reverse(m.time));
    v
}

fn safe_snapshot(app: &App, file: &str) -> Result<String, String> {
    if file.contains(['\\', '/']) || !file.ends_with(".json") {
        return Err("Bad snapshot name".into());
    }
    Ok(format!("{}\\{file}", snapshots_dir(app)))
}

#[tauri::command]
fn snapshot_delete(file: String, app: State<'_, Arc<App>>) -> Result<(), String> {
    std::fs::remove_file(safe_snapshot(&app, &file)?).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct Change {
    path: String,
    before: u64,
    after: u64,
    delta: i64,
}

/// Compares a saved snapshot with the current scan (F-14). Only the deepest
/// folder explaining each change is shown, so one change isn't listed five times.
#[tauri::command]
fn snapshot_compare(file: String, app: State<'_, Arc<App>>) -> Result<Vec<Change>, String> {
    let b = std::fs::read(safe_snapshot(&app, &file)?).map_err(|e| e.to_string())?;
    let old: Snapshot = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    let ix = index(&app)?;
    let now: HashMap<String, u64> = folder_sizes(&ix, 6, 16 << 20).into_iter().collect();
    let was: HashMap<String, u64> = old.folders.into_iter().collect();
    Ok(compare_folders(&was, &now))
}

fn compare_folders(was: &HashMap<String, u64>, now: &HashMap<String, u64>) -> Vec<Change> {
    let keys: HashSet<&String> = was.keys().chain(now.keys()).collect();
    let mut all: Vec<Change> = keys
        .into_iter()
        .map(|k| {
            let (b, a) = (
                was.get(k).copied().unwrap_or(0),
                now.get(k).copied().unwrap_or(0),
            );
            Change {
                path: k.clone(),
                before: b,
                after: a,
                delta: a as i64 - b as i64,
            }
        })
        .filter(|c| c.delta.unsigned_abs() >= 1 << 20)
        .collect();
    all.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
    let mut kept: Vec<Change> = Vec::new();
    for c in all {
        let prefix = format!("{}\\", c.path.trim_end_matches('\\'));
        // Keep a parent only if children don't already explain most of its change.
        let explained: i64 = kept
            .iter()
            .filter(|k| k.path.starts_with(&prefix))
            .map(|k| k.delta)
            .sum();
        if (c.delta - explained).unsigned_abs() >= (c.delta.unsigned_abs() / 4).max(1 << 20) {
            kept.push(c);
        }
    }
    kept.sort_by_key(|c| std::cmp::Reverse(c.delta.unsigned_abs()));
    kept.truncate(300);
    kept
}

#[tauri::command]
fn export_csv(query: String, app: State<'_, Arc<App>>) -> Result<String, String> {
    use std::io::Write;
    let ix = index(&app)?;
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let dir = format!("{home}\\Downloads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = format!("{dir}\\oz-cleaner-{}.csv", now_filetime());
    let mut w = std::io::BufWriter::new(std::fs::File::create(&file).map_err(|e| e.to_string())?);
    let write_err = |e: std::io::Error| e.to_string();
    writeln!(
        w,
        "path,type,on_disk_bytes,logical_bytes,files,modified_filetime"
    )
    .map_err(write_err)?;
    let ids: Vec<NodeId> = if query.trim().is_empty() {
        ix.ids().collect()
    } else {
        search(&ix, &query, 0, now_filetime()).ids
    };
    for i in ids {
        let kind = if ix.is_dir(i) { "folder" } else { "file" };
        writeln!(
            w,
            "{},{kind},{},{},{},{}",
            csv_field(&ix.path(i)),
            ix.allocated(i),
            ix.logical(i),
            ix.files(i),
            ix.modified(i)
        )
        .map_err(write_err)?;
    }
    w.flush().map_err(write_err)?;
    Ok(file)
}

#[tauri::command]
fn data_location(app: State<'_, Arc<App>>) -> String {
    app.data_dir.clone()
}

#[tauri::command]
fn clear_history(app: State<'_, Arc<App>>) -> Result<(), String> {
    let d = snapshots_dir(&app);
    if fsops::exists(&d) {
        fsops::remove_tree(&d, &|| false).map_err(plan::reason_text)?;
    }
    Ok(())
}

fn main() {
    let env = env_map();
    let rules = oz_rules::load(oz_rules::CATALOG).expect("built-in rule catalog is valid");
    let matcher = Matcher::new(rules, &env);
    let data_dir = format!(
        "{}\\OzCleaner",
        env.get("LOCALAPPDATA")
            .cloned()
            .unwrap_or_else(|| ".".into())
    );
    let _ = std::fs::create_dir_all(&data_dir);
    let app = Arc::new(App {
        scan: Mutex::new(Scan::default()),
        cancel: Mutex::new(None),
        generation: AtomicU64::new(0),
        helper: Arc::new(Mutex::new(None)),
        matcher,
        guard: Guard::from_env().map_err(|r| r.0),
        data_dir,
        op_stop: Arc::new(AtomicBool::new(false)),
    });
    tauri::Builder::default()
        .manage(app)
        .invoke_handler(tauri::generate_handler![
            volumes,
            start_scan,
            cancel_scan,
            tree_page,
            details,
            layout_map,
            labels,
            tree_locate,
            hover_info,
            search_index,
            tops,
            timeline_weeks,
            accounting,
            reveal,
            open_settings,
            reclaim_candidates,
            preview_plan,
            execute_plan,
            stop_operation,
            component_cleanup,
            ledger_entries,
            restore_held,
            purge_held,
            relocate_folder,
            relocations,
            relocate_undo,
            relocate_finish,
            installed_apps,
            run_uninstaller,
            snapshot_save,
            snapshot_list,
            snapshot_delete,
            snapshot_compare,
            export_csv,
            data_location,
            clear_history,
            tweaks::tweaks_list,
            tweaks::tweak_apply,
            tweaks::tweak_revert,
            leftovers::remove_registry_leftovers,
            leftovers::restore_registry_backup,
            debloat::bloat_list,
            debloat::bloat_remove,
            debloat::open_store
        ])
        .run(tauri::generate_context!())
        .expect("error while running Oz Cleaner");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_shows_deepest_cause() {
        let was: HashMap<_, _> = [
            ("C:\\", 100u64 << 20),
            ("C:\\a", 50 << 20),
            ("C:\\a\\b", 10 << 20),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let now: HashMap<_, _> = [
            ("C:\\", 200u64 << 20),
            ("C:\\a", 150 << 20),
            ("C:\\a\\b", 110 << 20),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let c = compare_folders(&was, &now);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, "C:\\a\\b");
        assert_eq!(c[0].delta, 100 << 20);
    }

    #[test]
    fn catalog_rules_never_target_guarded_paths_without_exception() {
        let g = Guard::new(
            r"C:\Windows",
            r"C:\Users\me",
            &[r"C:\Program Files"],
            r"C:\ProgramData",
        );
        let env: HashMap<String, String> = [
            ("LOCALAPPDATA", r"C:\Users\me\AppData\Local"),
            ("WINDIR", r"C:\Windows"),
            ("SYSTEMDRIVE", "C:"),
            ("PROGRAMDATA", r"C:\ProgramData"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let m = Matcher::new(oz_rules::load(oz_rules::CATALOG).unwrap(), &env);
        for r in m
            .rules
            .iter()
            .filter(|r| r.action == Action::DeleteContents)
        {
            for root in m.roots(&r.id) {
                assert!(
                    g.check(&format!("{root}\\child")).is_ok(),
                    "{} blocked by guard at {root}",
                    r.id
                );
            }
        }
    }
}
