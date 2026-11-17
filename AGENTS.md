# AGENTS.md: Permanent Instructions for Oz Cleaner

Read this first, always. The repository is the source of truth, not any conversation. This project is unrelated to `E:\code\shopify`; none of that project's rules apply here.

## 1. What this is

Oz Cleaner is a Windows storage and system toolbox by owner Osayeed Zaber. Three jobs, one app:

1. **Explore**: see what fills a drive, fast (MFT scan, treemap), in the spirit of WizTree.
2. **Clean**: find temp files, caches and leftovers (including AI and dev tools such as Claude Code, Codex, ChatGPT, npm, pip) and say plainly which are safe to delete.
3. **Debloat**: remove preinstalled Windows apps and switch off unwanted Windows features, every change reversible.

It is **not** an app installer. It does not download or install software, and it never activates or licenses Windows or Office.

Identity: the design language of zaber.tech. Pure black, monochrome, editorial, precise. Stack: Tauri 2, Rust core, React and TypeScript UI, hand-written WebGL2 treemap, separate elevated helper.

## 2. Read order

1. `AGENTS.md` (this file)
2. `04-ROADMAP.md`: current state and milestone
3. `01-PRODUCT-VISION.md`: why, principles, observed design language (section 8)
4. `02-FEATURE-SPEC.md`: what, with F-IDs and interface tokens
5. `03-ARCHITECTURE.md`: how, with D-decisions, A-rules and spikes

If documents conflict: safety rules here win, then `03`, then `02`, then `01`. Report the conflict to the owner and fix it.

## 3. Absolute safety rules

Breaking any of these is the worst possible failure of this project.

1. **No destructive experiments on real user data.** Delete, move, rename and tweak tests run only on the fixture VHDX, a scratch folder the test creates, or a disposable VM or Windows Sandbox.
2. **Never run cleaning or debloat against the owner's real system** unless they ask in that moment.
3. **Never weaken** protected roots, pre-flight checks, or the no-silent-fallback rule (A-7) to make something pass.
4. **Never write to a raw volume handle.** The MFT reader is read-only.
5. **Personal files are never recommended for deletion** because of their name, age or type. AI tool chats, settings, memory and credentials are personal data and stay **Protected**.
6. **Do not force.** No taking ownership, no ACL changes, no killing processes, no forced uninstalls, no removing apps Windows marks non-removable.
7. **Never elevate the UI process.** The helper accepts only fixed, named operations (an enum), never a command string, script or arbitrary registry path.
8. **No network calls or telemetry.** File paths are private data; keep them out of default-level logs and anything that leaves the machine.
9. **The registry is read-only, except for catalogued tweaks** (section 6) **and an uninstalled app's own leftover keys** (section 6b). Each tweak writes only the exact keys and values listed in its catalog entry. The previous value, or its absence, is saved to the Ledger before writing, and every tweak has a tested revert.
10. **Never touch security or updates.** No tweak may disable or weaken Defender, SmartScreen, Firewall, UAC, Windows Update, BitLocker, Secure Boot, or system restore.
11. **No licensing circumvention.** No activation scripts (for example MAS, `get.activated.win`), KMS emulators, or key changes. No `irm … | iex` or any downloaded script, ever.

## 4. Honesty rules

- Never invent benchmark results, test outcomes, screenshots, sizes, or API behaviour. Report only what you ran.
- Keep the evidence labels in `03` truthful: `[DOC]`, `[MEASURED]`, `[KNOWN]`, `[TARGET]`, `[UNVERIFIED]`. A target is not a result.
- Verify Windows behaviour in Microsoft's docs or by test. Forum claims and "debloat script" repos don't count as evidence; they are only leads.
- The UI must not imply certainty the app lacks: "estimated", "likely", "not recognised".
- No fake progress, delay, or data. Every number on screen comes from a real measurement. Mockups use `[placeholder]` labels, not invented figures.

## 5. Cleaning rule gate (F-30)

A cleaning rule may be added only when you have:
- confirmed that the path exists and holds what it claims;
- confirmed what regenerates it and what losing it costs, from vendor docs where available or a controlled test otherwise;
- filled in `verified` (date, Windows build, method);
- added a test.

