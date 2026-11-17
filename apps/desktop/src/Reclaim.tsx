import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppEntry, api, Leftover, Candidate, Change, ExecDone, filetimeToDate, fmtBytes, fmtDate, ItemResult, LedgerEntry, Method, midTrim, Plan, PlanItem, Relocated, SnapshotMeta, Tier } from "./api";

export interface ReclaimDraft { path: string; name: string; bytes: number; modified: number | null; }

/** Press and hold for irreversible actions; holding Space or Enter also works. */
export function HoldButton({ label, onDone, ms = 1200, disabled }: { label: string; onDone: () => void; ms?: number; disabled?: boolean }) {
  const ref = useRef<HTMLButtonElement>(null);
  const raf = useRef(0);
  const t0 = useRef(0);
  const cancel = () => { cancelAnimationFrame(raf.current); t0.current = 0; ref.current?.style.setProperty("--hold", "0"); };
  const start = () => {
    if (t0.current || disabled) return;
    t0.current = performance.now();
    const tick = (n: number) => {
      const p = (n - t0.current) / ms;
      ref.current?.style.setProperty("--hold", String(Math.min(1, p)));
      if (p >= 1) { cancel(); onDone(); } else raf.current = requestAnimationFrame(tick);
    };
    raf.current = requestAnimationFrame(tick);
  };
  useEffect(() => cancel, []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <button ref={ref} disabled={disabled} className="btn btn--danger" onPointerDown={start} onPointerUp={cancel} onPointerLeave={cancel}
      onKeyDown={(e) => { if ((e.key === " " || e.key === "Enter") && !e.repeat) { e.preventDefault(); start(); } }} onKeyUp={cancel} onBlur={cancel}>
      <span>{label}</span>
    </button>
  );
}


interface Line { key: string; cand?: Candidate; draft?: ReclaimDraft; method: Method; checked: boolean; }

function defaultMethod(t: Tier | "Manual"): Method {
  return t === "Rebuildable" ? "Permanent" : t === "Caution" ? "Hold" : t === "Review" ? "Hold" : "Recycle";
}

function toItem(l: Line): PlanItem {
  if (l.cand) {
    const contents = l.method === "Permanent" && l.cand.rule !== "dev-folder";
    return { path: l.cand.path, method: l.method, contents_only: contents, min_age_days: contents ? l.cand.min_age_days : 0, expect_modified: null, estimated: l.cand.bytes, rule: l.cand.rule };
  }
  const d = l.draft!;
  return { path: d.path, method: l.method, contents_only: false, min_age_days: 0, expect_modified: d.modified, estimated: d.bytes, rule: null };
}

export function ReclaimScreen({ hasScan, drafts, setDrafts, onChanged, stale, onRescan, canRescan, goVolumes, initialTab = "clean", onBack }: {
  initialTab?: "clean" | "relocate" | "programs"; onBack?: () => void;
  hasScan: boolean; drafts: ReclaimDraft[]; setDrafts: React.Dispatch<React.SetStateAction<ReclaimDraft[]>>; onChanged: () => void; stale: boolean; onRescan: (fast: boolean) => void; canRescan: boolean; goVolumes: () => void;
}) {
  const [tab] = useState<"clean" | "relocate" | "programs">(initialTab);
  if (tab === "clean") return <CleanTab hasScan={hasScan} drafts={drafts} setDrafts={setDrafts} onChanged={onChanged} stale={stale} onRescan={onRescan} canRescan={canRescan} goVolumes={goVolumes} />;
  return (
    <div className="page">
      <div className="page__inner gap-4">
        <div className="between wrap-row">
          {onBack ? <button className="btn btn--ghost btn--sm" onClick={onBack}>← Tools</button> : <span />}
        </div>
        {tab === "relocate" && <RelocateTab onChanged={onChanged} />}
        {tab === "programs" && <ProgramsTab hasScan={hasScan} onChanged={onChanged} />}
      </div>
    </div>
  );
}

