import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Accounting, api, fmtBytes, fmtCount, fmtMs, safeStore, ScanEvent, Volume } from "./api";
import { Atlas } from "./Atlas";
import { HistoryScreen, ReclaimScreen, ReclaimDraft } from "./Reclaim";
import { SettingsScreen, Prefs } from "./Settings";

export type Screen = "home" | "clean" | "atlas" | "tools" | "ledger" | "settings";
export type ToolTab = "programs" | "relocate";

export interface ScanState {
  root: string; running: boolean; items: number; bytes: number; ms: number; mode: string;
  /** Bumped whenever new data is ready to draw. */
  version: number;
  doneAt: number; partial: boolean; error: string | null; declined: boolean;
}

const emptyScan: ScanState = { root: "", running: false, items: 0, bytes: 0, ms: 0, mode: "", version: 0, doneAt: 0, partial: false, error: null, declined: false };

function loadPrefs(): Prefs {
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const d: Prefs = { motion: reduced ? "reduced" : "full", basis: "disk", density: "comfortable", history: true };
  try { return { ...d, ...JSON.parse(safeStore.get("oz-prefs") ?? "{}") }; } catch { return d; }
}

export function App() {
  const [screen, setScreen] = useState<Screen>("home");
  const [scan, setScan] = useState<ScanState>(emptyScan);
  const [prefs, setPrefsState] = useState<Prefs>(loadPrefs);
  const [acct, setAcct] = useState<Accounting | null>(null);
  const [acctOpen, setAcctOpen] = useState(false);
  const [drafts, setDrafts] = useState<ReclaimDraft[]>([]);
  const [stale, setStale] = useState(false);

  useEffect(() => {
    const r = document.documentElement;
    r.dataset.motion = prefs.motion;
    r.dataset.density = prefs.density;
    safeStore.set("oz-prefs", JSON.stringify(prefs));
  }, [prefs]);

  const setPrefs = useCallback((p: Partial<Prefs>) => setPrefsState((o) => ({ ...o, ...p })), []);
  const prefsRef = useRef(prefs);
  prefsRef.current = prefs;

  const [toolTab, setToolTab] = useState<ToolTab | null>(null);
  const startScan = useCallback((root: string, fast: boolean, to: Screen = "atlas") => {
    setScreen(to);
    setAcct(null);
    setStale(false);
    setScan({ ...emptyScan, root, running: true, mode: fast ? "fast" : "standard" });
    const handler = (e: ScanEvent) => {
      setScan((s) => {
        if (s.root !== root) return s;
        switch (e.type) {
          case "progress": return { ...s, items: e.items, bytes: e.bytes, ms: e.ms };
          case "partial": return { ...s, items: e.items, bytes: e.bytes, ms: e.ms, version: s.version + 1 };
          case "done": return { ...s, running: false, ms: e.ms, items: e.items || s.items, partial: e.partial, mode: e.mode, version: s.version + 1, doneAt: Date.now() };
          case "declined": return { ...s, running: false, declined: true };
          case "failed": return { ...s, running: false, error: e.message };
        }
      });
      if (e.type === "done") {
        api.accounting().then((a) => {
          setAcct(a);
          if (!e.partial && a.whole_volume && prefsRef.current.history) api.snapshotSave().catch(() => {});
        }).catch(() => {});
      }
      // A declined prompt is never an error: continue with the Standard scan.
      if (e.type === "declined") startScan(root, false);
    };
    api.startScan(root, fast, handler).catch((err) => setScan((s) => ({ ...s, running: false, error: String(err) })));
  }, []);

  const hasScan = scan.version > 0;
  const nav: [Screen, string, boolean][] = useMemo(() => [
    ["home", "Home", true],
    ["clean", "Clean", true],
    ["atlas", "Explore", scan.root !== ""],
    ["tools", "Tools", true],
    ["ledger", "History", true],
    ["settings", "Settings", true],
  ], [scan.root]);

  const win = getCurrentWindow();
  return (
    <div className="shell">
      <header className="bar drag" data-tauri-drag-region>
        <div className="bar__left" data-tauri-drag-region>
          <span className="label label--1" data-tauri-drag-region>Oz — Cleaner</span>
          {scan.root && <span className="label label--vol truncate" data-tauri-drag-region>{scan.root}{scan.running ? " · scanning" : scan.partial ? " · partial" : ""}</span>}
        </div>
        <nav className="nav" aria-label="Screens">
          {nav.map(([k, t, en]) => (
            <button key={k} className={`nav__item ${screen === k ? "nav__item--on" : ""}`} disabled={!en} aria-current={screen === k ? "page" : undefined} onClick={() => { setScreen(k); if (k === "tools") setToolTab(null); }}><span className="boldsafe" data-text={t}><span>{t}</span></span></button>
          ))}
        </nav>
        <div className="bar__right">
          <button className="winbtn" aria-label="Minimize" onClick={() => win.minimize()}><svg width="10" height="10"><path d="M0 5h10" stroke="currentColor" /></svg></button>
          <button className="winbtn" aria-label="Maximize" onClick={() => win.toggleMaximize()}><svg width="10" height="10"><rect x=".5" y=".5" width="9" height="9" fill="none" stroke="currentColor" /></svg></button>
          <button className="winbtn winbtn--close" aria-label="Close" onClick={() => win.close()}><svg width="10" height="10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg></button>
        </div>
      </header>

      <main className="screen">
        {screen === "home" && <HomeScreen onScan={startScan} current={scan} />}
        {screen === "clean" && (scan.running && !scan.doneAt
          ? <ScanningCard scan={scan} onStop={() => api.cancelScan()} />
          : <ReclaimScreen key="clean" initialTab="clean" hasScan={hasScan} drafts={drafts} setDrafts={setDrafts} onChanged={() => setStale(true)} stale={stale} onRescan={(f) => startScan(scan.root, f, "clean")} canRescan={scan.root !== ""} goVolumes={() => setScreen("home")} />)}
        {screen === "atlas" && <Atlas scan={scan} prefs={prefs} onCancel={() => api.cancelScan()} onRescan={(f) => startScan(scan.root, f)} stale={stale}
          onAdd={(d) => { setDrafts((o) => o.some((x) => x.path === d.path) ? o : [...o, d]); }} draftCount={drafts.length} onGoReclaim={() => setScreen("clean")} />}
        {screen === "tools" && (toolTab
          ? <ReclaimScreen key={toolTab} initialTab={toolTab} onBack={() => setToolTab(null)} hasScan={hasScan} drafts={drafts} setDrafts={setDrafts} onChanged={() => setStale(true)} stale={stale} onRescan={(f) => startScan(scan.root, f)} canRescan={scan.root !== ""} goVolumes={() => setScreen("home")} />
          : <ToolsScreen open={setToolTab} go={setScreen} />)}
        {screen === "ledger" && <HistoryScreen hasScan={hasScan} />}
        {screen === "settings" && <SettingsScreen prefs={prefs} setPrefs={setPrefs} />}
      </main>

      <footer className="status">
        <StatusLine scan={scan} acct={acct} onOpen={() => setAcctOpen(true)} />
        <span className="label truncate">{scan.mode ? `${scan.mode} scan${scan.doneAt ? ` · ${fmtCount(scan.items)} items · ${fmtMs(scan.ms)}` : ""}` : "Nothing changes until you confirm"}</span>
      </footer>
      {acctOpen && acct && <AccountingDialog a={acct} onClose={() => setAcctOpen(false)} />}
    </div>
  );
}

