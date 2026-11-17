import { Channel, invoke } from "@tauri-apps/api/core";

export interface Volume { root: string; label: string; fs: string; kind: string; total: number; free: number; fast: boolean; }
export interface Row { id: number; name: string; depth: number; alloc: number; logical: number; files: number; modified: number; flags: number; kids: boolean; expanded: boolean; pct: number; }
export interface Page { total: number; offset: number; rows: Row[]; }
export type Tier = "Rebuildable" | "Caution" | "Review" | "Protected";
export type Action = "delete_contents" | "run_tool" | "open_settings" | "none";
export interface RuleInfo { id: string; name: string; explain: string; after: string; tier: Tier; action: Action; settings_uri: string | null; tool: string | null; }
export interface Details { row: Row; path: string; parent: number; ancestors: [number, string][]; rule: RuleInfo | null; guess: string | null; deletable: string | null; }
export interface Accounting { root: string; whole_volume: boolean; scanned: number; used: number; unaccounted: number; unread: string[]; orphan: number; mode: string; complete: boolean; free: number; total: number; }
export interface SearchRow { row: Row; path: string; }
export interface SearchResult { total: number; bytes: number; rows: SearchRow[]; ids: number[]; valid: boolean; }
export interface TopEntry { id: number; name: string; path: string; alloc: number; files: number; }
export interface TypeEntry { ext: string; class: string; alloc: number; files: number; }
export interface Tops { files: TopEntry[]; folders: TopEntry[]; types: TypeEntry[]; }
export interface Week { start: number; bytes: number; files: number; }
export interface Candidate { rule: string; name: string; explain: string; after: string; tier: Tier; category: string; action: Action; path: string; bytes: number; id: number | null; min_age_days: number; needs_admin: boolean; settings_uri: string | null; tool: string | null; modified: number; }
export type Method = "Permanent" | "Recycle" | "Hold";
export interface PlanItem { path: string; method: Method; contents_only: boolean; min_age_days: number; expect_modified: number | null; estimated: number; rule: string | null; }
export interface Plan { id: string; items: PlanItem[]; }
export interface ItemResult { path: string; method: Method; outcome: "Done" | "Partial" | "Held" | "Skipped"; reason: string; bytes: number; denied: boolean; held_at: string | null; }
export interface Report { id: string; results: ItemResult[]; cancelled: boolean; }
export interface ExecDone { report: Report; freed: number; }
export interface Relocated { source: string; dest: string; old: string; files: number; bytes: number; }
export interface AppEntry { key: string; name: string; publisher: string; location: string; bytes: number; measured: boolean; }
export interface SnapshotMeta { file: string; time: number; root: string; total: number; }
export interface Change { path: string; before: number; after: number; delta: number; }
export interface Hover { name: string; alloc: number; files: number; dir: boolean; modified: number; path: string; }
export interface LedgerEntry { time: number; op: string; path: string; method: Method; outcome: string; reason: string; bytes: number; held_at: string | null; }

export interface TweakInfo { id: string; name: string; explain: string; category: string; tier: "Recommended" | "Optional" | "ExplainOnly"; restart: "none" | "explorer" | "signout" | "reboot"; needs_admin: boolean; evidence: string; state: "on" | "off" | "default" | "unknown"; current: string; applied: boolean; locked: string | null; }

export interface Leftover { path: string; kind: "folder" | "shortcut" | "registry"; confidence: "Certain" | "Likely" | "Name only" | "Shared" | "Keep"; reason: string; bytes: number; preselect: boolean; }

export interface BloatApp { family: string; name: string; explain: string; recommended: boolean; installed: boolean; removable: boolean; }

export type ScanEvent =
  | { type: "progress" | "partial"; items: number; bytes: number; ms: number }
  | { type: "done"; items: number; ms: number; partial: boolean; mode: string }
  | { type: "declined" }
  | { type: "failed"; message: string };