function CleanTab({ hasScan, drafts, setDrafts, onChanged, stale, onRescan, canRescan, goVolumes }: {
  hasScan: boolean; drafts: ReclaimDraft[]; setDrafts: React.Dispatch<React.SetStateAction<ReclaimDraft[]>>; onChanged: () => void; stale: boolean; onRescan: (fast: boolean) => void; canRescan: boolean; goVolumes: () => void;
}) {
  const [lines, setLines] = useState<Line[]>([]);
  const [confirm, setConfirm] = useState<{ plan: Plan; problems: { path: string; problem: string | null }[] } | null>(null);
  const [running, setRunning] = useState(false);
  const [live, setLive] = useState<ItemResult[]>([]);
  const [done, setDone] = useState<{ freed: number; results: ItemResult[]; elevated: boolean } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [group, setGroup] = useState<"safe" | "check" | "keep">("safe");
  const [focusKey, setFocusKey] = useState<string | null>(null);

  const load = useCallback(() => {
    if (!hasScan) return;
    api.candidates().then((c) => {
      setLines((old) => {
        const keep = new Map(old.map((l) => [l.key, l]));
        const a: Line[] = c.filter((x) => x.tier !== "Protected").map((x) => {
          const key = `c:${x.path}`, k = keep.get(key);
          return { key, cand: x, method: k?.method ?? defaultMethod(x.tier), checked: k?.checked ?? x.tier === "Rebuildable" };
        });
        const p: Line[] = c.filter((x) => x.tier === "Protected").map((x) => ({ key: `c:${x.path}`, cand: x, method: "Recycle", checked: false }));
        return [...a, ...p];
      });
    }).catch((e) => setError(String(e)));
  }, [hasScan]);
  useEffect(load, [load, stale]);

  const draftLines: Line[] = useMemo(() => drafts.map((d) => {
    const key = `d:${d.path}`;
    const o = lines.find((l) => l.key === key);
    return { key, draft: d, method: o?.method ?? "Recycle", checked: o?.checked ?? true };
  }), [drafts, lines]);

  const update = (key: string, p: Partial<Line>) => {
    setLines((o) => {
      const has = o.some((l) => l.key === key);
      if (has) return o.map((l) => l.key === key ? { ...l, ...p } : l);
      const dl = draftLines.find((l) => l.key === key);
      return dl ? [...o, { ...dl, ...p }] : o;
    });
  };

  const selected = [...lines.filter((l) => l.cand && l.cand.tier !== "Protected" && l.checked), ...draftLines.filter((l) => l.checked)];
  const total = selected.reduce((a, l) => a + (l.cand?.bytes ?? l.draft?.bytes ?? 0), 0);

  const review = async () => {
    const plan: Plan = { id: `op-${Date.now()}`, items: selected.map(toItem) };
    setError(null);
    try { setConfirm({ plan, problems: await api.preview(plan) }); } catch (e) { setError(String(e)); }
  };

  const adminPaths = new Set(lines.filter((l) => l.cand?.needs_admin).map((l) => l.cand!.path));

  const lastItems = useRef<PlanItem[]>([]);
  const runPlan = async (plan: Plan, elevated: boolean) => {
    lastItems.current = plan.items;
    setConfirm(null);
    setRunning(true);
    setLive([]);
    setDone(null);
    setError(null);
    const onItem = (r: ItemResult) => setLive((o) => [...o, r]);
    let freed = 0;
    const results: ItemResult[] = [];
    try {
      const normal = plan.items.filter((i) => !elevated && !adminPaths.has(i.path));
      const admin = plan.items.filter((i) => elevated || adminPaths.has(i.path));
      if (normal.length) { const r: ExecDone = await api.execute({ id: plan.id, items: normal }, false, onItem); freed += r.freed; results.push(...r.report.results); }
      if (admin.length) { const r: ExecDone = await api.execute({ id: plan.id, items: admin }, true, onItem); freed += r.freed; results.push(...r.report.results); }
    } catch (e) { setError(String(e)); }
    setDone({ freed, results, elevated });
    setRunning(false);
    onChanged();
    const gone = new Set(results.filter((r) => r.outcome === "Done" || r.outcome === "Held").map((r) => r.path));
    setDrafts((d) => d.filter((x) => !gone.has(x.path)));
    setLines((o) => o.map((l) => ({ ...l, checked: false })));
  };

  if (!hasScan) {
    return (
      <div className="page"><div className="page__inner gap-4" style={{ paddingBlock: 48 }}>
        <h1 className="display">Scan a drive first.</h1>
        <p className="sub">Reclaim only suggests what a scan has actually measured.</p>
        <div><button className="btn btn--primary" onClick={goVolumes}>Choose a drive <span className="arrow" aria-hidden="true">↗</span></button></div>
      </div></div>
    );
  }

  const GROUPS: { id: "safe" | "check" | "keep"; name: string; tiers: Tier[]; text: string }[] = [
    { id: "safe", name: "Safe", tiers: ["Rebuildable"], text: "Rebuilds itself. Losing it costs only time." },
    { id: "check", name: "Check first", tiers: ["Caution", "Review"], text: "Probably fine, with a real cost. Goes to the Holding Area first." },
    { id: "keep", name: "Keep", tiers: ["Protected"], text: "Never deleted here. Explained, with the supported way to manage it." },
  ];
  const groupLines = (g: typeof GROUPS[number]) => [...lines.filter((l) => l.cand && g.tiers.includes(l.cand.tier)), ...(g.id === "check" ? draftLines : [])];
  const current = GROUPS.find((g) => g.id === group)!;
  const shown = groupLines(current);
  const focusLine = [...lines, ...draftLines].find((l) => l.key === focusKey) ?? shown[0] ?? null;

  const row = (l: Line, i: number) => {
    const name = l.cand?.name ?? l.draft!.name, bytes = l.cand?.bytes ?? l.draft?.bytes ?? 0, path = l.cand?.path ?? l.draft!.path;
    const keep = l.cand?.tier === "Protected";
    const on = focusLine?.key === l.key;
    return (
      <div key={l.key} className={`crow ${on ? "crow--on" : ""}`} style={{ "--i": i } as React.CSSProperties} onClick={() => setFocusKey(l.key)}>
        {keep ? <span /> : <input type="checkbox" className="check" aria-label={`Include ${name}`} checked={l.checked} onClick={(e) => e.stopPropagation()} onChange={(e) => update(l.key, { checked: e.target.checked })} />}
        <span className="crow__text">
          <span className="crow__name">{name}{l.cand?.needs_admin && <span className="muted"> · needs admin</span>}{l.cand?.min_age_days ? <span className="muted"> · older than {l.cand.min_age_days} days</span> : null}</span>
          <span className="crow__path" title={path}>{midTrim(path, 72)}</span>
        </span>
        <span className="crow__size">{fmtBytes(bytes)}</span>
      </div>
    );
  };

  const about = (l: Line) => {
    const tier = l.cand?.tier;
    const methods: Method[] = tier === "Rebuildable" ? ["Permanent"] : l.cand?.rule === "dev-folder" || !l.cand ? ["Recycle", "Hold", "Permanent"] : ["Hold", "Permanent"];
    const badge = tier === "Rebuildable" ? "Safe · rebuilds itself" : tier === "Protected" ? "Keep · explain only" : "Check first";
    return (
      <>
        <h2 className="clean__h">{l.cand?.name ?? l.draft!.name}</h2>
        <span className={`tag ${tier && tier !== "Rebuildable" && tier !== "Protected" ? "tag--signal" : ""}`} style={{ justifySelf: "start" }}>{badge}</span>
        <p className="sub">{l.cand?.explain ?? "You added this from Explore. Oz Cleaner makes no recommendation."}</p>
        {l.cand?.after && <div className="gap-2"><span className="label">If it goes</span><p className="sub">{l.cand.after}</p></div>}
        {tier !== "Protected" && methods.length > 1 && (
          <div className="gap-2"><span className="label">How</span>
            <div className="chips" role="group" aria-label="Method">
              {methods.map((m) => <button key={m} className={`chip ${l.method === m ? "chip--on" : ""}`} onClick={() => update(l.key, { method: m })}>{m === "Hold" ? "Holding Area" : m === "Recycle" ? "Recycle Bin" : "Delete"}</button>)}
            </div>
          </div>
        )}
        {l.cand?.settings_uri && <button className="btn btn--ghost btn--sm" style={{ justifySelf: "start" }} onClick={() => api.openSettings(l.cand!.settings_uri!)}>Open Windows storage settings</button>}
        {l.cand?.tool === "ComponentCleanup" && <ComponentCleanup />}
        <p className="pathbox">{l.cand?.path ?? l.draft!.path}</p>
        <div className="row">
          <button className="btn btn--ghost btn--sm" onClick={() => api.reveal(l.cand?.path ?? l.draft!.path).catch(() => {})}>Show in Explorer</button>
          {l.draft && <button className="btn btn--ghost btn--sm" onClick={() => setDrafts((d) => d.filter((x) => x.path !== l.draft!.path))}>Remove from list</button>}
        </div>
      </>
    );
  };

  return (
    <div className="clean">
      <div className="clean__main">
        <div className="clean__head">
          <h1 className="clean__title"><span style={{ fontWeight: 250 }}>Up to </span>{fmtBytes(groupLines(GROUPS[0]).reduce((a, l) => a + (l.cand?.bytes ?? 0), 0))} <span style={{ fontWeight: 250 }}>is safe to go</span></h1>
          <div className="chips" role="tablist" aria-label="Safety">
            {GROUPS.map((g) => {
              const n = groupLines(g).length;
              return <button key={g.id} role="tab" aria-selected={group === g.id} className={`chip ${group === g.id ? "chip--on" : ""}`} onClick={() => { setGroup(g.id); setFocusKey(null); }}>{g.name} · {n}</button>;
            })}
          </div>
        </div>
        <p className="muted">{current.text}</p>
        {error && <p className="note">{error}</p>}
        {stale && canRescan && <p className="note">These figures predate your last change. <button className="link" onClick={() => onRescan(false)}>Scan again</button></p>}
        {(running || done) && <Results running={running} live={live} done={done} onRetry={(paths) => runPlan({ id: `op-${Date.now()}`, items: lastItems.current.filter((i) => paths.has(i.path)) }, true)} />}
        <div className="clean__list stagger" key={group}>
          {shown.map(row)}
          {shown.length === 0 && <p className="sub" style={{ paddingBlock: 24 }}>Nothing in this group for this scan.</p>}
        </div>
        <div className="clean__bar">
          <span className="sub">{selected.length} selected · <strong className="data" style={{ color: "var(--text-1)" }}>{fmtBytes(total)}</strong> estimated</span>
          <div className="row">
            {group === "safe" && shown.length > 0 && <button className="btn btn--ghost" onClick={() => shown.forEach((l) => update(l.key, { checked: !shown.every((x) => x.checked) }))}>{shown.every((x) => x.checked) ? "Select none" : "Select all"}</button>}
            <button className="btn btn--primary" disabled={!selected.length || running} onClick={review}>Clean selected <span className="arrow" aria-hidden="true">↗</span></button>
          </div>
        </div>
      </div>
      <aside className="clean__about" aria-label="What is this">
        <span className="label">What is this</span>
        {focusLine ? about(focusLine) : <p className="muted">Select an item to see what it is.</p>}
      </aside>
      {confirm && <ConfirmDialog c={confirm} lines={[...lines, ...draftLines]} onCancel={() => setConfirm(null)} onGo={() => runPlan(confirm.plan, false)} />}
    </div>
  );
}

