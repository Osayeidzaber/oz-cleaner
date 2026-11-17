# Oz Cleaner: Architecture and Technical Decisions

## Evidence labels

Every technical claim carries one label. Upgrade labels as evidence arrives; never upgrade one without evidence.

- **[DOC]** stated in the cited primary source, read on 2026-11-07.
- **[MEASURED]** checked on the owner's machine on 2026-11-07.
- **[KNOWN]** engineering knowledge not verified in this research. Must be proven by the named spike before anything relies on it.
- **[TARGET]** a goal, not a result.
- **[UNVERIFIED]** sources conflict or are silent. Must not be relied on until the named spike settles it.

## 1. Facts this design rests on

### Owner's machine [MEASURED]
Windows 11 Pro build 22631. All of C:, D:, E:, F: are NTFS. `LongPathsEnabled` is 0. WebView2 runtime 157.0.4341.42 present. Node 22.23.4 present. **Rust is not installed** (M0 installs it). The default session token is not elevated.

### How fast scanners work
- WizTree reads the NTFS MFT directly, bypassing the OS file APIs, and also scans other drive types by other means. Its FAQ: high speed NTFS scanning "is only possible when running with admin rights". [DOC: diskanalyzer.com/about, /faq]
- Opening a volume with `CreateFile` on `\\.\X:` requires that "the caller must have administrative privileges". [DOC: CreateFileW]
- `FSCTL_ENUM_USN_DATA` enumerates MFT records and needs an NTFS volume. [DOC] Its records contain file reference numbers, names, parents and attributes but **no file size**. [KNOWN, S1] So it can't produce a size map alone, and would require a further per-file query, losing the benefit.
- Hence direct parsing: boot sector or `FSCTL_GET_NTFS_VOLUME_DATA` to locate `$MFT`, read record 0 to get its data runs, read those runs sequentially, apply fixups, parse `$STANDARD_INFORMATION`, `$ATTRIBUTE_LIST`, `$FILE_NAME` and `$DATA`. Parsing record by record this way is an established approach. [DOC: dfir-orc.github.io parser notes; details KNOWN, S1]
- Multiple `$FILE_NAME` attributes represent hard links. [KNOWN, S1]