export const api = {
  volumes: () => invoke<Volume[]>("volumes"),
  startScan: (root: string, fast: boolean, onEvent: (e: ScanEvent) => void) => {
    const ch = new Channel<ScanEvent>();
    ch.onmessage = onEvent;
    return invoke<void>("start_scan", { root, fast, onEvent: ch });
  },
  cancelScan: () => invoke<void>("cancel_scan"),
  treePage: (root: number, expanded: number[], sort: string, offset: number, limit: number) =>
    invoke<Page>("tree_page", { root, expanded, sort, offset, limit }),
  treeLocate: (id: number, expanded: number[], sort: string) => invoke<number | null>("tree_locate", { id, expanded, sort }),
  details: (id: number) => invoke<Details>("details", { id }),
  layout: (node: number, width: number, height: number, mode: string, logical: boolean) =>
    invoke<ArrayBuffer>("layout_map", { node, width, height, mode, logical }),
  labels: (ids: number[]) => invoke<[string, number][]>("labels", { ids }),
  hover: (id: number) => invoke<Hover>("hover_info", { id }),
  search: (query: string, limit: number) => invoke<SearchResult>("search_index", { query, limit }),
  tops: () => invoke<Tops>("tops"),
  timeline: (weeks: number) => invoke<Week[]>("timeline_weeks", { weeks }),
  accounting: () => invoke<Accounting>("accounting"),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  openSettings: (uri: string) => invoke<void>("open_settings", { uri }),
  candidates: () => invoke<Candidate[]>("reclaim_candidates"),
  preview: (plan: Plan) => invoke<{ path: string; problem: string | null }[]>("preview_plan", { plan }),
  execute: (plan: Plan, elevated: boolean, onItem: (r: ItemResult) => void) => {
    const ch = new Channel<ItemResult>();
    ch.onmessage = onItem;
    return invoke<ExecDone>("execute_plan", { plan, elevated, onItem: ch });
  },
  stop: () => invoke<void>("stop_operation"),
  componentCleanup: () => invoke<string>("component_cleanup"),
  ledger: () => invoke<LedgerEntry[]>("ledger_entries"),
  restore: (heldAt: string, original: string, op: string) => invoke<void>("restore_held", { heldAt, original, op }),
  purge: (root: string, op: string) => invoke<void>("purge_held", { root, op }),
  relocate: (source: string, destParent: string, onProgress: (p: { done: number; total: number }) => void) => {
    const ch = new Channel<{ done: number; total: number }>();
    ch.onmessage = onProgress;
    return invoke<Relocated>("relocate_folder", { source, destParent, onProgress: ch });
  },
  relocations: () => invoke<Relocated[]>("relocations"),
  relocateUndo: (source: string) => invoke<void>("relocate_undo", { source }),
  relocateFinish: (source: string) => invoke<void>("relocate_finish", { source }),
  apps: () => invoke<AppEntry[]>("installed_apps"),
  uninstall: (key: string) => invoke<{ exit_code: number; still_installed: boolean; leftovers: Leftover[] }>("run_uninstaller", { key }),
  removeRegLeftovers: (keys: string[]) => invoke<string>("remove_registry_leftovers", { keys }),
  restoreRegBackup: (file: string) => invoke<void>("restore_registry_backup", { file }),
  snapshotSave: () => invoke<void>("snapshot_save"),
  snapshots: () => invoke<SnapshotMeta[]>("snapshot_list"),
  snapshotDelete: (file: string) => invoke<void>("snapshot_delete", { file }),
  snapshotCompare: (file: string) => invoke<Change[]>("snapshot_compare", { file }),
  exportCsv: (query: string) => invoke<string>("export_csv", { query }),
  dataLocation: () => invoke<string>("data_location"),
  clearHistory: () => invoke<void>("clear_history"),
  tweaks: () => invoke<TweakInfo[]>("tweaks_list"),
  bloatList: () => invoke<BloatApp[]>("bloat_list"),
  bloatRemove: (family: string) => invoke<void>("bloat_remove", { family }),
  openStore: (family: string) => invoke<void>("open_store", { family }),
  tweakApply: (id: string) => invoke<void>("tweak_apply", { id }),
  tweakRevert: (id: string) => invoke<void>("tweak_revert", { id }),
};

export const FLAG = { DIR: 1, LINK: 2, HARDLINK: 4, PLACEHOLDER: 8, ADS: 16, SYSTEM: 32, UNREAD: 64 };

export function fmtBytes(n: number): string {
  const a = Math.abs(n);
  if (a < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB", "PB"];
  let v = a / 1024, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${n < 0 ? "-" : ""}${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${u[i]}`;
}

export function fmtCount(n: number): string { return n.toLocaleString("en-US"); }

const FT_EPOCH = 116444736000000000;
export function filetimeToDate(ft: number): Date { return new Date((ft - FT_EPOCH) / 10000); }
export function fmtDate(ft: number): string {
  if (!ft) return "—";
  const d = filetimeToDate(ft);
  return isNaN(d.getTime()) ? "—" : d.toISOString().slice(0, 10);
}
export function fmtMs(ms: number): string { return ms < 10000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms / 1000)} s`; }

/** Shortens long paths in the middle, keeping the start and the file name. */
export function midTrim(s: string, max = 64): string {
  if (s.length <= max) return s;
  const tail = Math.floor(max * 0.55);
  return `${s.slice(0, max - tail - 1)}…${s.slice(-tail)}`;
}

export const safeStore = {
  get(k: string): string | null { try { return localStorage.getItem(k); } catch { return null; } },
  set(k: string, v: string) { try { localStorage.setItem(k, v); } catch { /* per-viewer convenience only */ } },
};
