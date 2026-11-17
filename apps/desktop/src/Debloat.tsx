import { useCallback, useEffect, useState } from "react";
import { api, BloatApp } from "./api";

export function DebloatScreen({ onBack }: { onBack: () => void }) {
  const [list, setList] = useState<BloatApp[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [removed, setRemoved] = useState<Set<string>>(new Set());
  const load = useCallback(() => { api.bloatList().then(setList).catch((e) => { setMsg(String(e)); setList([]); }); }, []);
  useEffect(load, [load]);

  const installed = (list ?? []).filter((a) => a.installed);
  const remove = async (a: BloatApp) => {
    setBusy(a.family); setMsg(null);
    try { await api.bloatRemove(a.family); setRemoved((s) => new Set(s).add(a.family)); }
    catch (e) { setMsg(`${a.name}: ${String(e)}`); }
    setBusy(null); load();
  };

  return (
    <div className="page">
      <div className="page__inner gap-4">
        <div><button className="btn btn--ghost btn--sm" onClick={onBack}>← Tools</button></div>
        <h1 className="display">Remove built-in apps</h1>
        <div className="stats">
          <div className="stat"><div className="stat__n">{list ? installed.length : "—"}</div><span className="label">Installed here</span></div>
          <div className="stat"><div className="stat__n">{list ? installed.filter((a) => a.recommended).length : "—"}</div><span className="label">Recommended to remove</span></div>
          <div className="stat"><div className="stat__n">{removed.size}</div><span className="label">Removed this session</span></div>
        </div>
        <p className="sub">Preinstalled Microsoft Store apps, removed for your account only. Every one can be reinstalled free from the Microsoft Store; Get it back opens its page. Apps Windows marks as non-removable, and apps that games or Windows features depend on, are never listed.</p>
        {msg && <p className="note">{msg}</p>}
        <div className="tweaks stagger">
          {list === null && [0, 1, 2, 3, 4].map((i) => <div key={i} className="skel" style={{ height: 64, "--i": i } as React.CSSProperties} />)}
          {list && installed.length === 0 && <p className="sub" style={{ paddingBlock: 24 }}>None of the apps Oz knows about are installed.</p>}
          {[...installed, ...(list ?? []).filter((a) => !a.installed && removed.has(a.family))].map((a, i) => (
            <div key={a.family} className="tweak" style={{ "--i": i } as React.CSSProperties}>
              <div className="tweak__text">
                <span className="tweak__name">{a.name}{a.recommended && <span className="tag" style={{ marginInlineStart: 10 }}>Recommended</span>}</span>
                <span className="sub">{a.explain}</span>
              </div>
              <div className="tweak__side">
                {a.installed
                  ? <button className="btn btn--primary btn--sm" style={{ minWidth: 88 }} disabled={!a.removable || busy !== null} title={a.removable ? undefined : "Windows marks this app as non-removable"} onClick={() => remove(a)}>{busy === a.family ? "Removing…" : "Remove"}</button>
                  : <button className="btn btn--ghost btn--sm" onClick={() => api.openStore(a.family).catch((e) => setMsg(String(e)))}>Get it back</button>}
              </div>
            </div>
          ))}
        </div>
        {list && <p className="muted">Not installed here: {(list ?? []).filter((a) => !a.installed && !removed.has(a.family)).map((a) => a.name).join(", ") || "none"}.</p>}
      </div>
    </div>
  );
}