### Windows file operation facts
- `\\?\` paths allow about 32,767 characters. Without it, long path support needs both the `LongPathsEnabled` registry value and a `longPathAware` manifest, and the owner's registry value is 0. [DOC: Maximum Path Length; MEASURED]
- Therefore **the app must never depend on the registry value**. It always uses `\\?\` internally. (A-7)
- Junctions link directories across local volumes via reparse points; hard links are files only on the same volume. [DOC: Hard links and junctions]
- `IFileOperation` replaced `SHFileOperation` and is STA-only. [DOC] Recycle flag and long-path behaviour: reports conflict and Microsoft's page doesn't settle it. [UNVERIFIED, S5]
- WinSxS: "Don't delete the WinSxS folder". Supported: `StartComponentCleanup` task or `Dism /Online /Cleanup-Image /StartComponentCleanup`. `/ResetBase` makes existing updates impossible to uninstall. [DOC]
- Change journal records file changes, read with `FSCTL_READ_USN_JOURNAL`. [DOC] Privileges are unconfirmed. [S6]

### UI platform facts
- Tauri's Core process alone has full OS access and routes IPC. The event system "is not designed for low latency or high throughput"; Channels "are designed to be fast and deliver ordered data". [DOC: v2.tauri.app]
- WebView2 Evergreen is included with Windows 11, but "some devices might not have the Runtime pre-installed", so installers must check. [DOC]
- A Tauri app manifest applies to the whole exe; `requireAdministrator` always prompts. [DOC: Tauri build API, tauri-apps source]
- Published Tauri vs Electron memory comparisons disagree. Installer sizes agree (Tauri far smaller). Treat memory as untested. [Secondary sources]

## 2. Decisions

### D-01 Shell: Tauri 2 (Rust core, WebView2 UI)
| Option | Speed of core | UI ceiling for this brief | Cost | Verdict |
|---|---|---|---|---|
| **Tauri 2 + Rust** | Native | Full CSS, variable fonts, WebGL2 | New language for owner | **Chosen** |
| Electron + native addon | Native addon | Same web UI | Large installer, needs a native addon anyway | Rejected: no benefit over Tauri here |
| WinUI 3 / C# | Good | Strong native, but custom type-led motion is laborious and less familiar | No web skills reuse | Rejected |
| Native Rust GUI (egui, wgpu) | Native | Immediate-mode look; typography and accessibility far harder | High | Rejected |

Reasoning: the two hardest requirements pull opposite ways, raw-disk speed and a zaber.tech-grade interface. Rust handles one; the owner's real expertise (TypeScript, CSS, motion) handles the other. Rust gives memory-safe parsing of an untrusted on-disk format. Risk: WebView2 version variance on other machines, addressed by feature checks.

### D-02 Frontend: React + TypeScript + Vite, hand-written CSS
Owner's daily tools, per zaber.tech's toolkit. React manages only chrome (panels, forms, screens). All high-frequency drawing bypasses React (D-04). No component library, no Tailwind: E.1 tokens in plain CSS layers make the custom look unavoidable rather than fought for. Headless accessibility helpers may be added under the dependency rule. Motion uses CSS transitions and WAAPI; no animation library until a real need is shown.

### D-03 Process model: unelevated app plus short-lived elevated helper
- `oz-cleaner.exe` (Tauri, `asInvoker`): UI and index. **Never elevated.** A browser engine should not run with admin rights. Drag-drop and shell integration also misbehave across elevation levels.
- `oz-helper.exe` (Rust, no UI, `requireAdministrator`): launched through `ShellExecuteEx` `runas`, exits when the parent exits or after idle timeout.

Rejected: whole app `requireAdministrator` (prompt every launch, elevated WebView2); scheduled task or service to skip UAC (standing privileged surface for a convenience).

Helper security rules:
1. Named pipe with an ACL restricted to the launching user's SID, random name, and a nonce given on launch.
2. Verify connected client's process ID matches the launcher.
3. A small fixed typed protocol: `ScanVolume`, `Cancel`, `ExecutePlan`, `RunComponentCleanup` (Microsoft's `StartComponentCleanup` only, never `/ResetBase`), `Shutdown`. No general "run command", no arbitrary path read, no content reads. Relocate's copy verification reads file contents for hashing, and only runs in the unelevated core in v1.
4. `ExecutePlan` revalidates every item independently against the protected list (A-7). The helper never trusts the UI.

### D-04 Treemap rendering: hand-written WebGL2 instancing
Instanced quads with per-instance position, size, and colour index. Custom shader draws gutters and luminance, with lens palettes as uniforms. No chart library (no rendering suited to this scale, and generic look). PixiJS rejected: its abstraction isn't needed for one draw call, and available evidence on its rectangle throughput is old anecdote. WebGPU rejected: reported as enabled in WebView2 only by an unofficial flag in an old thread, and Tauri does not officially support that. Revisit later.

Fallback when WebGL2 is unavailable: Canvas 2D at reduced detail. Labels are drawn on a separate Canvas 2D overlay.

### D-05 Layout in Rust
Squarified layout of the visible subtree runs in the core, culled by a pixel threshold, sent as a compact binary array of instances. [TARGET: under 50 ms, under 200,000 instances] Hit-testing in TypeScript against a uniform grid from the same array. Zoom animates the camera over existing geometry, then swaps in the new layout on landing.

### D-06 Data transfer: Tauri Channels with binary payloads
Channels for ordered streams (progress, layouts). The UI **never** holds the whole tree: it asks for windows (children page, layout for node X). Binary rather than JSON for arrays. [Exact binary mechanism: S3]

### D-07 In-memory index: struct-of-arrays
Arena per scan: parallel arrays for parent, first child, next sibling, size, allocated, modified, flags; names in one UTF-16/UTF-8 string arena. Node IDs are `u32`. MFT record numbers map to nodes. [TARGET: under 150 bytes per record, about 2 million records in 300 MB worst case]

### D-08 Own MFT parser, `mft` crate as test oracle
The `mft` crate (safe Rust, current 0.7.0) parses MFT **files** and is meant for forensics. [DOC: docs.rs] We need raw volume streaming, minimal allocation and just a few attributes, so a focused parser is justified, about 1,000 lines. Dump `$MFT` to a fixture and compare parsers in tests. Every read is bounds-checked; malformed records are skipped and counted, never panic. No `unsafe` outside the Windows FFI module.

### D-09 Standard scan: parallel walk
Rust workers using `FindFirstFileExW` with `FindExInfoBasic` and `FIND_FIRST_EX_LARGE_FETCH`, or `NtQueryDirectoryFile` if S2 shows real gains. `\\?\` paths. Reparse points recorded, never entered. Sizes from directory enumeration may lack allocated size, so we estimate by cluster rounding and mark "approx". [KNOWN, S2]

### D-10 Persistence
`%LOCALAPPDATA%\OzCleaner\`: `settings.json`; `ledger.sqlite` for the Ledger (F-39) via `rusqlite`; `snapshots/` compressed binary index dumps (`zstd`). Local only. Never in logs: full paths at default log level.

### D-11 Packaging
Tauri NSIS installer, per-user install, WebView2 check at install, portable zip also offered. Helper bundled. Code signing is undecided; unsigned builds will trigger SmartScreen warnings and show an unknown UAC publisher. Owner decides at M8. Auto-update: only with signing, and opt-in.

### D-12 Rule catalog as data
TOML files compiled in. See A-9. Lets rules be reviewed and tested independently of code.

### D-13 Rust crates
Initial allowed: `tauri`, `windows` (windows-rs), `serde`, `rusqlite`, `zstd`, `toml`, `rayon`, `blake3` (Relocate verification), `thiserror`, `tracing`. Pin versions at M0 to then-current releases; none are specified here because they weren't verified. Anything else needs a note in the decisions log (`AGENTS.md`, section 8).

### D-14 Debloat: catalogued, reversible tweaks (2026-11-17)
New pure crate `oz-tweaks` holds a TOML catalog like D-12. Each entry: `id`, `name`, `explain`, `tier` (Recommended / Optional / Explain only), `kind` (`appx_user`, `optional_feature`, `reg_value`, `task`, `service_start`), exact `targets`, `apply`, `revert`, `builds`, `verified`. Windows calls live in `oz-win`; HKLM, service and task changes run in the helper as the fixed enum operation `ApplyTweak(id)` / `RevertTweak(id)`, never a path or command. Prior state is written to the Ledger before every change. A restore point is offered before each batch (mechanism [UNVERIFIED, S8]). Security and update settings are excluded at the loader: the loader rejects any target under Defender, Firewall, UAC, Windows Update, BitLocker or SmartScreen policy keys. Out of scope: app installation, downloaded scripts, licensing.

## 3. System view

```
 Unelevated                                          Elevated (on request)