If unsure, make it Protected (explain only). For AI and dev tools, split each tool's folder into its cache, which can be cleanable, and its history, config and auth, which stay Protected. Popularity on cleanup blogs isn't evidence.

## 6. Debloat tweak gate

A tweak (appx removal, feature toggle, registry value, scheduled task, service start type) may be added only when:
- the change is made through a documented Microsoft mechanism: `Remove-AppxPackage` for the current user, the DISM or optional features API, a documented policy or registry value, Task Scheduler, or the Service Control Manager;
- its catalog entry lists the exact targets, the apply action, the revert action, the Windows builds tested, and `verified`;
- apply, then revert, then check that the state matches the original has been run in a VM or Sandbox and passes as an automated test;
- it is tiered like cleaning rules: **Recommended**, **Optional**, or **Explain only**.

Before applying any batch, the app offers a System Restore point and records every prior state in the Ledger. One-click "Undo all" must exist for every batch. No tweak bundles ("apply everything") without showing each change.

## 6b. Full uninstall (A to Z)

Uninstall removes an app and everything it left behind:
1. Run the app's own uninstaller first (never forced; if it fails, stop and say so).
2. Then search for leftovers: its install folder, `AppData` (Roaming, Local, LocalLow), `ProgramData`, Start menu and desktop shortcuts, its uninstall and `Software\<Publisher>\<App>` registry keys, scheduled tasks, and services whose binary was inside the install folder.
3. Every leftover shows **why it matched**, with a confidence level:
   - **Certain**: the path is inside the recorded install location, or the uninstall record names it.
   - **Likely**: a publisher-and-app name match.
   - **Name only**: a name match alone. These are never preselected.

Folders shared by several apps of one publisher are flagged and left unticked if another installed app uses them. Documents, Pictures, Desktop, saves and other user data are **Keep** and never preselected. Files go to the Holding Area. Registry keys are exported to the Ledger before removal, so all of it can be undone. Nothing under `Windows`, `System32`, `WinSxS` or another app's install folder is ever proposed.

## 7. Design rules

Before UI work, read `01` section 8 and `02` Part E. Then:

- **zaber.tech, adapted for a tool.** Pure black. Greyscale text ramp `#f2f2f2 / #bdbdbd / #8a8a8a / #3a3a3a`. Hairlines `#1f1f1f` or white at 20%.
- **Editorial structure.** Mono masthead `OZ — CLEANER`, numbered mono section labels (`01 — CLEAN`), metadata in opposing corners, stat rows split by hairlines with big numerals over tiny mono captions.
- **Type with fixed roles.** Bricolage Grotesque for words (heavy, tight tracking at display sizes). JetBrains Mono only where it's needed: paths, small uppercase labels at 0.14em, and dense numeric tables. Contrast comes from weight (200 against 800), not from a third family. All numbers use tabular figures.
- **Components.** A floating pill nav whose active item inverts to white. Primary action is a white pill with ↗. Secondary is an outlined pill. Radii: 4 px cells, 12 px panels, full pills on controls.
- **Colour.** **Strict monochrome. No accent colour.** The primary action is an inverted white pill with black text. The only hues are for risk: `--signal` yellow for caution, `--danger` red for irreversible actions. Lens colours appear only inside a lens. Instrument Serif italic is retired.
- **Motion.** Use the token curves. Animate only transform, opacity and uniforms. Motion is interruptible and honours the Motion setting (Full / Reduced / Off).
- **Tokens only** in components: no raw colours, sizes or durations. No component libraries. No unstyled browser controls. None of the E.7 forbidden patterns.
- **Not WizTree.** Don't copy WizTree's name, icon, colours or layout, and don't decompile it. Not Chris Titus's utility either; borrow the idea of a toolbox, not its look.
- **A tool, not a portfolio.** No marquees, long scrolling or ambient effects on working screens. Big type only at moments of arrival (empty states, a finished scan, space freed).
- UI changes must be seen running before being called finished. Check at least 720x500 and 1920x1080, with Motion Off and Full.
- Quality test: if it could be any SaaS dashboard, redo it. The owner signs off at milestone exits in `04`.

## 8. Engineering rules