function ComponentCleanup() {
  const [state, setState] = useState<"idle" | "ask" | "running" | string>("idle");
  if (state === "idle") return <div><button className="link" onClick={() => setState("ask")}>Run Microsoft's cleanup ↗</button></div>;
  if (state === "ask") return (
    <div className="row wrap-row">
      <span className="muted">Runs StartComponentCleanup as administrator. It can take a long time.</span>
      <button className="btn btn--ghost btn--sm" onClick={() => { setState("running"); api.componentCleanup().then((c) => setState(c === "0" ? "Finished. Scan again to see the change." : `Windows reported code ${c}.`)).catch((e) => setState(String(e))); }}>Run</button>
      <button className="link" onClick={() => setState("idle")}>Cancel</button>
    </div>
  );
  return <p className="data muted">{state === "running" ? "Running. Windows controls how long this takes." : state}</p>;
}

function ConfirmDialog({ c, lines, onCancel, onGo }: { c: { plan: Plan; problems: { path: string; problem: string | null }[] }; lines: Line[]; onCancel: () => void; onGo: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); }, []);
  const problems = c.problems.filter((p) => p.problem);
  const bad = new Set(problems.map((p) => p.path));
  const items = c.plan.items.filter((i) => !bad.has(i.path));
  const sum = (m: Method) => items.filter((i) => i.method === m).reduce((a, i) => a + i.estimated, 0);
  const count = (m: Method) => items.filter((i) => i.method === m).length;
  const tierOf = (p: string) => lines.find((l) => (l.cand?.path ?? l.draft?.path) === p)?.cand?.tier;
  const riskyPermanent = items.some((i) => i.method === "Permanent" && tierOf(i.path) !== "Rebuildable");
  const anyPermanent = count("Permanent") > 0;
  return (
    <dialog ref={ref} className="modal" onClose={onCancel}>
      <div className="gap-4">
        <h2 className="title">Confirm</h2>
        <div className="gap-2">
          {count("Permanent") > 0 && <div className="between"><span className="signal">▲ Delete permanently: {count("Permanent")}</span><span className="data">{fmtBytes(sum("Permanent"))}</span></div>}
          {count("Recycle") > 0 && <div className="between"><span>To the Recycle Bin: {count("Recycle")}</span><span className="data">{fmtBytes(sum("Recycle"))}</span></div>}
          {count("Hold") > 0 && <div className="between"><span>To the Holding Area: {count("Hold")}</span><span className="data">{fmtBytes(sum("Hold"))}</span></div>}
        </div>
        {anyPermanent && <p className="sub">Permanent deletion can't be undone. Items in use are skipped.</p>}
        {count("Hold") > 0 && <p className="sub">Held items stay restorable from the Ledger, and their space is not freed until you purge them.</p>}
        {problems.length > 0 && (
          <div className="note gap-2">
            <p>{problems.length} {problems.length === 1 ? "item" : "items"} will be left alone. Nothing falls back to a less safe method.</p>
            <ul className="data" style={{ maxHeight: 140, overflow: "auto" }}>{problems.map((p) => <li key={p.path}>{midTrim(p.path, 52)}: {p.problem}</li>)}</ul>
          </div>
        )}
        <div className="row wrap-row">
          {riskyPermanent ? <HoldButton label="Hold to delete" onDone={onGo} /> : <button className="btn btn--primary" disabled={!items.length} onClick={onGo}>Go <span className="arrow">↗</span></button>}
          <button className="btn btn--ghost" onClick={onCancel}>Cancel</button>
        </div>
      </div>
    </dialog>
  );
}

