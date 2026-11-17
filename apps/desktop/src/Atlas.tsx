import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, Details, filetimeToDate, FLAG, fmtBytes, fmtCount, fmtDate, fmtMs, Hover, midTrim, Row, SearchResult, Tops, Week } from "./api";
import type { ScanState } from "./App";
import { KIND_DIR, KIND_SMALL, Lens, LENS_LEGEND, MapRenderer } from "./map";
import type { ReclaimDraft } from "./Reclaim";
import type { Prefs } from "./Settings";

type View = "map" | "strata" | "timeline";
type Tab = "tree" | "files" | "folders" | "types";

// Survives view switches, so the arrival moment plays once per finished scan.
let arrivalShownFor = 0;
const motionMs = (p: Prefs, ms: number) => (p.motion === "full" ? ms : 0);
const TYPE_NAMES = ["Other", "Video", "Images", "Audio", "Documents", "Archives", "Programs", "Code", "Data & disks"];

export function Atlas({ scan, prefs, onCancel, onRescan, onAdd, draftCount, onGoReclaim, stale }: {
  scan: ScanState; prefs: Prefs; onCancel: () => void; onRescan: (fast: boolean) => void;
  onAdd: (d: ReclaimDraft) => void; draftCount: number; onGoReclaim: () => void; stale: boolean;
}) {
  const [selected, setSelected] = useState<number | null>(null);
  const [hovered, setHovered] = useState<number | null>(null);
  const [focus, setFocus] = useState(0);
  const [view, setView] = useState<View>("map");
  const [lens, setLens] = useState<Lens>("none");
  const [tab, setTab] = useState<Tab>("tree");
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<SearchResult | null>(null);
  const [details, setDetails] = useState<Details | null>(null);
  const [treeW, setTreeW] = useState(380);
  const [showDetail, setShowDetail] = useState(true);
  const [treeOpen, setTreeOpen] = useState(false);
  const hasData = scan.version > 0;

  useEffect(() => { setFocus(0); setSelected(null); setDetails(null); setResult(null); setQuery(""); }, [scan.root, scan.mode]);

  useEffect(() => {
    if (selected === null || !hasData) { setDetails(null); return; }
    let live = true;
    api.details(selected).then((d) => live && setDetails(d)).catch(() => live && setDetails(null));
    return () => { live = false; };
  }, [selected, hasData, scan.version]);

  useEffect(() => {
    if (!hasData || !query.trim()) { setResult(null); return; }
    const t = setTimeout(() => { api.search(query, 300).then(setResult).catch(() => setResult(null)); }, 120);
    return () => clearTimeout(t);
  }, [query, hasData, scan.version]);

  const matchSet = useMemo(() => (result ? new Set(result.ids) : null), [result]);

  const onDrag = (e: React.PointerEvent) => {
    const x0 = e.clientX, w0 = treeW;
    const move = (m: PointerEvent) => setTreeW(Math.min(700, Math.max(220, w0 + m.clientX - x0)));
    const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const zoomTo = useCallback((id: number) => setFocus(id), []);

  return (
    <div className={`atlas ${showDetail && details ? "" : "atlas--nodetail"}`} style={{ "--tree-w": `${treeW}px` } as React.CSSProperties}>
      <div className={`tree ${treeOpen ? "tree--open" : ""}`}>
        <div className="tree__tabs">
          <div className="chips" role="tablist" aria-label="Lists">
            {(["tree", "files", "folders", "types"] as Tab[]).map((t) => (
              <button key={t} role="tab" aria-selected={tab === t} className={`chip ${tab === t ? "chip--on" : ""}`} onClick={() => setTab(t)}>{t === "tree" ? "Folders" : t === "files" ? "Top files" : t === "folders" ? "Top folders" : "Types"}</button>
            ))}
          </div>
        </div>
        <SearchBar query={query} setQuery={setQuery} result={result} />
        {query.trim() && result ? (
          <SearchList result={result} selected={selected} onSelect={setSelected} onHover={setHovered} />
        ) : tab === "tree" ? (
          <TreeTable version={scan.version} selected={selected} hovered={hovered} onSelect={setSelected} onHover={setHovered} basis={prefs.basis} />
        ) : (
          <TopList tab={tab} version={scan.version} selected={selected} onSelect={setSelected} onHover={setHovered} setQuery={setQuery} />
        )}
      </div>
      <div className="splitter" role="separator" aria-orientation="vertical" onPointerDown={onDrag} />
      <div className="viz">
        <div className="viz__top">
          <button className="link tree-toggle" onClick={() => setTreeOpen((o) => !o)}>List</button>
          <Breadcrumb focus={focus} version={scan.version} onGo={zoomTo} />
          <div className="row">
            {view !== "timeline" && (
              <div className="chips" role="group" aria-label="Lens">
                {(["none", "type", "age", "reclaim"] as Lens[]).map((l) => (
                  <button key={l} className={`chip ${lens === l ? "chip--on" : ""}`} aria-pressed={lens === l} onClick={() => setLens(l)}>{l === "none" ? "Size" : l === "type" ? "Type" : l === "age" ? "Age" : "Cleanable"}</button>
                ))}
              </div>
            )}
            <span style={{ width: 8 }} />
            <div className="chips" role="tablist" aria-label="View">
              {(["map", "strata", "timeline"] as View[]).map((v) => (
                <button key={v} role="tab" aria-selected={view === v} className={`chip ${view === v ? "chip--on" : ""}`} onClick={() => setView(v)}>{v === "map" ? "Map" : v === "strata" ? "Strata" : "Timeline"}</button>
              ))}
            </div>
          </div>
        </div>
        {view === "timeline" ? (
          <TimelineView version={scan.version} setQuery={setQuery} />
        ) : (
          <MapStage mode={view} prefs={prefs} version={scan.version} focus={focus} setFocus={setFocus} selected={selected} hovered={hovered}
            onSelect={setSelected} onHover={setHovered} lens={lens} matches={matchSet} doneAt={scan.doneAt} running={scan.running} />
        )}
        {scan.running && (
          <div className="scanbox" role="status">
            <span className="label label--1">{scan.mode} scan</span>
            <span className="data">{fmtCount(scan.items)} items</span>
            <span className="data muted">{fmtBytes(scan.bytes)} · {fmtMs(scan.ms)}</span>
            {scan.mode === "standard" && scan.ms > 0 && <span className="data muted">{fmtCount(Math.round(scan.items / Math.max(scan.ms / 1000, .1)))} items/s</span>}
            <button className="link" style={{ textAlign: "start", marginBlockStart: 4 }} onClick={onCancel}>Stop, keep what's found</button>
          </div>
        )}
        {!scan.running && hasData && (scan.partial || stale) && (
          <div className="scanbox">
            <span className="label signal">{stale ? "Files changed" : "Partial results"}</span>
            <button className="link" style={{ textAlign: "start" }} onClick={() => onRescan(scan.mode === "fast")}>Scan again ↗</button>
          </div>
        )}
      </div>
      {showDetail && details && (
        <DetailPanel key={details.row.id} d={details} onClose={() => setShowDetail(false)} onZoom={() => zoomTo(details.row.kids ? details.row.id : details.parent)} onAdd={onAdd} draftCount={draftCount} onGoReclaim={onGoReclaim} />
      )}
      {!showDetail && details && (
        <button className="link" style={{ position: "absolute", insetBlockStart: 10, insetInlineEnd: 12, display: "none" }} onClick={() => setShowDetail(true)}>Details</button>
      )}
    </div>
  );
}

function SearchBar({ query, setQuery, result }: { query: string; setQuery: (q: string) => void; result: SearchResult | null }) {
  return (
    <div className="tree__tabs" style={{ gap: 8 }}>
      <label className="search" title="Examples: report   ext:iso   size>1g   age>1y   ext:mp4,mkv size>500m">
        <span className="sr">Search and filter</span>
        <span className="muted" aria-hidden>⌕</span>
        <input type="text" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search · ext:iso size>1g age>1y" spellCheck={false} />
      </label>
      {query && <button className="link" onClick={() => setQuery("")}>Clear</button>}
      {result && !result.valid && <span className="label signal">Check filter</span>}
    </div>
  );
}

function SearchList({ result, selected, onSelect, onHover }: { result: SearchResult; selected: number | null; onSelect: (i: number) => void; onHover: (i: number | null) => void }) {
  return (
    <>
      <div className="tree__head" style={{ gridTemplateColumns: "1fr auto" }}>
        <span className="label">{fmtCount(result.total)} matches</span>
        <span className="data muted">{fmtBytes(result.bytes)} in files</span>
      </div>
      <div className="tree__scroll" style={{ contain: "none" }} onMouseLeave={() => onHover(null)}>
        {result.rows.length === 0 && <p className="muted" style={{ padding: 12 }}>Nothing matches.</p>}
        {result.rows.map((r) => (
          <button key={r.row.id} title={r.path} className={`trow ${selected === r.row.id ? "trow--sel" : ""}`} style={{ position: "relative", width: "100%", textAlign: "start", height: "auto", padding: "5px 12px" }} onClick={() => onSelect(r.row.id)} onMouseEnter={() => onHover(r.row.id)}>
            <span className="truncate">{r.row.name}<br /><span className="data muted">{midTrim(r.path, 44)}</span></span>
            <span className="num">{fmtBytes(r.row.alloc)}</span>
            <span />
          </button>
        ))}
      </div>
    </>
  );
}

function TreeTable({ version, selected, hovered, onSelect, onHover, basis }: {
  version: number; selected: number | null; hovered: number | null; onSelect: (i: number) => void; onHover: (i: number | null) => void; basis: Prefs["basis"];
}) {
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [sort, setSort] = useState(basis === "logical" ? "logical" : "alloc");
  const [rows, setRows] = useState<Map<number, Row>>(new Map());
  const [total, setTotal] = useState(0);
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewH, setViewH] = useState(600);
  const rowH = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--row")) || 28;

  useEffect(() => { setSort(basis === "logical" ? "logical" : "alloc"); }, [basis]);
  useEffect(() => { if (version <= 1) setExpanded(new Set()); }, [version]);

  const first = Math.max(0, Math.floor(scrollTop / rowH) - 10);
  const count = Math.ceil(viewH / rowH) + 20;
  const reqKey = `${version}|${sort}|${[...expanded].join(",")}`;
  const lastKey = useRef("");

  useEffect(() => {
    if (!version) return;
    let live = true;
    api.treePage(0, [...expanded], sort, first, count).then((p) => {
      if (!live) return;
      setTotal(p.total);
      setRows((old) => {
        const m = lastKey.current === reqKey ? new Map(old) : new Map<number, Row>();
        p.rows.forEach((r, i) => m.set(p.offset + i, r));
        return m;
      });
      lastKey.current = reqKey;
    }).catch(() => {});
    return () => { live = false; };
  }, [reqKey, first, count, version]);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // A selection from the map opens its ancestors and scrolls to it.
  useEffect(() => {
    if (selected === null || !version) return;
    let live = true;
    api.details(selected).then(async (d) => {
      const next = new Set(expanded);
      let changed = false;
      for (const [id] of d.ancestors.slice(0, -1)) if (id !== 0 && !next.has(id)) { next.add(id); changed = true; }
      if (!live) return;
      if (changed) setExpanded(next);
      const idx = await api.treeLocate(selected, [...next], sort);
      if (live && idx !== null && scroller.current) {
        const top = idx * rowH, el = scroller.current;
        if (top < el.scrollTop || top + rowH > el.scrollTop + el.clientHeight) el.scrollTop = Math.max(0, top - el.clientHeight / 2);
      }
    }).catch(() => {});
    return () => { live = false; };
    // Only a new selection should drive this.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  const toggle = (id: number) => setExpanded((o) => { const n = new Set(o); if (n.has(id)) n.delete(id); else n.add(id); return n; });

  const onKey = (e: React.KeyboardEvent) => {
    const list = [...rows.entries()].sort((a, b) => a[0] - b[0]);
    const i = list.findIndex(([, r]) => r.id === selected);
    const cur = i >= 0 ? list[i][1] : null;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = list[Math.max(0, Math.min(list.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (n) onSelect(n[1].id);
    } else if (cur && e.key === "ArrowRight" && cur.kids && !cur.expanded) { e.preventDefault(); toggle(cur.id); }
    else if (cur && e.key === "ArrowLeft" && cur.expanded) { e.preventDefault(); toggle(cur.id); }
    else if (cur && e.key === "Enter" && cur.kids) { e.preventDefault(); toggle(cur.id); }
  };

  const visible: [number, Row][] = [];
  for (let i = first; i < first + count; i++) { const r = rows.get(i); if (r) visible.push([i, r]); }

  return (
    <>
      <div className="tree__head">
        <button className="label" style={{ textAlign: "start", color: sort === "name" ? "var(--text-1)" : undefined }} onClick={() => setSort("name")}>Name</button>
        <button className="label" style={{ textAlign: "end", color: sort === "alloc" || sort === "logical" ? "var(--text-1)" : undefined }} onClick={() => setSort(sort === "alloc" ? "logical" : "alloc")} title="Click to switch between on-disk and logical size">{sort === "logical" ? "Logical" : "On disk"}</button>
        <button className="label" style={{ textAlign: "end", color: sort === "modified" ? "var(--text-1)" : undefined }} onClick={() => setSort(sort === "modified" ? "alloc" : "modified")} title="Switch between share of parent and newest date">{sort === "modified" ? "Date" : "Share"}</button>
      </div>
      <div className="tree__scroll" ref={scroller} role="tree" aria-label="Folders and files" tabIndex={0} onKeyDown={onKey} onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)} onMouseLeave={() => onHover(null)}>
        <div style={{ height: total * rowH }} />
        {visible.map(([i, r]) => (
          <div key={r.id} role="treeitem" aria-selected={selected === r.id} aria-expanded={r.kids ? r.expanded : undefined} aria-level={r.depth + 1}
            className={`trow ${selected === r.id ? "trow--sel" : hovered === r.id ? "trow--hover" : ""}`}
            style={{ top: i * rowH }} onClick={() => onSelect(r.id)} onDoubleClick={() => r.kids && toggle(r.id)} onMouseEnter={() => onHover(r.id)}>
            <div className="trow__name" style={{ paddingInlineStart: Math.min(r.depth, 24) * 12 }}>
              {r.kids ? (
                <button className={`tw ${r.expanded ? "tw--open" : ""}`} aria-label={r.expanded ? "Collapse" : "Expand"} tabIndex={-1} onClick={(e) => { e.stopPropagation(); toggle(r.id); }}>
                  <svg width="8" height="8"><path d="M2 0l4 4-4 4" fill="none" stroke="currentColor" /></svg>
                </button>
              ) : <span className="tw" />}
              <span style={{ color: r.flags & FLAG.DIR ? "var(--text-1)" : "var(--text-2)" }}>
                {r.name}{r.flags & FLAG.LINK ? " ↗" : ""}{r.flags & FLAG.UNREAD ? " ⚠" : ""}
              </span>
            </div>
            <span className="num">{fmtBytes(sort === "logical" ? r.logical : r.alloc)}</span>
            <span className="num num--3">{sort === "modified" ? fmtDate(r.modified).slice(2, 7) : `${Math.round(r.pct)}%`}</span>
            <div className="trow__bar" style={{ width: `${Math.min(100, r.pct)}%` }} />
          </div>
        ))}
      </div>
    </>
  );
}

function TopList({ tab, version, selected, onSelect, onHover, setQuery }: { tab: Tab; version: number; selected: number | null; onSelect: (i: number) => void; onHover: (i: number | null) => void; setQuery: (q: string) => void }) {
  const [tops, setTops] = useState<Tops | null>(null);
  useEffect(() => { if (version) api.tops().then(setTops).catch(() => {}); }, [version]);
  if (!tops) return <p className="label" style={{ padding: 12 }}>Reading</p>;
  if (tab === "types") {
    return (
      <div className="tree__scroll" style={{ contain: "none" }}>
        {tops.types.map((t) => (
          <button key={t.ext} className="trow" style={{ position: "relative", width: "100%", textAlign: "start" }} onClick={() => t.ext && setQuery(`ext:${t.ext}`)}>
            <span className="truncate">{t.ext ? `.${t.ext}` : "No extension"} <span className="data muted">{t.class}</span></span>
            <span className="num">{fmtBytes(t.alloc)}</span>
            <span className="num num--3">{t.files > 9999 ? `${Math.round(t.files / 1000)}k` : t.files}</span>
          </button>
        ))}
      </div>
    );
  }
  const list = tab === "files" ? tops.files : tops.folders;
  return (
    <>
      {tab === "folders" && <p className="data muted" style={{ padding: "6px 12px", borderBlockEnd: "1px solid var(--line)" }}>Ranked by files directly inside, so nothing counts twice.</p>}
      <div className="tree__scroll" style={{ contain: "none" }} onMouseLeave={() => onHover(null)}>
        {list.map((e) => (
          <button key={e.id} title={e.path} className={`trow ${selected === e.id ? "trow--sel" : ""}`} style={{ position: "relative", width: "100%", textAlign: "start", height: "auto", padding: "5px 12px" }} onClick={() => onSelect(e.id)} onMouseEnter={() => onHover(e.id)}>
            <span className="truncate">{e.name}<br /><span className="data muted">{midTrim(e.path, 44)}</span></span>
            <span className="num">{fmtBytes(e.alloc)}</span>
            <span />
          </button>
        ))}
      </div>
    </>
  );
}

function Breadcrumb({ focus, version, onGo }: { focus: number; version: number; onGo: (i: number) => void }) {
  const [path, setPath] = useState<[number, string][]>([]);
  useEffect(() => { if (version) api.details(focus).then((d) => setPath(d.ancestors)).catch(() => setPath([])); }, [focus, version]);
  return (
    <nav className="crumbs" aria-label="Location">
      {path.map(([id, n], i) => (
        <span key={id} style={{ display: "contents" }}>
          {i > 0 && <span className="muted data">/</span>}
          <button className="crumb" onClick={() => onGo(id)} aria-current={i === path.length - 1 ? "location" : undefined} key={i === path.length - 1 ? `last-${id}` : id}>{n.replace(/\\$/, "")}</button>
        </span>
      ))}
    </nav>
  );
}

function MapStage({ mode, prefs, version, focus, setFocus, selected, hovered, onSelect, onHover, lens, matches, doneAt, running }: {
  mode: "map" | "strata"; prefs: Prefs; version: number; focus: number; setFocus: (i: number) => void; selected: number | null; hovered: number | null;
  onSelect: (i: number) => void; onHover: (i: number | null) => void; lens: Lens; matches: Set<number> | null; doneAt: number; running: boolean;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const glCanvas = useRef<HTMLCanvasElement>(null);
  const labelCanvas = useRef<HTMLCanvasElement>(null);
  const mapRef = useRef<MapRenderer | null>(null);
  const [fail, setFail] = useState<string | null>(null);
  const [tip, setTip] = useState<{ x: number; y: number; h: Hover; small: boolean } | null>(null);
  const [desc, setDesc] = useState("");
  const [arrival, setArrival] = useState<{ total: number; key: number } | null>(null);
  const labels = useRef<Map<number, string>>(new Map());
  const prevFocus = useRef(focus);
  const seq = useRef(0);
  const zoomRect = useRef<{ x: number; y: number; w: number; h: number } | null>(null);

  const drawLabels = useCallback(() => {
    const m = mapRef.current, c = labelCanvas.current;
    if (!m || !c) return;
    const dpr = window.devicePixelRatio || 1;
    if (c.width !== Math.round(m.width * dpr) || c.height !== Math.round(m.height * dpr)) { c.width = Math.round(m.width * dpr); c.height = Math.round(m.height * dpr); }
    const g = c.getContext("2d")!;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, m.width, m.height);
    g.font = '11px "JetBrains Mono Variable", monospace';
    g.textBaseline = "top";
    const [cx, cy, sx, sy] = m.camera();
    for (let i = 0; i < m.count; i++) {
      const r = m.inst(i);
      if (r.kind & KIND_SMALL) continue;
      const w = r.w * sx, h = r.h * sy;
      if (w < 44 || h < 16) continue;
      const name = labels.current.get(r.id);
      if (!name) continue;
      const isDir = (r.kind & KIND_DIR) !== 0;
      const x = (r.x - cx) * sx, y = (r.y - cy) * sy;
      if (isDir && mode === "map" && h <= 40) continue;
      const maxChars = Math.floor((w - 8) / 6.65);
      const text = name.length > maxChars ? `${name.slice(0, Math.max(1, maxChars - 1))}…` : name;
      g.fillStyle = isDir ? "#8a8a8a" : "#f2f2f2";
      g.fillText(text, x + 4, y + 3);
    }
  }, [mode]);

  const requestLabels = useCallback(async (m: MapRenderer, s: number) => {
    const want: { id: number; a: number }[] = [];
    for (let i = 0; i < m.count; i++) {
      const r = m.inst(i);
      if (r.kind & KIND_SMALL || r.w < 44 || r.h < 16) continue;
      want.push({ id: r.id, a: r.w * r.h });
    }
    want.sort((a, b) => b.a - a.a);
    const ids = want.slice(0, 600).map((x) => x.id);
    const names = await api.labels(ids).catch(() => []);
    if (s !== seq.current) return;
    const next = new Map<number, string>();
    ids.forEach((id, i) => { if (names[i]?.[0]) next.set(id, names[i][0].replace(/\\$/, "")); });
    labels.current = next;
    drawLabels();
  }, [drawLabels]);

  const reload = useCallback(async (after?: (m: MapRenderer) => void) => {
    const m = mapRef.current;
    if (!m || !version) return;
    const s = ++seq.current;
    const buf = await api.layout(focus, m.width, m.height, mode === "strata" ? "strata" : "map", prefs.basis === "logical").catch(() => null);
    if (!buf || s !== seq.current) return;
    m.setData(buf);
    after?.(m);
    requestLabels(m, s);
  }, [focus, mode, version, prefs.basis, requestLabels]);

  useEffect(() => {
    if (!glCanvas.current || !stage.current) return;
    let m: MapRenderer;
    try { m = new MapRenderer(glCanvas.current); } catch (e) { setFail(String(e)); return; }
    mapRef.current = m;
    m.onFrame = drawLabels;
    (window as unknown as { __ozMap?: MapRenderer }).__ozMap = m;
    const el = stage.current;
    let t = 0;
    const ro = new ResizeObserver(() => {
      m.resize(el.clientWidth, el.clientHeight);
      clearTimeout(t);
      t = window.setTimeout(() => reloadRef.current(), 60);
    });
    ro.observe(el);
    return () => { ro.disconnect(); clearTimeout(t); m.destroy(); mapRef.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const reloadRef = useRef(reload);
  reloadRef.current = reload;

  useEffect(() => { if (mapRef.current) mapRef.current.onFrame = drawLabels; }, [drawLabels]);

  // Focus changes fly the camera; other changes just refresh the layout.
  useEffect(() => {
    const m = mapRef.current;
    if (!m) return;
    const old = prevFocus.current;
    prevFocus.current = focus;
    if (old === focus || mode !== "map") { m.resetCam(); reload(); return; }
    const ms = motionMs(prefs, 300);
    const idx = m.findById(focus);
    if (idx >= 0) {
      // Zooming in: fly into the existing rectangle, then swap in detail.
      labels.current = new Map();
      m.flyTo(m.inst(idx), ms, () => { m.resetCam(); reload(); });
    } else {
      // Zooming out: load the parent, then start from where the child sits in it.
      reload((mm) => {
        const j = mm.findById(old);
        if (j >= 0 && ms > 0) mm.flyFrom(mm.inst(j), ms); else mm.resetCam();
      });
    }
    zoomRect.current = null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus]);

  useEffect(() => { mapRef.current?.resetCam(); reload(); /* new data */ }, [version, mode, prefs.basis]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => { mapRef.current?.setSelected(selected); }, [selected]);
  useEffect(() => { mapRef.current?.setHover(hovered); }, [hovered]);
  useEffect(() => { mapRef.current?.setLens(lens); }, [lens]);
  useEffect(() => { mapRef.current?.setMatches(matches); }, [matches]);

  // The arrival moment reflects a scan genuinely finishing.
  useEffect(() => {
    if (!doneAt || doneAt === arrivalShownFor || running) return;
    arrivalShownFor = doneAt;
    if (prefs.motion === "off") return;
    mapRef.current?.playReveal(motionMs(prefs, 700));
    api.hover(0).then((h) => setArrival({ total: h.alloc, key: doneAt })).catch(() => {});
    const t = setTimeout(() => setArrival(null), 3000);
    return () => clearTimeout(t);
  }, [doneAt, running, prefs]);

  const hoverReq = useRef(0);
  const pos = (e: React.MouseEvent) => { const r = stage.current!.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top] as const; };
  const onMove = (e: React.MouseEvent) => {
    const m = mapRef.current;
    if (!m) return;
    const [x, y] = pos(e);
    const i = m.pick(x, y);
    if (i < 0) { onHover(null); setTip(null); return; }
    const r = m.inst(i);
    onHover(r.kind & KIND_SMALL ? null : r.id);
    const req = ++hoverReq.current;
    const cx = e.clientX, cy = e.clientY;
    api.hover(r.id).then((h) => { if (req === hoverReq.current) setTip({ x: cx, y: cy, h, small: !!(r.kind & KIND_SMALL) }); }).catch(() => {});
  };
  const onClick = (e: React.MouseEvent) => {
    const m = mapRef.current;
    if (!m) return;
    const [x, y] = pos(e);
    const i = m.pick(x, y);
    if (i >= 0) { const r = m.inst(i); onSelect(r.id); stage.current?.focus(); }
  };
  const onDouble = (e: React.MouseEvent) => {
    const m = mapRef.current;
    if (!m) return;
    const [x, y] = pos(e);
    // Double-click on a file zooms to the deepest folder holding it.
    const i = m.pick(x, y);
    if (i < 0) return;
    const r = m.inst(i);
    if (r.kind & KIND_DIR) setFocus(r.id);
    else api.details(r.id).then((d) => { if (d.parent !== focus) setFocus(d.parent); }).catch(() => {});
  };
  const up = () => { if (focus !== 0) api.details(focus).then((d) => setFocus(d.parent)).catch(() => {}); };

  // Keyboard: arrows walk to the nearest rectangle in that direction.
  const onKey = (e: React.KeyboardEvent) => {
    const m = mapRef.current;
    if (!m || !m.count) return;
    if (e.key === "Enter" && selected !== null) { e.preventDefault(); api.details(selected).then((d) => { if (d.row.kids) setFocus(selected); }).catch(() => {}); return; }
    if (e.key === "Escape" || e.key === "Backspace") { e.preventDefault(); up(); return; }
    const dirs: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const d = dirs[e.key];
    if (!d) return;
    e.preventDefault();
    const cur = selected !== null ? m.findById(selected) : -1;
    if (cur < 0) { for (let i = 0; i < m.count; i++) { const r = m.inst(i); if (!(r.kind & KIND_SMALL)) { onSelect(r.id); break; } } return; }
    const a = m.inst(cur), ax = a.x + a.w / 2, ay = a.y + a.h / 2;
    let best = -1, score = Infinity;
    for (let i = 0; i < m.count; i++) {
      const r = m.inst(i);
      if (i === cur || r.kind & KIND_SMALL || r.depth !== a.depth) continue;
      const dx = r.x + r.w / 2 - ax, dy = r.y + r.h / 2 - ay, along = dx * d[0] + dy * d[1];
      if (along <= 0) continue;
      const s = along + Math.abs(dx * d[1] + dy * d[0]) * 2;
      if (s < score) { score = s; best = i; }
    }
    if (best >= 0) onSelect(m.inst(best).id);
  };

  useEffect(() => {
    if (selected === null) { setDesc(""); return; }
    api.hover(selected).then((h) => setDesc(`${h.name}, ${fmtBytes(h.alloc)}`)).catch(() => {});
  }, [selected]);

  if (fail) return <div className="viz__stage"><div className="empty"><p className="sub">The map needs WebGL2, which isn't available. The list on the left still shows everything.</p></div></div>;

  return (
    <div className="viz__stage" ref={stage} tabIndex={0} role="application" aria-label="Storage map. Arrow keys move, Enter zooms in, Escape zooms out." onKeyDown={onKey}
      onMouseMove={onMove} onMouseLeave={() => { onHover(null); setTip(null); }} onClick={onClick} onDoubleClick={onDouble}>
      <canvas ref={glCanvas} />
      <canvas ref={labelCanvas} className="viz__labels" />
      <span className="sr" aria-live="polite">{desc}</span>
      {!version && <div className="empty"><p className="label">Reading the drive</p></div>}
      {version > 0 && !running && mapRef.current?.count === 0 && <div className="empty"><p className="sub">This folder is empty.</p></div>}
      {lens === "type" && <div className="legend">{TYPE_NAMES.map((n, i) => <span key={n} className="label"><i style={{ background: `rgb(${LENS_LEGEND[i].map((c) => Math.round(c * 255)).join(",")})` }} />{n}</span>)}</div>}
      {lens === "age" && <div className="legend"><span className="label"><i style={{ background: "rgb(117,117,117)" }} />Recent</span><span className="label"><i style={{ background: "rgb(13,13,13)", outline: "1px solid var(--text-4)" }} />5+ years</span></div>}
      {lens === "reclaim" && <div className="legend"><span className="label"><i style={{ background: "var(--signal)" }} />Matches a cleanup rule</span></div>}
      {arrival && (
        <div className="arrival" key={arrival.key}>
          <div className="display-xl" style={{ fontSize: "clamp(48px, 6vw, 88px)" }}><span className="rise"><span>{fmtBytes(arrival.total)}</span></span></div>
          <hr className="rule rule--draw" style={{ background: "var(--text-1)", margin: "12px 0 8px" }} />
          <span className="label">Mapped</span>
        </div>
      )}
      {tip && (
        <div className="tip" style={{ left: Math.min(tip.x + 14, window.innerWidth - 430), top: Math.min(tip.y + 14, window.innerHeight - 90) }}>
          {tip.small ? <div className="tip__name">Small items in {tip.h.name.replace(/\\$/, "")}</div> : <div className="tip__name">{tip.h.name}</div>}
          <div className="data muted">{fmtBytes(tip.h.alloc)}{tip.h.dir ? ` · ${fmtCount(tip.h.files)} files` : ""} · {fmtDate(tip.h.modified)}</div>
        </div>
      )}
    </div>
  );
}

function TimelineView({ version, setQuery }: { version: number; setQuery: (q: string) => void }) {
  const [weeks, setWeeks] = useState<Week[]>([]);
  const [on, setOn] = useState<number | null>(null);
  const [tip, setTip] = useState<Week | null>(null);
  useEffect(() => { if (version) api.timeline(53 * 5).then(setWeeks).catch(() => {}); }, [version]);
  const max = useMemo(() => Math.max(1, ...weeks.slice(1).map((w) => w.bytes)), [weeks]);
  const rows = useMemo(() => {
    const r: Week[][] = [];
    for (let i = 0; i < weeks.length; i += 53) r.push(weeks.slice(i, i + 53));
    return r;
  }, [weeks]);
  const shade = (w: Week) => {
    if (!w.bytes) return "#111";
    const t = Math.min(1, Math.log(1 + w.bytes) / Math.log(1 + max));
    const v = Math.round(40 + t * t * 215);
    return `rgb(${v},${v},${v})`;
  };
  const WEEK = 7 * 864000000000;
  return (
    <div className="timeline" style={{ flex: 1 }}>
      <div className="between" style={{ marginBlockEnd: 16 }}>
        <h2 className="title">When your data last changed</h2>
        <span className="data muted">{tip ? `${fmtDate(tip.start)} · ${fmtBytes(tip.bytes)} · ${fmtCount(tip.files)} files` : "Each square is a week. Click to filter."}</span>
      </div>
      <div className="tl-grid">
        {rows.map((r, ri) => (
          <div key={ri} style={{ display: "contents" }}>
            <span className="label" style={{ fontSize: 10, letterSpacing: ".06em" }}>{r[0] ? filetimeToDate(r[0].start).getUTCFullYear() : ""}</span>
            {r.map((w, i) => (
              <button key={i} className={`tl ${on === w.start ? "tl--on" : ""}`} style={{ background: shade(w) }}
                aria-label={`Week of ${fmtDate(w.start)}: ${fmtBytes(w.bytes)}`} onMouseEnter={() => setTip(w)} onMouseLeave={() => setTip(null)} onFocus={() => setTip(w)}
                onClick={() => { setOn(w.start); setQuery(`from:${ri === 0 && i === 0 ? 0 : w.start} to:${w.start + WEEK}`); }} />
            ))}
          </div>
        ))}
      </div>
      <p className="data muted" style={{ marginBlockStart: 16 }}>The first square also holds everything older than five years.</p>
    </div>
  );
}

function DetailPanel({ d, onClose, onZoom, onAdd, draftCount, onGoReclaim }: { d: Details; onClose: () => void; onZoom: () => void; onAdd: (x: ReclaimDraft) => void; draftCount: number; onGoReclaim: () => void }) {
  const [copied, setCopied] = useState(false);
  const r = d.row;
  const copy = () => { navigator.clipboard.writeText(d.path).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1000); }).catch(() => {}); };
  const isDir = (r.flags & FLAG.DIR) !== 0;
  const attrs = [
    r.flags & FLAG.LINK ? "Link, not followed" : null,
    r.flags & FLAG.HARDLINK ? "Hard link, counted once" : null,
    r.flags & FLAG.PLACEHOLDER ? "Cloud placeholder" : null,
    r.flags & FLAG.ADS ? "Has extra streams" : null,
    r.flags & FLAG.SYSTEM ? "System" : null,
    r.flags & FLAG.UNREAD ? "Could not be read" : null,
  ].filter(Boolean) as string[];
  return (
    <aside className="detail" aria-label="Details">
      <div className="between">
        <span className="label">{isDir ? "Folder" : "File"}</span>
        <button className="link" onClick={onClose}>Hide</button>
      </div>
      <h2 className="title selectable" style={{ wordBreak: "break-word" }}>{r.name.replace(/\\$/, "") || d.path}</h2>
      <p className="display" style={{ fontSize: 32 }}>{fmtBytes(r.alloc)}</p>
      <div className="pathbox">{d.path}</div>
      <div className="row wrap-row">
        <button className="link" onClick={copy}>{copied ? "Copied" : "Copy path"}</button>
        <button className="link" onClick={() => api.reveal(d.path)}>Reveal ↗</button>
        <button className="link" onClick={onZoom}>Zoom</button>
      </div>
      <hr className="rule" />
      <dl className="kv">
        <dt>On disk</dt><dd>{fmtBytes(r.alloc)}</dd>
        <dt>Logical</dt><dd>{fmtBytes(r.logical)}</dd>
        {isDir && <><dt>Files</dt><dd>{fmtCount(r.files)}</dd></>}
        <dt>Share</dt><dd>{r.pct.toFixed(1)}%</dd>
        <dt>{isDir ? "Newest" : "Modified"}</dt><dd>{fmtDate(r.modified)}</dd>
      </dl>
      {attrs.length > 0 && <div className="row wrap-row">{attrs.map((a) => <span className="tag" key={a}>{a}</span>)}</div>}
      <hr className="rule" />
      <div className="gap-2">
        <span className="label">What is this?</span>
        {d.rule ? (
          <>
            <p style={{ fontWeight: 600 }}>{d.rule.name}</p>
            <p className="sub">{d.rule.explain}</p>
            {d.rule.after && <p className="muted">{d.rule.after}</p>}
            <span><span className={`tag ${d.rule.tier === "Protected" ? "" : ""}`}>{d.rule.tier}</span></span>
            {d.rule.settings_uri && <button className="link" style={{ textAlign: "start" }} onClick={() => api.openSettings(d.rule!.settings_uri!)}>Open Windows storage settings ↗</button>}
          </>
        ) : d.guess ? (
          <p className="sub">Not recognised. It may belong to <em>{d.guess}</em>, going by the folder name. That's a guess.</p>
        ) : <p className="sub">Not recognised.</p>}
      </div>
      <div style={{ marginBlockStart: "auto" }} className="gap-2">
        {d.deletable ? (
          <p className="muted">Can't be added to Reclaim: {d.deletable.toLowerCase()}.</p>
        ) : r.flags & (FLAG.LINK | FLAG.PLACEHOLDER) ? (
          <p className="muted">Links and cloud placeholders can't be added to Reclaim.</p>
        ) : (
          <button className="btn btn--ghost" onClick={() => onAdd({ path: d.path, name: r.name, bytes: r.alloc, modified: isDir ? null : r.modified })}>Add to Reclaim</button>
        )}
        {draftCount > 0 && <button className="link" style={{ textAlign: "start" }} onClick={onGoReclaim}>{draftCount} in Reclaim ↗</button>}
      </div>
    </aside>
  );
}