- Stay inside the stack in `03`. Changing it, or adding a dependency not in D-13 or the UI list, needs a recorded decision.
- Crate boundaries from `03` section 3. All FFI in `oz-win`; `unsafe` nowhere else. Tweak catalog and logic live in a pure crate (`oz-tweaks`); its Windows calls go through `oz-win`. The parser never panics on input.
- Rust: `cargo fmt`, `cargo clippy -D warnings`, no `unwrap` on external data. TypeScript: strict mode, no `any` without a comment.
- Long paths, Unicode, junctions and hard links are normal cases. Any file code needs fixtures for them.
- UI thread budget is 16 ms. Bulk data stays in Rust (D-05, D-06, A-4).
- Windows paths may contain spaces. Owner's machine: Windows 11 Pro 22631, C:/D:/E:/F: NTFS, `LongPathsEnabled` = 0. The app must work without changing that.
- Build in roadmap order.

## 9. Workflow

**Before a task:** run `git status`, read the roadmap state, identify the F-IDs and exit checks.

**While working:** one milestone task per branch, `m<N>/<short-name>`. Commit messages are imperative and reference IDs (`F-11: add map zoom camera`). Commit only when the owner asks. Never force-push, rewrite history, or delete branches.

**After a task:**
1. Run the tests and exit checks; report the real results.
2. Update the state line and status in `04`.
3. If a decision changed, edit `03` and append a dated entry to the log below. Never reverse a decision silently.
4. Update the feature spec if behaviour differs. Docs change in the same commit as the code.
5. End with a short summary: done, verified, not verified, next.

**Ask the owner only about:** signing and licensing, publishing, anything that touches their real data or system, and design sign-offs. Otherwise decide and record the reasoning.

## 10. Glossary

| Term | Meaning |
|---|---|
| MFT | NTFS Master File Table; one record per file |
| Fast / Standard | Elevated MFT scan / unelevated directory walk |
| Helper | `oz-helper.exe`, the elevated process |
| Explore (Atlas) | Main results workspace: tree, map, types |
| Map, Strata, Timeline | Treemap, depth icicle, age grid |
| Lens | Temporary colour encoding on a visualization |
| Safe, Check first, Keep | UI names for Rebuildable, Caution/Review, Protected |
| Reclaim | Staging list before any cleaning |
| Holding Area | Restorable same-volume move, `X:\OzHold\` |
| Relocate | Verified copy plus junction |
| Tweak | One catalogued, reversible debloat change |
| Ledger | Log of all operations and prior states |
| Unaccounted | Volume used minus scanned total |
| Signal | The one UI hue, amber, for risk |

## 11. Decision log

Append new entries; do not edit old ones.

- **2026-11-07** Project planned. Stack, process model and rules set in `03`. Project location moved by owner from `E:\Oz Cleaner` to `E:\code\oz-cleaner`.
- **2026-11-17** Owner widened scope to an all-in-one toolbox: Clean, Explore, and **Debloat**. Registry rule relaxed to "read-only except catalogued, reversible tweaks" (rule 3.9), with the tweak gate (section 6). Explicitly out of scope: app installer, downloaded scripts, licensing or activation tools (rule 3.11). Security and update settings are never changed (rule 3.10). The design moves closer to zaber.tech's editorial language. The owner requested this in chat.
- **2026-11-17** Owner asked for the uninstaller to remove an app "A to Z". Added section 6b: official uninstaller first, then evidence-ranked leftovers (files and registry), held or exported so they can be undone. Rule 3.9 is extended to cover removing an uninstalled app's own registry keys, with a backup.
- **2026-11-17** Design system "Oz Ink": tokens moved to OKLCH, text contrast raised (text-3 is now L .72), and an owner-requested **accent** `oklch(.86 .16 152)` added for the primary action, focus, selection, progress and success. It is never used for risk. The serif italic accent word is retired; `.accent` is now upright sans in the accent colour. Navigation is now Home / Clean / Explore / Tools / History / Settings. Motion tokens added: `--pop` 200 ms, exits 20% shorter, 30 ms stagger, press scale .97, popovers from .95.
- **2026-11-17** Owner reversed the accent: "DO NOT USE ACCENT COLOR." The green is removed. `--accent` now resolves to white, so primary actions and selection invert, as on zaber.tech.