function Results({ running, live, done, onRetry }: { running: boolean; live: ItemResult[]; done: { freed: number; results: ItemResult[] } | null; onRetry: (paths: Set<string>) => void }) {
  const rs = done?.results ?? live;
  const removed = rs.filter((r) => r.outcome === "Done" || r.outcome === "Partial").length;
  const held = rs.filter((r) => r.outcome === "Held");
  const skipped = rs.filter((r) => r.outcome === "Skipped").length;
  const denied = rs.filter((r) => r.denied);
  return (
    <section className="section gap-3" aria-live="polite">
      {running && <div className="row"><span className="label label--1">Working</span><button className="link" onClick={() => api.stop()}>Stop after this item</button></div>}
      <div className="stats">
        {done && <div className="stat"><div className="stat__n">{fmtBytes(Math.max(0, done.freed))}</div><span className="label">Freed, measured</span></div>}
        <div className="stat"><div className="stat__n">{removed}</div><span className="label">Removed</span></div>
        <div className="stat"><div className="stat__n">{held.length}</div><span className="label">Held, not yet freed</span></div>
        <div className="stat"><div className="stat__n">{skipped}</div><span className="label">Skipped</span></div>
      </div>
      <ul className="gap-2">
        {rs.map((r, i) => (
          <li key={i} className="between">
            <span className={`truncate data ${r.outcome === "Skipped" ? "" : "muted"}`} style={{ textDecoration: r.outcome === "Done" ? "line-through" : undefined }} title={r.path}>{midTrim(r.path, 70)}</span>
            <span className="data muted truncate" style={{ flexShrink: 0, maxWidth: "45%" }}>{r.reason || r.outcome}</span>
          </li>
        ))}
      </ul>
      {done && denied.length > 0 && (
        <div className="row wrap-row">
          <span className="muted">{denied.length} needed administrator permission.</span>
          <button className="btn btn--ghost btn--sm" onClick={() => onRetry(new Set(denied.map((r) => r.path)))}>Retry as administrator</button>
        </div>
      )}
    </section>
  );
}