┌───────────────────────────────┐                  ┌──────────────────────┐
│ oz-cleaner.exe                │  named pipe,     │ oz-helper.exe        │
│ ┌───────────┐ Tauri ┌───────┐ │  user SID ACL    │  MFT reader          │
│ │ WebView2  │Channel│ Rust  │ │◄────────────────►│  Plan executor       │
│ │ React UI  │◄─────►│ core  │ │  records, typed  │  (revalidates)       │
│ │ WebGL2 map│       │ index │ │  ops             └──────────┬───────────┘
│ └───────────┘       │ layout│ │                             │ \\.\C:
│                     │ rules │ │                             ▼
│                     │ walker│ │                        NTFS volume
│                     └───────┘ │
└───────────────────────────────┘
```

Cargo workspace (created at M0, not yet):
```
crates/oz-model     index, sizes, no Windows code, fully unit tested
crates/oz-ntfs      MFT parser, operates on byte slices, no I/O
crates/oz-win       all Windows FFI: volume read, walker, file ops, shell
crates/oz-rules     catalog loader and matcher
crates/oz-ops       plan building, pre-flight, execution, ledger
crates/oz-layout    squarified layout
apps/helper         elevated binary
apps/desktop        Tauri app; src-tauri + ui/
```

## 4. Key flows

**Fast scan:** user picks volume → core launches helper (UAC) → pipe handshake → helper streams batched raw fields → core builds index, resolves parents → periodic layout pushed over Channel → completes → Unaccounted computed from `GetDiskFreeSpaceExW`.

**Clean:** Reclaim list → core builds Plan (typed items with file ID and tier) → confirm → items needing admin go to helper, others run in core → each passes pre-flight → results stream to UI and Ledger → freed measured.

## 5. Architecture rules (permanent)

- **A-1** The scan path is read-only. The volume handle is opened with read access and is never used to write.
- **A-2** No raw sector writes, ever. All changes go through the file system APIs.
- **A-3** UI process never elevated.
- **A-4** UI thread never parses bulk data; anything over 16 ms goes to Rust or a worker.
- **A-5** All Windows FFI lives in `oz-win`. All other crates are testable on any input.
- **A-6** Paths internally are `\\?\`-normalized wide strings. Display strips the prefix.
- **A-7 File operation safety:**
  - Protected roots are hard-coded in `oz-ops`, checked in both core and helper, and include everything beneath them: `C:\Windows`, Program Files, ProgramData, and `$`-prefixed metadata, plus anything matching a Protected rule. The volume root and each user profile folder are protected as objects themselves, not their contents. A short hard-coded exception list sits beside these roots: `C:\Windows\Temp` contents and Windows Error Reporting archives under ProgramData (F-33). Adding an exception requires a decision log entry.
  - Never traverse a reparse point on delete. Delete the link itself only.
  - Recursive delete works depth-first with `\\?\` and handles read-only attribute removal for files in a plan. No ACL or ownership changes. Long-path fixtures are mandatory for all of it.
  - No operation may recall cloud placeholders. Placeholders are not delete candidates in v1, since deleting one may delete the cloud copy.
  - Never fall from a safer method to a less safe one automatically (F-40).
  - Pre-flight re-check for each item (F-37).
  - Other users' profiles are not delete targets in v1.
- **A-8** No network access except a future opt-in update check. Tauri capabilities and CSP are locked to the minimum; the frontend has no direct file system permission and calls only named commands.
- **A-9 Rule format.** Fields: `id`, `name`, `explain`, `after` (what happens afterwards), `tier`, `category`, `paths` (env-based patterns), `conditions` (`min_age`, `process_not_running`), `action` (`delete_contents`, `run_tool`, `open_settings`, `none`), `verified` (date, Windows build, method). The loader rejects a rule without `verified`, and any non-Protected rule that matches a protected root outside the A-7 exception list. `run_tool` names an entry in a fixed enum (for example `ComponentCleanup`), never a command string.

## 6. Open spikes

Each must run in M0 unless noted, and record measured results in this file.

| ID | Question | Pass condition | If it fails |
|---|---|---|---|
| S1 | Can a Rust parser read owner's C: MFT fast and correctly? Confirm USN record lacks size, hard link and ADS handling | Total within 1% of a slow elevated walk, under 5 s | Investigate data; relax target |
| S2 | Best unelevated walk API; can allocated size be had cheaply? | Under 60 s on C: | Accept slower, note it |
| S3 | Binary transfer over Tauri Channel at 200k instances | Under 16 ms transfer | Shared buffer or chunking |
| S4 | WebGL2 instanced map in WebView2 on real data; frameless window with Snap Layouts | 60 fps; snap works | Reduce detail; native title bar controls |
| S5 | `IFileOperation` recycle with paths over 260 chars, on fixtures only | Documented behaviour table | Holding Area for those paths |
| S6 | USN journal read privileges, journal-disabled cases (M7 or later) | Documented | F-16 stays deferred |
| S7 | UAC launch and pipe handshake: decline, cancel, parent crash | All handled cleanly | Redesign before M2 |

Spike code lives in `spikes/`, is thrown away, and never touches real data for write operations. S5 uses a scratch folder.

## 7. Known risks

| Risk | Handling |
|---|---|
| Parser bug produces wrong sizes | Differential tests, Unaccounted always visible |
| Wrong rule deletes real data | Verified-rule gate, tiers, pre-flight, protected roots checked twice |
| MFT read is inconsistent with a changing drive | Snapshot accepted as approximate; pre-flight guards deletion |
| Antivirus flags raw volume read or unsigned helper | Signing decision at M8; document for users |
| ReFS or Dev Drive volumes | Standard scan only |
| BitLocker volumes | Expected to read normally when unlocked [KNOWN, test at M2 if available] |
| Owner new to Rust | Agent-written with heavy tests; simple crate boundaries |

## 8. Sources

- WizTree: https://diskanalyzer.com/about, https://diskanalyzer.com/faq
- CreateFileW: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
- FSCTL_ENUM_USN_DATA: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_enum_usn_data
- Path limits: https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation
- Change journals: https://learn.microsoft.com/en-us/windows/win32/fileio/change-journals
- IFileOperation: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation
- SHFileOperation: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shfileoperationw
- Hard links and junctions: https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions
- WinSxS cleanup: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/clean-up-the-winsxs-folder
- WebView2 distribution: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution
- Tauri: https://v2.tauri.app/concept/process-model/, https://v2.tauri.app/concept/inter-process-communication/, https://v2.tauri.app/develop/calling-frontend/
- Tauri manifest: https://www.github.com/tauri-apps/tauri/blob/33c3d72480993d1bc179e4efc3ddea2a1557eac0/core/tauri-build/src/lib.rs
- Tauri WebGPU thread: https://github.com/tauri-apps/tauri/issues/6381
- MFT parsing notes: https://dfir-orc.github.io/fs_implem_details.html
- `mft` crate: https://docs.rs/crate/mft/latest
- Tauri vs Electron (secondary): https://tech-insider.org/tauri-vs-electron-desktop-app-framework-2026/
