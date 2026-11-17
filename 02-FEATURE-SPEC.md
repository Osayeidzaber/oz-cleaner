# Oz Cleaner: Feature Specification

Feature IDs (F-xx) are stable; never renumber. Milestones (M0-M8) are defined in `04-ROADMAP.md`. Architecture rules (A-x) and decisions (D-xx) are in `03-ARCHITECTURE.md`.

Priority: **P0** required for v1, **P1** v1 if time allows, **P2** after v1.

## Part A: Terms

- **On-disk size:** allocated clusters. Default everywhere, because it answers "what would I get back?"
- **Logical size:** file length. Available as a toggle. Differs for compressed, sparse and cloud placeholder files.
- **Unaccounted:** volume used space minus the sum of everything scanned.
- **Reclaimable:** what a rule estimates; never presented as guaranteed.
- **Freed:** the measured change in volume free space after an operation.

## Part B: Scanning and analysis

### F-01 Volume overview (P0, M1)
Lists fixed and removable volumes with label, file system, capacity, used, free, and last scan age. Also supports scanning any chosen folder. Selecting a volume starts a scan immediately with no extra step.

### F-02 Two scan modes and explained elevation (P0, M1/M2)
- **Standard** (no elevation): parallel directory walk. Works on any file system and needs no prompt.
- **Fast** (elevated, NTFS only): direct MFT read by the helper. Triggers one UAC prompt per app session.
- First use explains in one sentence what Fast does and that it only reads file records. The user can keep using Standard forever.
- If UAC is declined: Standard runs with no error and no nag. Fast is offered again only when the user selects it.

Acceptance: declining UAC never produces an error state. Non-NTFS volumes never offer Fast.

### F-03 Progressive results (P0, M1)
The map appears early and refines. Shows records processed, elapsed time, and throughput. Can be cancelled at any time, keeping what was found and labelled partial.

### F-04 Correct accounting (P0, M1/M2)
- Hard links counted once toward totals, attributed to the first name, marked on every name.
- Reparse points (junctions, symlinks, mount points) never followed; shown as a link with its target.
- Alternate data streams included in the file's size in Fast mode. In Standard they're ignored, and the gap appears in Unaccounted.
- Cloud placeholders (OneDrive etc.) shown distinctly with on-disk and logical size. Scanning must never trigger hydration.
- Compressed, sparse and WOF-compacted files use real allocated size where readable.

Acceptance: differential test fixtures (see `04`, Testing) match expected totals in both modes, within documented exceptions.

### F-05 Results Workspace (P0, M3)
The main screen; see Part E. Tree, visualization and details are always linked: selecting in one selects in all.

### F-06 Tree table (P0, M3)
Virtualized list, sortable by on-disk size, logical size, % of parent, file count, modified. Inline bars. Must feel instant with folders of 100,000+ children. Keyboard navigation throughout.