function RelocateTab({ onChanged }: { onChanged: () => void }) {
  const [source, setSource] = useState("");
  const [dest, setDest] = useState("");
  const [busy, setBusy] = useState<{ done: number; total: number } | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [list, setList] = useState<Relocated[]>([]);
  const refresh = () => { api.relocations().then(setList).catch(() => {}); };
  useEffect(refresh, []);
  const ok = /^[a-zA-Z]:\\.+/.test(source.trim()) && /^[a-zA-Z]:\\/.test(dest.trim());
  const go = async () => {
    setMsg(null);
    setBusy({ done: 0, total: 0 });
    try {
      const r = await api.relocate(source.trim(), dest.trim(), setBusy);
      setMsg(`Moved ${r.files} files, ${fmtBytes(r.bytes)}. Every file was verified by hash. The original stays beside the junction until you remove it.`);
      setSource("");
      onChanged();
    } catch (e) { setMsg(String(e)); }
    setBusy(null);
    refresh();
  };
  return (
    <div className="gap-4">
      <h1 className="display">Move a folder to another drive</h1>
      <p className="sub" style={{ maxWidth: 640 }}>Copies a folder to another drive, verifies every file by hash, then leaves a junction so apps still find it at the old path. If anything fails before the junction exists, your folder stays as it was. Close apps using the folder first.</p>
      <label className="gap-2"><span className="label">Folder to move</span><span className="field"><input type="text" value={source} onChange={(e) => setSource(e.target.value)} placeholder="C:\Users\you\BigFolder" spellCheck={false} /></span></label>
      <label className="gap-2"><span className="label">New location, parent folder</span><span className="field"><input type="text" value={dest} onChange={(e) => setDest(e.target.value)} placeholder="E:\Moved" spellCheck={false} /></span></label>
      <div className="row"><button className="btn btn--primary" disabled={!ok || !!busy} onClick={go}>Move and verify <span className="arrow">↗</span></button>{busy && <button className="link" onClick={() => api.stop()}>Cancel</button>}</div>
      {busy && <div className="gap-2"><div className="progress"><div style={{ transform: `scaleX(${busy.total ? busy.done / busy.total : 0})` }} /></div><span className="data muted">{fmtBytes(busy.done)} of {fmtBytes(busy.total)} copied, then verifying</span></div>}
      {msg && <p className="note selectable">{msg}</p>}
      {list.length > 0 && (
        <section className="section">
          <h2 className="title" style={{ marginBlockEnd: 8 }}>Relocated</h2>
          {list.map((r) => (
            <div className="item" style={{ gridTemplateColumns: "minmax(0,1fr) auto" }} key={r.source}>
              <div className="gap-2" style={{ minWidth: 0 }}>
                <p className="data truncate" title={r.source}>{midTrim(r.source, 70)} → {midTrim(r.dest, 40)}</p>
                <p className="muted">{fmtBytes(r.bytes)} still occupied by the original until removed.</p>
              </div>
              <div className="row">
                <button className="btn btn--ghost btn--sm" onClick={() => api.relocateUndo(r.source).then(() => { setMsg("Undone. The folder is back where it was."); refresh(); onChanged(); }).catch((e) => setMsg(String(e)))}>Undo</button>
                <HoldButton label="Hold to remove original" onDone={() => api.relocateFinish(r.source).then(() => { setMsg("Original removed. The space is now free."); refresh(); onChanged(); }).catch((e) => setMsg(String(e)))} />
              </div>
            </div>
          ))}
        </section>
      )}
    </div>
  );
}