function StatusLine({ scan, acct, onOpen }: { scan: ScanState; acct: Accounting | null; onOpen: () => void }) {
  if (scan.running) return <span className="data muted">{fmtCount(scan.items)} items · {fmtBytes(scan.bytes)} · {fmtMs(scan.ms)}</span>;
  if (scan.error) return <span className="data signal truncate">{scan.error}</span>;
  if (!acct) return <span className="data muted">No scan yet</span>;
  if (!acct.whole_volume) return <span className="data muted">Scanned {fmtBytes(acct.scanned)} in {acct.root}</span>;
  return (
    <button className="data muted truncate" onClick={onOpen} title="What is unaccounted space?">
      Scanned {fmtBytes(acct.scanned)} · Used {fmtBytes(acct.used)} · Unaccounted {fmtBytes(Math.max(acct.unaccounted, 0))}{acct.unaccounted < 0 ? " (scan exceeds used)" : ""} ↗
    </button>
  );
}

function AccountingDialog({ a, onClose }: { a: Accounting; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); }, []);
  return (
    <dialog ref={ref} className="modal" onClose={onClose}>
      <div className="gap-4">
        <h2 className="title">Unaccounted space</h2>
        <div className="stats">
          <div className="stat"><div className="stat__n">{fmtBytes(a.scanned)}</div><span className="label">Scanned</span></div>
          <div className="stat"><div className="stat__n">{fmtBytes(a.used)}</div><span className="label">Used</span></div>
          <div className="stat"><div className="stat__n">{fmtBytes(Math.max(0, a.unaccounted))}</div><span className="label">Unaccounted</span></div>
        </div>
        <p className="sub">The gap between what Windows reports as used and what this scan could add up.</p>
        <ul className="gap-2 sub">
          {a.mode === "standard" && <li>Likely: folders this account can't read, including other users' profiles. A Fast scan reads every file record.</li>}
          {a.mode === "standard" && <li>Likely: NTFS metadata such as the MFT itself, which a folder walk can't see.</li>}
          <li>Likely: shadow copies and restore points, which store data outside normal files.</li>
          {a.orphan > 0 && <li>Measured: {fmtBytes(a.orphan)} in file records whose parent folder couldn't be resolved.</li>}
          {a.unaccounted < 0 && <li>The scan exceeds used space because files changed during it, or shared data is counted per file.</li>}
        </ul>
        {a.unread.length > 0 && (
          <details>
            <summary className="label">Measured: {a.unread.length}{a.unread.length >= 500 ? "+" : ""} folders could not be read</summary>
            <ul className="pathbox gap-2" style={{ maxHeight: 200, overflow: "auto", marginBlockStart: 8 }}>{a.unread.map((u) => <li key={u}>{u}</li>)}</ul>
          </details>
        )}
        <div><button className="btn btn--primary" onClick={onClose}>Close</button></div>
      </div>
    </dialog>
  );
}

function HomeScreen({ onScan, current }: { onScan: (root: string, fast: boolean, to?: Screen) => void; current: ScanState }) {
  const [vols, setVols] = useState<Volume[] | null>(null);
  const [folder, setFolder] = useState("");
  const [explained, setExplained] = useState(safeStore.get("oz-fast-explained") === "1");
  const [askFast, setAskFast] = useState<string | null>(null);
  useEffect(() => { api.volumes().then(setVols).catch(() => setVols([])); }, []);
  const fast = (root: string) => { if (explained) onScan(root, true); else setAskFast(root); };
  const folderOk = /^[a-zA-Z]:\\/.test(folder.trim());
  // The drive to clean: the system drive if present, else the fullest one. Every figure is measured.
  const target = vols && vols.length ? (vols.find((v) => /^c:/i.test(v.root)) ?? [...vols].sort((a, b) => a.free / Math.max(a.total, 1) - b.free / Math.max(b.total, 1))[0]) : null;
  const letter = target?.root.slice(0, 2) ?? "";
  return (
    <div className="home">
      <section className="hero" aria-labelledby="hero-title">
        <div className="hero__main">
          {target ? (
            <h1 id="hero-title" className="hero__title">
              <span className="light">{letter} has {fmtBytes(target.free)} free.</span><br />Let’s find <span className="accent">more.</span>
            </h1>
          ) : vols === null ? (
            <div aria-busy="true" aria-label="Reading drives" className="gap-3" style={{ minHeight: "calc(clamp(48px, 8.2vw, 116px) * 1.8)" }}>
              <div className="skel" style={{ height: "clamp(40px, 7vw, 96px)", width: "70%" }} />
              <div className="skel" style={{ height: "clamp(40px, 7vw, 96px)", width: "45%" }} />
            </div>
          ) : (
            <h1 id="hero-title" className="hero__title">No drives could be read.</h1>
          )}
          <p className="hero__lede">One scan finds temporary files, caches, and what Claude Code, Codex and npm leave behind. Each item says what it is and what happens if it goes. Nothing moves until you confirm.</p>
          <div className="hero__cta">
            <button className="btn btn--primary btn--xl" disabled={!target} onClick={() => target && onScan(target.root, false, "clean")}>
              Find what’s safe to delete{letter ? ` on ${letter}` : ""} <span className="arrow" aria-hidden="true">↗</span>
            </button>
            <span className="hero__note">No administrator permission needed.</span>
          </div>
        </div>
      </section>

      <section className="drives" aria-labelledby="drives-title">
        <div className="between wrap-row">
          <h2 id="drives-title" className="title">Drives</h2>
        </div>
        <div className="drives__grid stagger">
          {vols === null && [0, 1, 2].map((i) => <div key={i} className="skel skel--vol" style={{ "--i": i } as React.CSSProperties} />)}
          {vols?.map((v, i) => {
            const used = v.total - v.free, pct = v.total ? used / v.total : 0, tight = v.total > 0 && v.free / v.total < 0.1;
            return (
              <article className="drive" key={v.root} style={{ "--i": i } as React.CSSProperties}>
                <div className="drive__top">
                  <span className="drive__letter">{v.root.slice(0, 2)}</span>
                  <span className="sub truncate">{v.label || "Local disk"}</span>
                </div>
                <div className="drive__top">
                  <span className={`drive__free ${tight ? "signal" : ""}`}>{fmtBytes(v.free)}</span>
                  <span className="muted">free of {fmtBytes(v.total)}</span>
                </div>
                <div className="drive__bar" role="img" aria-label={`${Math.round(pct * 100)}% used`}><div className={tight ? "tight" : ""} style={{ width: `${pct * 100}%` }} /></div>
                <div className="drive__actions">
                  {v.fast && <button className="btn btn--ghost btn--sm" onClick={() => fast(v.root)}>Fast scan</button>}
                  <button className="btn btn--ghost btn--sm" onClick={() => onScan(v.root, false)}>{v.fast ? "Standard" : "Scan"}</button>
                  {current.root === v.root && current.doneAt > 0 && <span className="muted" style={{ alignSelf: "center" }}>Scanned</span>}
                </div>
              </article>
            );
          })}
        </div>
        <form className="field" style={{ maxWidth: 520 }} onSubmit={(e) => { e.preventDefault(); if (folderOk) onScan(folder.trim(), false); }}>
          <input type="text" aria-label="Scan one folder" placeholder="Or scan one folder, e.g. E:\code" value={folder} onChange={(e) => setFolder(e.target.value)} spellCheck={false} />
          <button className="btn btn--ghost btn--sm" disabled={!folderOk}>Scan</button>
        </form>
      </section>
      {askFast && <FastDialog onClose={() => setAskFast(null)} onYes={() => { safeStore.set("oz-fast-explained", "1"); setExplained(true); const r = askFast; setAskFast(null); onScan(r, true); }} onNo={() => { const r = askFast; setAskFast(null); onScan(r, false); }} />}
    </div>
  );
}

function ScanningCard({ scan, onStop }: { scan: ScanState; onStop: () => void }) {
  return (
    <div className="page">
      <div className="page__inner scanning" aria-live="polite">
        <p className="sub">Looking through {scan.root} for temporary files, caches and leftovers</p>
        <p className="scanning__n">{fmtCount(scan.items)} <span style={{ fontWeight: 250 }}>items</span></p>
        <p className="data muted">{fmtBytes(scan.bytes)} measured · {fmtMs(scan.ms)}</p>
        <div className="progress" aria-hidden="true"><div className="skel" style={{ height: "100%", background: "var(--accent-dim)" }} /></div>
        <div><button className="btn btn--ghost" onClick={onStop}>Stop</button></div>
      </div>
    </div>
  );
}

function ToolsScreen({ open, go }: { open: (t: ToolTab) => void; go: (s: Screen) => void }) {
  const tools: { name: string; text: string; run: () => void }[] = [
    { name: "Programs by size", text: "Every installed program, measured on disk. Runs the program’s own uninstaller.", run: () => open("programs") },
    { name: "Move a folder", text: "Copy to another drive, verify every file, leave a link behind so nothing breaks.", run: () => open("relocate") },
    { name: "Explore a drive", text: "Map, tree and file types for any drive or folder.", run: () => go("home") },
    { name: "Compare scans", text: "What grew since your last scan, folder by folder.", run: () => go("ledger") },
    { name: "Storage Sense", text: "Opens Windows’ own automatic cleanup settings.", run: () => { api.openSettings("ms-settings:storagesense").catch(() => {}); } },
  ];
  return (
    <div className="page">
      <div className="page__inner gap-4">
        <h1 className="display-xl" style={{ fontSize: "clamp(48px, 7vw, 96px)", marginBlock: "16px 24px" }}><span style={{ fontWeight: 250 }}>The</span> toolbox</h1>
        <div className="tools stagger">
          {tools.map((t, i) => (
            <button key={t.name} className="tool" style={{ "--i": i } as React.CSSProperties} onClick={t.run}>
              <span className="tool__name">{t.name}</span>
              <p className="sub">{t.text}</p>
              <span className="tool__go" aria-hidden="true">↗</span>
            </button>
          ))}
        </div>
        <p className="muted">Debloat arrives in milestone M6b. It isn’t shown until it’s built and tested.</p>
      </div>
    </div>
  );
}


function FastDialog({ onClose, onYes, onNo }: { onClose: () => void; onYes: () => void; onNo: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); }, []);
  return (
    <dialog ref={ref} className="modal" onClose={onClose}>
      <div className="gap-4">
        <h2 className="title">Fast scan needs one permission</h2>
        <p className="sub">Windows will ask for administrator permission so a small helper can read this drive's file records directly. It only reads names, sizes and dates, never file contents, and it stops when you close the app.</p>
        <div className="row wrap-row">
          <button className="btn btn--primary" onClick={onYes}>Continue <span className="arrow">↗</span></button>
          <button className="btn btn--ghost" onClick={onNo}>Use Standard</button>
        </div>
      </div>
    </dialog>
  );
}
