import { useCallback, useEffect, useMemo, useState } from "react";
import { api, TweakInfo } from "./api";

const CATS = ["All", "Performance", "Privacy", "Explorer", "Taskbar", "Storage", "Accessibility"] as const;
const RESTART: Record<TweakInfo["restart"], string> = { none: "", explorer: "Takes effect when File Explorer restarts", signout: "Takes effect after sign-out", reboot: "Takes effect after restart" };

function stateText(t: TweakInfo): string {
  if (t.state === "on") return "On";
  if (t.state === "default") return "Windows default";
  if (t.state === "unknown") return "Couldn’t read";
  return "Off";
}

export function TweaksScreen({ onBack }: { onBack: () => void }) {
  const [list, setList] = useState<TweakInfo[] | null>(null);
  const [cat, setCat] = useState<(typeof CATS)[number]>("All");
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const load = useCallback(() => { api.tweaks().then(setList).catch((e) => { setMsg(String(e)); setList([]); }); }, []);
  useEffect(load, [load]);

  const shown = useMemo(() => (list ?? []).filter((t) => cat === "All" || t.category === cat), [list, cat]);
  const on = (list ?? []).filter((t) => t.state === "on").length;
  const open = (list ?? []).filter((t) => !t.locked).length;

  const act = async (t: TweakInfo, undo: boolean) => {
    setBusy(t.id); setMsg(null);
    // Optimistic: show the new state at once, then confirm from a fresh read.
    setList((l) => l?.map((x) => x.id === t.id ? { ...x, state: undo ? "off" : "on", applied: !undo } : x) ?? null);
    try { await (undo ? api.tweakRevert(t.id) : api.tweakApply(t.id)); } catch (e) { setMsg(String(e)); }
    setBusy(null); load();
  };

  return (
    <div className="page">
      <div className="page__inner gap-4">
        <div><button className="btn btn--ghost btn--sm" onClick={onBack}>← Tools</button></div>
        <div className="between wrap-row" style={{ alignItems: "flex-end" }}>
          <h1 className="display">Tweaks</h1>
          <div className="chips" role="tablist" aria-label="Category">
            {CATS.map((c) => <button key={c} role="tab" aria-selected={cat === c} className={`chip ${cat === c ? "chip--on" : ""}`} onClick={() => setCat(c)}>{c}</button>)}
          </div>
        </div>
        <div className="stats">
          <div className="stat"><div className="stat__n">{list ? on : "—"}</div><span className="label">Already set</span></div>
          <div className="stat"><div className="stat__n">{list ? list.length : "—"}</div><span className="label">In the catalogue</span></div>
          <div className="stat"><div className="stat__n">{list ? open : "—"}</div><span className="label">Ready to apply</span></div>
        </div>
        <p className="sub">Every tweak shows its live state on this PC. Oz saves the previous value before changing anything, so Undo puts back exactly what was there. Security and Windows Update settings are never touched.</p>
        {list && <p className="muted">Machine-wide tweaks stay locked until they’re tested in Windows Sandbox and the elevated helper can apply them.</p>}
        {msg && <p className="note">{msg}</p>}
        <div className="tweaks stagger" key={cat}>
          {list === null && [0, 1, 2, 3, 4].map((i) => <div key={i} className="skel" style={{ height: 72, "--i": i } as React.CSSProperties} />)}
          {shown.map((t, i) => (
            <div key={t.id} className="tweak" style={{ "--i": i } as React.CSSProperties}>
              <div className="tweak__text">
                <span className="tweak__name">{t.name}{t.tier === "Recommended" && <span className="tag" style={{ marginInlineStart: 10 }}>Recommended</span>}</span>
                <span className="sub">{t.explain}</span>
                <span className="muted tweak__meta">{[t.category, RESTART[t.restart], t.needs_admin ? "Needs admin" : "", t.locked ?? ""].filter(Boolean).join(" · ")}</span>
              </div>
              <div className="tweak__side">
                <span className={`tweak__state ${t.state === "on" ? "tweak__state--on" : ""}`} title={`Current value: ${t.current}`}>{stateText(t)}</span>
                {t.applied
                  ? <button className="btn btn--ghost btn--sm" disabled={busy === t.id} onClick={() => act(t, true)}>Undo</button>
                  : t.state === "on" ? <span style={{ width: 72 }} aria-hidden="true" />
                  : <button className="btn btn--primary btn--sm" style={{ minWidth: 72 }} disabled={!!t.locked || busy === t.id} title={t.locked ?? undefined} onClick={() => act(t, false)}>Apply</button>}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