function ProgramsTab({ hasScan, onChanged }: { hasScan: boolean; onChanged: () => void }) {
  const [apps, setApps] = useState<AppEntry[] | null>(null);
  const [filter, setFilter] = useState("");
  const [ask, setAsk] = useState<AppEntry | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [left, setLeft] = useState<{ app: string; items: Leftover[] } | null>(null);
  const refresh = useCallback(() => { api.apps().then(setApps).catch(() => setApps([])); }, []);
  useEffect(refresh, [refresh, hasScan]);
  const run = (a: AppEntry) => {
    setAsk(null);
    setBusy(true);
    setMsg(`${a.name}'s own uninstaller is running. It may open its own window.`);
    api.uninstall(a.key).then((r) => {
      setMsg(r.still_installed
        ? `${a.name} is still installed (its uninstaller exited with code ${r.exit_code}). It may have refused or been cancelled. Oz Cleaner won't force it.`
        : r.leftovers.length ? `${a.name} was removed by its uninstaller. It left the items below behind.` : `${a.name} was removed by its uninstaller, and left nothing behind that Oz recognises.`);
      if (!r.still_installed && r.leftovers.length) setLeft({ app: a.name, items: r.leftovers });
      refresh();
      onChanged();
    }).catch((e) => setMsg(String(e))).finally(() => setBusy(false));
  };
  const shown = (apps ?? []).filter((a) => a.name.toLowerCase().includes(filter.toLowerCase()));
  return (
    <div className="gap-4">
      <h1 className="display">Programs by size</h1>
      <p className="sub">Uninstall A to Z: runs the program's own uninstaller, then finds its leftover folders, shortcuts and settings. If the uninstaller refuses, that's reported and nothing is forced.</p>
      <div className="field"><input type="text" aria-label="Filter programs" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter" spellCheck={false} /></div>
      {msg && <p className="note">{msg}</p>}
      {left && <Leftovers app={left.app} items={left.items} onDone={() => { onChanged(); }} onClose={() => setLeft(null)} />}
      {apps === null && <p className="label">Reading</p>}
      <table className="table">
        <thead><tr><th className="label">Program</th><th className="label">Publisher</th><th className="label" style={{ textAlign: "end" }}>Size</th><th /></tr></thead>
        <tbody>
          {shown.map((a) => (
            <tr key={a.key}>
              <td className="wrap">{a.name}</td>
              <td className="muted">{a.publisher}</td>
              <td style={{ textAlign: "end", whiteSpace: "nowrap" }} title={a.measured ? "Measured by the scan" : "As reported by the program"}>{a.bytes ? fmtBytes(a.bytes) : "—"}{a.bytes && !a.measured ? " est." : ""}</td>
              <td style={{ textAlign: "end" }}><button className="link" disabled={busy} onClick={() => setAsk(a)}>Uninstall</button></td>
            </tr>
          ))}
        </tbody>
      </table>
      {ask && <AskUninstall a={ask} onCancel={() => setAsk(null)} onGo={() => run(ask)} />}
    </div>
  );
}