### F-07 Unaccounted bar (P0, M2)
Persistent line: `Scanned 118.2 GB · Used 123.0 GB · Unaccounted 4.8 GB`. Clicking explains likely contributors (NTFS metadata, shadow copies, folders that couldn't be read) and lists unread paths, including other users' profiles in Standard mode. It says "likely" unless measured.

### F-08 Details panel (P0, M3)
Path (copyable), sizes, counts, dates, attributes, link status, and the matched "what is this" explanation (F-31). Actions: reveal in Explorer, copy path, add to Reclaim, ignore.

### F-09 Search and filter (P0, M4)
Instant substring search over names and paths. Filters by extension, size, age, attribute. Filter syntax documented in a tooltip (`ext:iso size>1g age>1y`). Results update the visualizations, by dimming non-matches.

### F-10 Top lists (P0, M4)
Largest files, largest folders (excluding nested double-counting), largest types. Maps directly to WizTree-class needs.

### F-11 Map: treemap (P0, M3)
Squarified treemap of the current folder with nested depth.
- Default greyscale. Luminance encodes depth; 1 px black gutters define structure.
- Hover: instant outline and a mono tooltip following the cursor. Click: select. Double-click or Enter: zoom. Esc/Backspace: up. Breadcrumb above.
- Zoom is a continuous camera move of 300 ms that keeps the clicked rectangle spatially continuous, never a hard cut.
- Labels on rectangles large enough to fit them; items below 2 px are merged into a labelled "n small items" cell.
- **Lenses** colour the map only while active: Type, Age, Reclaimable.

Acceptance: 60 fps on the owner's machine with the worst real folder; layout of a new view within 50 ms (target).

### F-12 Strata: depth view (P0, M4)
Horizontal icicle: each row one depth level, width proportional to size. Better than a treemap for "why is this deep path so big". Same selection, zoom and lens behaviour as the Map.

### F-13 Timeline: age grid (P1, M4)
Grid of weeks by months, each square brightness-coded by bytes last modified then, modelled on the ledger grid but holding real storage data. Clicking a square filters to those files. Answers "what have I not touched in years?"

### F-14 Scan snapshots and compare (P1, M7)
Save a compressed snapshot per scan locally. Compare two: what grew, what shrank, what's new. Answers "what ate 10 GB since last week?" User can disable or delete history.

### F-15 Export (P1, M7)
CSV of the current view, or the full list. Local file only.

### F-16 Live refresh by change journal (P2)
After a Fast scan, apply USN journal changes rather than rescanning. Depends on S6.

### F-17 Duplicate finder (P2)
Size, then partial hash, then full hash. Never auto-selects originals. Hard-linked files aren't duplicates.

## Part C: Understanding

### F-30 Rule catalog (P0, M5)
Declarative rules ship with the app (format in `03`, A-9). Each has: path pattern, display name, plain explanation, category, tier, conditions (age, app-not-running), action, what happens afterwards, and how it was verified. **A rule merges only when its path and behaviour have been verified**; see `AGENTS.md`.

### F-31 "What is this?" (P0, M5)
Any matched path shows the explanation. Unmatched: "Not recognised". Never a guess presented as fact. Heuristics can suggest an owner ("appears to belong to X, from its folder name") labelled as a guess.

### F-32 Tiers
| Tier | Meaning | Default state | Delete method |
|---|---|---|---|
| **Rebuildable** | Regenerated automatically; loss costs only time | Offered, unchecked until Reclaim is reviewed, then one "select all rebuildable" | Permanent, listed |
| **Caution** | Probably safe; has a real cost such as re-download or re-login | Unchecked, cost shown | Holding Area |
| **Review** | Candidates only, app has no opinion | Unchecked; no recommendation | Recycle Bin or Holding Area |
| **Protected** | Never deleted | No delete action; explanation and supported method only | None |

### F-33 Initial rule catalog (P0, M5)
Starting list. Paths must be confirmed per F-30 before shipping.

| Item | Tier | Handling |
|---|---|---|
| User temp `%LOCALAPPDATA%\Temp`, older than 24 h | Rebuildable | Delete; skip locked items |
| `C:\Windows\Temp`, older than 24 h | Rebuildable | Elevated delete; skip locked |
| Browser disk caches (Chrome, Edge, Firefox `cache` folders only, never profiles) | Rebuildable | Browser must be closed |
| npm cache, pip cache, Gradle/Maven-style dev caches | Caution | Prefer tool's own clean command; re-download cost |
| Crash dumps, Windows Error Reporting archives | Rebuildable | Delete |
| Thumbnail cache | Rebuildable | Skip locked; Explorer rebuilds |
| `node_modules`, `.venv`, `target`, `build` under dev projects | Review | Requires nearby project marker; shows last modified |
| Downloads, old large files | Review | Never recommended |
| Windows.old | Protected | Link to Settings > Storage |
| Windows Update cache | Protected | Link to Disk Cleanup / Storage Sense |
| Delivery Optimization | Protected | Link to Settings |
| WinSxS | Protected | Run Microsoft's `StartComponentCleanup` with the user's consent. Never `/ResetBase` by default |
| `hiberfil.sys`, `pagefile.sys` | Protected | Explain; link to settings |
| WSL and Docker virtual disks | Protected | Explain; shrink guidance only |
| `C:\Windows\Installer`, `System32`, `ProgramData` generally | Protected | Explain |
| Recycle Bin contents | Review | Shown with its size; emptying is permanent and uses the F-36 hold-to-confirm |

### F-34 Personal files rule (P0, permanent)
Files under user document areas and anything unmatched are **never** suggested on name alone. Largest and oldest lists are surfaced as facts, without recommendation language.

## Part D: Cleaning

Nothing here ships before M5. All operations follow A-7.

### F-35 Reclaim list (P0, M5)
A staging list that collects items from rules and manual additions. Grouped by tier, showing size, consequences and method. Nothing is deleted without passing through it.

### F-36 Review and confirm (P0, M5)
Confirmation lists counts, total, and method per tier. Permanent deletion uses a press-and-hold button (1.2 s, with keyboard alternative), with the wording stating it can't be undone. Rebuildable items use a normal button.

### F-37 Pre-flight re-check (P0, M5)
Immediately before each item: same file ID, still not a reparse point, not under a protected root, not newly modified after the scan. Changed items are skipped and reported.

### F-38 Honest results (P0, M5)
Report: freed (measured from volume free space), deleted, skipped with per-item reason (in use, access denied, changed). Partial success is normal and shown calmly. Access denied leads to an offer to retry elevated, not to a force.

### F-39 Ledger (P0, M5)
Append-only local log of every operation: time, paths, sizes, method, result. Viewable in the app.

### F-40 Recycle Bin (P0, M5)
Sends through the Windows shell. If an item can't be recycled (too long a path, no bin on volume, too large; see S5) the app says so **before** any change and offers Holding Area. It never silently falls back to permanent delete.

### F-41 Holding Area (P0, M5)
Same-volume move into `X:\OzHold\<operation-id>\` with a manifest. Instant, long-path-safe, restorable to the original location from the Ledger. Space is only freed when purged: UI shows "Held, not yet freed". Auto-purge prompt after 14 days; never purges silently.

### F-42 Relocate (P1, M6)
Moves a folder off a nearly full drive and leaves a junction. Steps: check target space, check nothing has files open, copy with long-path support and attributes, verify (file count, sizes, and hashes), rename original to `<name>.ozold`, create junction, test it, offer to remove the original later. Any failure before the junction rolls back and leaves the source intact. Excludes system folders, cloud folders, and anything on the protected list. Provides an Undo that reverses the junction.

### F-43 Uninstall helper (P1, M6)
Lists installed apps with size from the scan, and launches the vendor's registered uninstaller. If it refuses or fails (Edge is the known case) report it and stop. No forced removal, no registry surgery.

### F-44 Scheduled reminders (P2)
Tray-less notification when a drive crosses a threshold. No background cleaning.

## Part E: Interface specification

### E.1 Design tokens
Implement as CSS custom properties. Derived from the observations in `01`, section 8.1. No raw values in components.

**Colour**
| Token | Value | Use |
|---|---|---|
| `--ink-0` | `#000000` | App background |
| `--ink-1` | `#050505` | Recessed |
| `--ink-2` | `#0A0A0A` | Panel |
| `--ink-3` | `#141414` | Hover, selected row |
| `--line` | `#1F1F1F` | Hairlines |
| `--line-strong` | `rgb(255 255 255 / .2)` | Control outlines |
| `--text-1` | `#F2F2F2` | Primary |
| `--text-2` | `#BDBDBD` | Secondary |
| `--text-3` | `#8A8A8A` | Labels (at least 12 px; contrast is about 6:1 on black) |
| `--text-4` | `#3A3A3A` | Disabled and decoration only, never required information |
| `--invert` | `#FFFFFF` | Active pill |
| `--signal` | `#FFB454` | Risk, irreversible actions, only UI hue |

Lens palettes live in a separate token set, are validated for colour-blind separation, and always have labels. Signal never conveys meaning by colour alone; it is paired with text and an icon.

**Type**
| Token | Font | Spec |
|---|---|---|
| `display-xl` | Bricolage Grotesque 800 | 72-120 px fluid, tracking -0.05em, lh .9. Arrival moments only |
| `display` | Bricolage 700 | 32-44 px, -0.04em, lh 1 |
| `title` | Bricolage 600 | 20 px, -0.02em |
| `body` | Bricolage 400 | 14 px, lh 1.5 |
| `accent` | Instrument Serif italic | Display size matched; one word per screen |
| `label` | JetBrains Mono 400 | 11 px uppercase, tracking .14em |
| `data` | JetBrains Mono 400 | 12-13 px, tabular, for sizes and paths |

Fonts are bundled locally as woff2. No web font requests. Confirm each font's licence (expected SIL OFL) at M0.

**Spacing:** 4 px base: 4, 8, 12, 16, 24, 32, 48, 64, 96.
**Radius:** 0 for data surfaces, 4 small cells, 12 panels, pill for controls.
**Density:** table rows 28 px (default) or 24 px (compact). Minimum pointer target 28 px, keyboard for everything.

**Motion**
| Token | Value | Use |
|---|---|---|
| `--fast` | 150 ms `cubic-bezier(.4,0,.2,1)` | Hover, press, toggles |
| `--move` | 300 ms `cubic-bezier(.4,0,.2,1)` | Panels, map zoom |
| `--arrive` | 420 ms `cubic-bezier(.16,1,.3,1)` | Screen changes, reveals |
| `--stagger` | 24 ms, max 8 items | Lists entering |

Rules: animate only `transform`, `opacity` and shader uniforms. Never make a user wait for animation: input during a transition retargets it. Hover has no delay. Motion setting is Full / Reduced / Off, defaulting from `prefers-reduced-motion`. Reduced replaces movement with short fades; Off removes all but progress indication.

### E.2 Shell
Custom frameless title bar, `OZ — CLEANER` at left in `label`, volume and scan state centred, native-feel window controls at right (including Snap Layouts behaviour; see S4). Screen navigation is an outlined floating pill; the active item inverts. Screens numbered in mono: `00 VOLUMES`, `02 ATLAS`, `03 RECLAIM`, `04 LEDGER`, `05 SETTINGS`. A status line at the bottom holds the Unaccounted bar and mode.

### E.3 Screens

**00 Volumes.** Editorial cover. Volumes stacked as full-width rows divided by hairlines; each has a thin capacity line drawn from zero on entry, big used number, mono metadata at the corners. Nearly-full volume gets a Signal tick plus text ("4% free"). Row hover lifts the arrow glyph. Empty black is intentional: no cards, no gauges, no donut charts.

**01 Scan.** A state of Atlas rather than a separate page: the map draws itself as data arrives, with counts ticking in `data`. On completion, the used total sets in `display` with a drawn rule. Skipped under Motion Off.

**02 Atlas.** Three panes: tree table (left), visualization with Map/Strata/Timeline switch (centre, biggest), details (right, collapsible). Hairlines only between panes, draggable. Top lists appear as tabs on the tree. Below 1100 px wide, details becomes an overlay; below 800 px, tree collapses to a drawer. Minimum window 720x500.

**03 Reclaim.** A statement-like ledger: tiers as numbered sections, each row a plain sentence with size in mono and consequence below. The running total at top in `display` labelled "estimated". Primary action a white pill with arrow; irreversible actions outlined in Signal.

**Operation running.** Compact progress in place; the list strikes through items as they finish. Results then appear as stats split by hairlines: Freed / Deleted / Skipped. No confetti. The free-space figure counts to its measured value once.

**04 Ledger.** History in a table, with restore for Holding Area items and space-over-time sparkline (P1).

**05 Settings.** Motion, size basis, density, history retention, exclusions, Holding Area, about and licences. Every setting must take effect.

### E.4 Micro-interactions
- Hovering a tree row highlights its rectangle, and vice versa, within one frame.
- Copy path: label swaps to `COPIED` for 1 s, in place.
- Size basis toggle morphs rectangles to new layout over `--move`.
- Breadcrumb segments roll in with masked rise on zoom.
- Press states scale controls to .98. Arrows nudge 2 px on hover.
- Skeletons don't exist: real data or a mono status line.

### E.5 States every view must design
Empty, loading/partial, cancelled, permission-limited, drive removed mid-scan, zero results, enormous values (TB, millions of files), long paths truncated in the middle (`C:\Users\…\file.bin`), full path available on hover and copy.

### E.6 Accessibility
Full keyboard operation including the map (arrows move between rectangles, Enter zooms). The tree table is the screen-reader equivalent of every visualization; canvas has a live description of the selection. Visible focus ring: 1 px `--text-1` with offset. Text meets WCAG AA, with `--text-4` restricted as above. Respects Windows text scale and high contrast mode (falls back to system colours).

### E.7 Forbidden visual patterns
Default component-library looks, gradients as decoration, glassmorphism, drop-shadow card grids, donut or gauge "health" charts, coloured status pills everywhere, emoji, stock icons in filled style, spinners longer than necessary, purple. Reference: if it could be any SaaS dashboard, it is wrong.
