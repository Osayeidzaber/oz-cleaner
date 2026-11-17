# Oz Cleaner: Development Roadmap

**Current state (2026-11-17): code for M1-M6 exists in the working tree but no exit check is recorded as run, and nothing is committed. Scope widened to add Debloat (M6b, D-14). UI redesign in progress on a Claude Design canvas. Next: audit the existing code against exit checks.** Keep this line current.

Status values: `TODO`, `IN PROGRESS`, `DONE`. A milestone is DONE only when every exit check has actually been run. No calendar estimates: they would be invented. Order is what matters.

## Sequencing logic

1. Prove the risky things first (M0), before any product code.
2. Read-only product before anything that can delete (M1-M4). The app is valuable and harmless at M4.
3. Fast scan after the walker: the walker provides the test oracle and the fallback.
4. Design system (M1) precedes screens, so no screen is ever built ugly "for now".

## M0: Foundations and spikes (TODO)

- Install Rust stable and the MSVC toolchain. `git init`, private repository, `.gitignore`.
- Scaffold the Cargo workspace and Tauri app per `03`, section 3. Pin dependencies.
- Run spikes S1, S2, S3, S4, S5, S7. Record results and update `03` labels.
- Confirm font licences; bundle fonts.
- Build `tools/make-fixtures`: creates a test tree with long paths (over 260, over 1000), hard links, junctions, symlinks, ADS, sparse and compressed files, read-only files, Unicode names, and 100,000 files in one folder.

**Exit:** spike results recorded; `oz-cleaner.exe` opens an empty black window; decisions changed by spikes are recorded.

## M1: Design system, shell, Standard scan (TODO)

- E.1 tokens, fonts, shell, nav, buttons, table row, motion utilities; a hidden `/styleguide` screen.
- `oz-model`, `oz-win` walker, Channels. 00 Volumes (F-01). Scan with progress (F-02 Standard, F-03). Correct accounting in Standard mode (F-04).
- Plain tree table reading from the index.

**Exit:** Scans owner's four drives with no elevation. Fixture totals correct. Owner reviews look and feel of 00 Volumes and sign-off is recorded. No writes anywhere.

## M2: Fast scan (TODO)

- `oz-ntfs`, helper, pipe protocol, UAC flow (F-02), differential tests against walker and `mft` crate.
- Unaccounted bar (F-07). BitLocker check if a volume is available.
- Optional cross-check: compare with a WizTree run by the owner (totals only, no reverse engineering).

**Exit:** S1 targets met or relaxed with reasons. Malformed-record fuzz test passes without panic. UAC decline path clean. Helper security rules from D-03 tested.

## M3: Atlas (TODO)

- `oz-layout`; WebGL2 map (F-11) with zoom, hover, labels, keyboard; label overlay.
- Three-pane Atlas with linked selection (F-05, F-06, F-08). Scan-as-reveal signature moment.

**Exit:** 60 fps on owner's machine in worst real folder, measured with a frame log. Owner sign-off: "feels as immediate as WizTree".

## M4: Full analysis; first usable release (TODO)

- Search and filters (F-09), Top lists (F-10), Strata (F-12), Timeline (F-13), Lenses, Settings for what exists, all E.5 states, accessibility pass (E.6).
- Internal release `0.1 Analyzer`, read-only.

**Exit:** Owner uses it instead of WizTree for a week and logs issues. Keyboard-only test passes. Screen reader pass on tree table with NVDA.

## M5: Understanding and cleaning (TODO)

Highest-risk milestone. Every step on fixtures before a real path.

Order: rule engine and verified rules (F-30 to F-34) → read-only "what is this" ships first → Reclaim list (F-35) → pre-flight, protected roots, and `oz-ops` tests → Ledger (F-39) → Holding Area (F-41) → Recycle (F-40) → permanent deletion for Rebuildable → confirm and results UI (F-36 to F-38) → elevated plan execution.

**Exit:**
- A test suite proves: protected roots are refused in core and helper; reparse points aren't traversed; long paths delete fully; changed files are skipped; locked files are reported; placeholders aren't touched; other users' profiles are refused.
- Dry-run mode shows the exact plan and matches actual execution on fixtures.
- Owner runs it on real caches with `freed` measured and reconciled.

## M6: Relocate and uninstall helper (TODO)

F-42 with rollback tests at every step, including simulated copy failure and full target drive. F-43, confirming the Edge refusal case is reported rather than forced.

**Exit:** Round-trip relocate and undo on fixtures, with hash equality and long paths. One real low-value folder relocated by owner.

## M6b: Debloat (IN PROGRESS)

`oz-tweaks` catalog and loader (D-14), helper `ApplyTweak` / `RevertTweak`, Ledger prior-state records, restore point offer (spike S8), Debloat screen with per-tweak explain, Undo all.

Done 2026-11-17: `oz-tweaks` catalog (77 tweaks: performance, privacy, Explorer, taskbar, storage, accessibility), loader rejecting security and update keys (tested), `oz-win::tweak` registry and power-plan read and write, Tweaks screen with live state, apply and undo with saved prior values. Apply is locked for every tweak: `verified` is empty because Windows Sandbox isn't installed on the owner's machine. HKLM and power-setting tweaks also need helper operations, not built yet.

**Exit:** Every catalogued tweak passes apply, revert and state-equal in Windows Sandbox. The loader rejects security and update targets (test). No tweak is run on the owner's machine without their request in that moment.

## M7: Memory and refinement (TODO)

Snapshots and compare (F-14), export (F-15). Motion polish pass; performance pass. S6.

**Exit:** Compare view correct across two real scans. Full design review against `02` Part E and E.7.

## M8: Ship 1.0 (TODO)

Installer and portable build (D-11). Owner decides signing, licence and whether to make it public. Test on a clean Windows 11 VM: standard user, no dev tools, Windows 10 if supported. README, screenshots from the real app. Portfolio case study for zaber.tech at the owner's discretion.

**Exit:** Clean VM install and uninstall leaves no residue. Full manual test passes. Owner approves publishing.

## After 1.0

Live refresh (F-16), duplicates (F-17), threshold reminders (F-44), WebGPU evaluation.

## Testing strategy

| Level | What | When |
|---|---|---|
| Unit | Model, parser on byte fixtures, layout, rule matching | Every change |
| Differential | MFT vs walker vs `mft` crate on fixture volume | Parser or walker changes |
| Fixture volume | A VHDX created by the tool, mounted for tests, so destructive tests can't reach real data | All `oz-ops` work |
| Fuzz | Parser with corrupted records | M2 onward |
| UI | Component states, keyboard flows, frame timing logs | M3 onward |
| Manual | Owner checklists at each exit | Each milestone |

Destructive tests run only against the fixture VHDX or scratch folder.