function Leftovers({ app, items, onDone, onClose }: { app: string; items: Leftover[]; onDone: () => void; onClose: () => void }) {
  const [pick, setPick] = useState<Set<string>>(() => new Set(items.filter((i) => i.preselect).map((i) => i.path)));
  const [state, setState] = useState<"idle" | "busy" | "done">("idle");
  const [out, setOut] = useState<{ held: number; skipped: string[]; backup: string } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const chosen = items.filter((i) => pick.has(i.path));
  const total = chosen.reduce((a, i) => a + i.bytes, 0);
  const go = async () => {
    setState("busy"); setErr(null);
    try {
      const files = chosen.filter((i) => i.kind !== "registry");
      const regs = chosen.filter((i) => i.kind === "registry").map((i) => i.path);
      let held = 0; const skipped: string[] = [];
      if (files.length) {
        const plan: Plan = { id: `op-${Date.now()}`, items: files.map((i) => ({ path: i.path, method: "Hold", contents_only: false, min_age_days: 0, expect_modified: null, estimated: i.bytes, rule: "uninstall-leftover" })) };
        const r = await api.execute(plan, false, () => {});
        for (const x of r.report.results) { if (x.outcome === "Held" || x.outcome === "Done") held++; else skipped.push(`${midTrim(x.path, 60)}: ${x.reason}`); }
      }
      const backup = regs.length ? await api.removeRegLeftovers(regs) : "";
      setOut({ held: held + (backup ? regs.length : 0), skipped, backup });
      setState("done"); onDone();
    } catch (e) { setErr(String(e)); setState("idle"); }
  };
  return (
    <section className="panel gap-3" aria-label="Leftovers">
      <div className="between wrap-row"><h2 className="title">What {app} left behind</h2><button className="btn btn--ghost btn--sm" onClick={onClose}>Close</button></div>
      {state !== "done" && <>
        <div className="tweaks">
          {items.map((i) => (
            <label key={i.path} className="crow" style={{ minHeight: 56 }}>
              {i.confidence === "Keep" && i.kind !== "folder" ? <span /> : <input type="checkbox" className="check" checked={pick.has(i.path)} onChange={(e) => setPick((s) => { const n = new Set(s); if (e.target.checked) n.add(i.path); else n.delete(i.path); return n; })} aria-label={`Remove ${i.path}`} />}
              <span className="crow__text"><span className="crow__name">{i.reason}</span><span className="crow__path" title={i.path}>{midTrim(i.path, 80)}</span></span>
              <span className="row"><span className={`tag ${i.confidence === "Shared" || i.confidence === "Keep" ? "tag--signal" : ""}`}>{i.confidence}</span><span className="crow__size" style={{ minWidth: 72, textAlign: "end" }}>{i.kind === "registry" ? "Key" : fmtBytes(i.bytes)}</span></span>
            </label>
          ))}
        </div>
        <p className="muted">Files go to the Holding Area, so you can restore them from History. Registry keys are backed up before removal.</p>
        {err && <p className="note">{err}</p>}
        <div className="between wrap-row"><span className="sub">{chosen.length} selected · <strong>{fmtBytes(total)}</strong></span>
          <button className="btn btn--primary" disabled={!chosen.length || state === "busy"} onClick={go}>{state === "busy" ? "Removing…" : `Remove ${chosen.length} leftover${chosen.length === 1 ? "" : "s"}`}</button></div>
      </>}
      {state === "done" && out && (
        <div className="gap-3 pulse">
          <p className="sub">{out.held} item{out.held === 1 ? "" : "s"} removed. Files are in the Holding Area.</p>
          {out.skipped.length > 0 && <ul className="gap-2 muted">{out.skipped.map((s) => <li key={s}>{s}</li>)}</ul>}
          {out.backup && <div><button className="btn btn--ghost btn--sm" onClick={() => api.restoreRegBackup(out.backup).then(() => setErr("Registry keys restored.")).catch((e) => setErr(String(e)))}>Restore registry keys</button></div>}
          {err && <p className="note">{err}</p>}
        </div>
      )}
    </section>
  );
}

function AskUninstall({ a, onCancel, onGo }: { a: AppEntry; onCancel: () => void; onGo: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); }, []);
  return (
    <dialog ref={ref} className="modal" onClose={onCancel}>
      <div className="gap-4">
        <h2 className="title">Uninstall {a.name}?</h2>
        <p className="sub">This starts the program's own uninstaller, which takes over from here. Afterwards, Oz looks for what it left behind and shows you the list. Nothing else is removed until you choose.</p>
        <div className="row"><button className="btn btn--primary" onClick={onGo}>Start uninstaller</button><button className="btn btn--ghost" onClick={onCancel}>Cancel</button></div>
      </div>
    </dialog>
  );
}

const HOLD_PROMPT_DAYS = 14;

export function HistoryScreen({ hasScan }: { hasScan: boolean }) {
  const [tab, setTab] = useState<"ledger" | "compare">("ledger");
  return (
    <div className="page">
      <div className="page__inner gap-4">
        <div className="between wrap-row">
          <h1 className="display">History</h1>
          <div className="chips" role="tablist">{(["ledger", "compare"] as const).map((t) => <button key={t} role="tab" aria-selected={tab === t} className={`chip ${tab === t ? "chip--on" : ""}`} onClick={() => setTab(t)}>{t === "ledger" ? "Actions" : "Compare scans"}</button>)}</div>
        </div>
        {tab === "ledger" ? <LedgerTab /> : <CompareTab hasScan={hasScan} />}
      </div>
    </div>
  );
}

