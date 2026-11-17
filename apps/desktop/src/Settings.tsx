import { useEffect, useState } from "react";
import { api } from "./api";

export interface Prefs {
  motion: "full" | "reduced" | "off";
  basis: "disk" | "logical";
  density: "comfortable" | "compact";
  history: boolean;
}

function Choice<T extends string>({ value, options, onChange, label }: { value: T; options: [T, string][]; onChange: (v: T) => void; label: string }) {
  return (
    <div className="chips" role="group" aria-label={label}>
      {options.map(([v, t]) => <button key={v} className={`chip ${value === v ? "chip--on" : ""}`} aria-pressed={value === v} onClick={() => onChange(v)}>{t}</button>)}
    </div>
  );
}

export function SettingsScreen({ prefs, setPrefs }: { prefs: Prefs; setPrefs: (p: Partial<Prefs>) => void }) {
  const [loc, setLoc] = useState("");
  const [cleared, setCleared] = useState(false);
  useEffect(() => { api.dataLocation().then(setLoc).catch(() => {}); }, []);
  return (
    <div className="page">
      <div className="page__inner">
        <h1 className="display" style={{ marginBlock: "16px 32px" }}>Settings</h1>
        <div className="setting">
          <div><h2 className="title">Motion</h2><p className="muted">Full, shorter fades, or none.</p></div>
          <Choice label="Motion" value={prefs.motion} onChange={(motion) => setPrefs({ motion })} options={[["full", "Full"], ["reduced", "Reduced"], ["off", "Off"]]} />
        </div>
        <div className="setting">
          <div><h2 className="title">Map size basis</h2><p className="muted">On disk shows what you would get back. Logical shows file length.</p></div>
          <Choice label="Size basis" value={prefs.basis} onChange={(basis) => setPrefs({ basis })} options={[["disk", "On disk"], ["logical", "Logical"]]} />
        </div>
        <div className="setting">
          <div><h2 className="title">List density</h2></div>
          <Choice label="Density" value={prefs.density} onChange={(density) => setPrefs({ density })} options={[["comfortable", "Comfortable"], ["compact", "Compact"]]} />
        </div>
        <div className="setting">
          <div><h2 className="title">Scan history</h2><p className="muted">After each full drive scan, save folder sizes on this PC so you can compare later.</p></div>
          <div className="row">
            <Choice label="History" value={prefs.history ? "on" : "off"} onChange={(v) => setPrefs({ history: v === "on" })} options={[["on", "On"], ["off", "Off"]]} />
            <button className="link" onClick={() => api.clearHistory().then(() => setCleared(true))}>{cleared ? "Cleared" : "Clear saved"}</button>
          </div>
        </div>
        <div className="setting">
          <div><h2 className="title">Your data</h2><p className="muted">Everything stays on this PC. Oz Cleaner makes no network requests.</p><p className="data muted selectable">{loc}</p></div>
          <button className="link" onClick={() => loc && api.reveal(loc)}>Reveal ↗</button>
        </div>
        <div className="setting">
          <div>
            <h2 className="title">About</h2>
            <p className="muted">Oz Cleaner 0.1.0, by Osayeed Zaber. All rights reserved.</p>
            <p className="muted">Fonts under the SIL Open Font License: Bricolage Grotesque, JetBrains Mono, Instrument Serif. Built with Tauri and React.</p>
          </div>
        </div>
      </div>
    </div>
  );
}