function LedgerTab() {
  const [entries, setEntries] = useState<LedgerEntry[] | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const load = useCallback(() => { api.ledger().then(setEntries).catch(() => setEntries([])); }, []);
  useEffect(load, [load]);
  const all = entries ?? [];
  const purged = new Set(all.filter((e) => e.reason.startsWith("Purged")).map((e) => e.op));
  const restored = new Set(all.filter((e) => e.reason.startsWith("Restored")).map((e) => `${e.op}|${e.path}`));
  const held = all.filter((e) => e.outcome === "Held" && e.held_at && !purged.has(e.op) && !restored.has(`${e.op}|${e.path}`));
  const heldBytes = held.reduce((a, e) => a + e.bytes, 0);
  const ops = [...new Set(held.map((e) => e.op))];
  const opRoot = (op: string) => { const e = held.find((x) => x.op === op)!; return e.held_at!.replace(/\\[^\\]+$/, ""); };
  const old = held.some((e) => (Date.now() - filetimeToDate(e.time).getTime()) / 86400000 > HOLD_PROMPT_DAYS);
  return (
    <div className="gap-4">
      
      {msg && <p className="note">{msg}</p>}
      {held.length > 0 && (
        <section className="section gap-3">
          <div className="between wrap-row"><h2 className="title">Holding Area</h2><span className="data">{fmtBytes(heldBytes)} held, not yet freed</span></div>
          {old && <p className="note">Some items have been held for over {HOLD_PROMPT_DAYS} days. If you haven't missed them, purging frees the space.</p>}
          {ops.map((op) => (
            <div key={op} className="gap-2">
              {held.filter((e) => e.op === op).map((e) => (
                <div className="between" key={e.held_at!}>
                  <span className="data truncate" title={e.path}>{midTrim(e.path, 70)}</span>
                  <div className="row"><span className="data muted">{fmtBytes(e.bytes)}</span><button className="link" onClick={() => api.restore(e.held_at!, e.path, e.op).then(() => { setMsg("Restored to its original location."); load(); }).catch((x) => setMsg(String(x)))}>Restore</button></div>
                </div>
              ))}
              <div><HoldButton label="Hold to purge this batch" onDone={() => api.purge(opRoot(op), op).then(() => { setMsg("Purged. The space is free."); load(); }).catch((x) => setMsg(String(x)))} /></div>
            </div>
          ))}
        </section>
      )}
      {entries?.length === 0 && <p className="sub">Nothing has been changed by Oz Cleaner yet.</p>}
      {all.length > 0 && (
        <table className="table">
          <thead><tr><th className="label">When</th><th className="label">Item</th><th className="label">Result</th><th className="label" style={{ textAlign: "end" }}>Size</th></tr></thead>
          <tbody>
            {all.map((e, i) => (
              <tr key={i}>
                <td style={{ whiteSpace: "nowrap" }}>{fmtDate(e.time)}</td>
                <td className="wrap selectable">{e.path}</td>
                <td>{e.reason || `${e.outcome}${e.method === "Permanent" ? ", deleted" : e.method === "Recycle" ? ", recycled" : ""}`}</td>
                <td style={{ textAlign: "end", whiteSpace: "nowrap" }}>{e.bytes ? fmtBytes(e.bytes) : "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function CompareTab({ hasScan }: { hasScan: boolean }) {
  const [snaps, setSnaps] = useState<SnapshotMeta[]>([]);
  const [changes, setChanges] = useState<Change[] | null>(null);
  const [active, setActive] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const load = () => { api.snapshots().then(setSnaps).catch(() => {}); };
  useEffect(load, []);
  const compare = (f: string) => { setActive(f); api.snapshotCompare(f).then(setChanges).catch((e) => setMsg(String(e))); };
  return (
    <div className="gap-4">
      <h2 className="title">What grew since a saved scan</h2>
      <p className="sub">Compares a saved scan with your current one. A summary is saved after each full drive scan unless you turn that off in Settings.</p>
      {msg && <p className="note">{msg}</p>}
      <div className="toolbar">
        <button className="btn btn--ghost btn--sm" disabled={!hasScan} onClick={() => api.snapshotSave().then(load).catch((e) => setMsg(String(e)))}>Save current scan</button>
        <button className="btn btn--ghost btn--sm" disabled={!hasScan} onClick={() => api.exportCsv("").then((f) => { setMsg(`Exported to ${f}`); api.reveal(f); }).catch((e) => setMsg(String(e)))}>Export scan as CSV</button>
      </div>
      {snaps.length === 0 && <p className="muted">No saved scans yet.</p>}
      <ul>
        {snaps.map((s) => (
          <li className="between" key={s.file} style={{ padding: "8px 0", borderBlockStart: "1px solid var(--line)" }}>
            <span className="data">{filetimeToDate(s.time).toISOString().slice(0, 16).replace("T", " ")} · {s.root} · {fmtBytes(s.total)}</span>
            <div className="row">
              <button className={`chip ${active === s.file ? "chip--on" : ""}`} disabled={!hasScan} onClick={() => compare(s.file)}>Compare</button>
              <button className="link" onClick={() => api.snapshotDelete(s.file).then(load)}>Delete</button>
            </div>
          </li>
        ))}
      </ul>
      {changes && (
        <section className="section">
          {changes.length === 0 ? <p className="sub">No folder changed by more than 1 MB. Only scans of the same drive compare meaningfully.</p> : (
            <table className="table">
              <thead><tr><th className="label">Folder</th><th className="label" style={{ textAlign: "end" }}>Before</th><th className="label" style={{ textAlign: "end" }}>Now</th><th className="label" style={{ textAlign: "end" }}>Change</th></tr></thead>
              <tbody>{changes.map((c) => (
                <tr key={c.path}>
                  <td className="wrap selectable">{c.path}</td>
                  <td style={{ textAlign: "end", whiteSpace: "nowrap" }} className="muted">{fmtBytes(c.before)}</td>
                  <td style={{ textAlign: "end", whiteSpace: "nowrap" }}>{fmtBytes(c.after)}</td>
                  <td style={{ textAlign: "end", whiteSpace: "nowrap" }}>{c.delta > 0 ? "+" : ""}{fmtBytes(c.delta)}</td>
                </tr>
              ))}</tbody>
            </table>
          )}
        </section>
      )}
    </div>
  );
}
