# Oz Cleaner: Product Vision

Status: planning. No application code exists. Written 2026-11-07.
Read with: `02-FEATURE-SPEC.md`, `03-ARCHITECTURE.md`, `04-ROADMAP.md`, `AGENTS.md`.

## 1. The one-sentence product

Oz Cleaner shows you, within seconds, exactly what is filling a Windows drive, explains what each large thing is, and helps you safely get the space back.

## 2. The problem

Windows reports "C: is nearly full" and offers little else. Three gaps follow:

1. **Not knowing.** Settings > Storage gives broad categories. Explorer shows no folder sizes.
2. **Not understanding.** A tool can show that `AppData\Local\Something` is 40 GB, but not whether it is a rebuildable cache or years of someone's work.
3. **Not trusting.** Most "cleaner" software earns its reputation badly: scary scores, registry cleaning, one-button "optimise", bundled upsells. Careful users avoid it and clean by hand.

WizTree, the owner's favourite tool, solves gap 1 superbly. Oz Cleaner must match that speed feeling, then also solve gaps 2 and 3, and look far better while doing it.

## 3. Where this came from

The owner, Osayeed Zaber, recently cleaned a 158 GB C: drive by hand with an AI agent. That session is the requirements source. Each lesson is a product rule:

| What happened by hand | Product rule |
|---|---|
| Caches and temp in AppData were the easy wins | A curated rule catalog finds these first (F-30) |
| Personal files can't be judged by name | Personal files are never recommended for deletion; the user reviews them (F-34) |
| Some folders needed admin rights | Elevation is explicit, explained and optional (F-02) |
| Some uninstallers refuse (Edge) | Run the vendor's uninstaller, report a refusal, never force (F-43) |
| Other user accounts were unreadable without elevation | Show them as "not readable", with the unread gap accounted for (F-07) |
| Paths over 260 characters broke deletion and verification | Long paths are a first-class test case in every file operation (A-7 in `03`) |
| Verified copy plus a junction moved data off C: well | This becomes a guided feature, Relocate (F-42) |

## 4. Who it is for

- **Primary: the owner.** Windows 11 Pro; NTFS drives C: (158 GB), D:, E: (466 GB each), F: (74 GB); a developer with Node, Python and Git. Has developer-sized clutter: package caches, `node_modules`, WSL/Docker disks.
- **Secondary: technically comfortable Windows users** who currently reach for WizTree, WinDirStat or TreeSize.
- **Not for:** people wanting one-click "PC boost", or fleet and enterprise deployment.

The project is also a portfolio piece, so its finish matters as much as its function.

## 5. Principles, in priority order when they conflict

1. **Never lose someone's data.** Safety beats space reclaimed, speed and looks.
2. **Tell the truth.** Real measurements, stated confidence, no scare scores. Numbers that can't be known are shown as unknown.
3. **Feel instant.** First useful picture in seconds; every interaction answers within one frame where possible.
4. **Earn beauty.** Visual polish must serve reading the data. Anything decorative must be cheap and removable.
5. **Read-only first.** Analysis works entirely without touching a file. Cleaning is a separate, deliberate step.
6. **Local only.** No account, no telemetry, no network calls except a user-approved update check. File names are private data.

## 6. What it refuses to do (permanent)

- Registry cleaning, "PC health" scores, RAM or startup "boosters", driver updates.
- Deleting Prefetch or similar for imagined speed gains.
- Touching `WinSxS` directly. Microsoft says not to delete it and provides supported methods.
- Forcing uninstalls, taking ownership of system files, or altering ACLs to get past a refusal.
- Auto-cleaning on a schedule without review in v1.
- Fear-based copy ("Your PC is at risk!"), dark patterns, ads, bundled software.

## 7. Success criteria

Targets, not measurements. Spike S1 replaces them with real baselines.

| Measure | Target |
|---|---|
| Elevated scan of owner's C: (NTFS, 123 GB used) | Under 5 s to complete tree |
| Unelevated scan of the same drive | Under 60 s, with progressive results from the first second |
| Treemap interaction on the largest view | 60 fps hover and zoom on the owner's machine |
| Scanned total vs volume used | Gap always displayed and explained |
| Cleaning | Zero unintended deletions; measured space freed always reported |
| Owner's verdict | Chooses it over WizTree for everyday use |

## 8. Identity and design language

### 8.1 What was actually observed on zaber.tech

Studied live on 2026-11-07 in a browser with screenshots and computed styles. Values below are measured unless marked otherwise.

**Colour.** Body background pure `rgb(0,0,0)`. Text `rgb(242,242,242)` primary, `rgb(189,189,189)` secondary, `rgb(138,138,138)` muted, `rgb(58,58,58)` faint. Surfaces barely lift: `rgb(5,5,5)`, `rgb(10,10,10)`, `rgb(10,9,13)`. Borders are a very dark grey (about oklab L 0.17) or white at 20% alpha. The interface is effectively monochrome; colour comes from project imagery, not from the UI.

**Typography.** Three families, each with one job:
- **Bricolage Grotesque:** display and body. The name is enormous (about 164 px at a 1024 px viewport), weight 800, letter-spacing about -0.055em, line-height about 0.88. Section headings 65-92 px, weight 700-800, tracking -0.045 to -0.05em. Body 17-18 px, line-height about 1.6.
- **JetBrains Mono:** every label, nav item, filter and caption. 12 px, uppercase, letter-spacing 0.14em.
- **Instrument Serif italic:** a single accent word beside heavy sans ("The *ledger*", "Full-stack", "*event websites.*"). Used sparingly, with strong contrast of voice.

The hero sets "Osayeed" in a condensed, light cut directly above "Zaber" in a very heavy one, contrasting weights in one lockup. The page text says the name changes weight with the cursor; I did not test that interaction.

**Structure.** Editorial, like a printed journal: "OZ — VOL. 26" masthead, numbered sections ("00 — COVER", "04 — LEDGER"), small mono metadata in opposing corners, hairline rules, stat rows split by hairlines with big numerals over tiny mono captions, generous empty black.

**Components.** Floating pill nav with a thin border; active item inverts to a white pill with black text. Primary button a white pill with an arrow glyph; secondary an outlined pill. Filter chips with the same inversion. Radii: 3.5-4 px on small cells, 12-24 px on panels, full pill on controls, 0 on most elements. Arrows (↘ ↗) as recurring signposts.

**Data display.** "The ledger" is the most relevant reference for this app: a GitHub contribution grid of small rounded squares in a greyscale ramp, in a barely lifted panel with a less-to-more legend. The copy says "Nothing here is typed in by hand", and that honesty suits a storage tool. Language percentages and a source timestamp sit beside it.

**Motion.** Measured transition timing: `cubic-bezier(0.4,0,0.2,1)` at 150, 300 and 500 ms; `cubic-bezier(0.16,1,0.3,1)` at 420 and 500 ms for expressive moves. Hero keyframes: lines rising from `translateY(110%)` behind masks, `hero-fade` (opacity plus 12 px), a rule drawing via `scaleX(0)`, a 24 px nudge. Marquees. Nav links contain doubled text, which suggests a rolling hover effect (inferred from DOM, not watched). A single canvas draws white light streaks radiating behind the hero. A visible "FULL EFFECTS" control in the footer gives users a say in motion, and the stylesheets include `prefers-reduced-motion` rules.

**Not observed:** scroll-driven transitions in motion (I jumped between positions by script), the page-transition behaviour, and the /lab and /tools subpages.

### 8.2 What Oz Cleaner takes from it

| Keep | Adapt for a dense tool |
|---|---|
| Pure black, greyscale text ramp, hairlines | Same values |
| The three-font system with fixed roles | Mono also carries all numbers and paths; display sizes shrink except at moments of arrival |
| Numbered mono section labels, corner metadata | Screen headers and status lines |
| Inverting pills, arrow glyphs | Navigation and primary actions |
| Greyscale grid as data | Timeline view (F-13) is its direct descendant |
| Masked rise, drawn rules, expo-out easing | Short durations; the same easing curves |
| Effects control | Motion setting: Full / Reduced / Off |
| Serif italic accent word | One per screen at most |

**Not taken:** long scrolling, marquees, drag carousels, ambient canvas streaks during work, huge type in working screens. A portfolio is browsed; a tool is used.

### 8.3 Oz Cleaner's own identity

**"A storage instrument, not a cleaner."** It should feel like a precision device printed in black ink: monochrome until something deserves attention.

- **Monochrome map.** The default treemap is greyscale and reads as a lit relief of the drive. This is the clearest break from WizTree's look and from every coloured disk map.
- **Colour means something.** The only UI hue is **Signal** (warm amber), reserved for risk and irreversible actions. Type colours appear only when the user switches on a lens, and leave when it is switched off.
- **Lineage.** Part of the owner's "OZ" mark. Masthead reads `OZ — CLEANER`, volume label beside it.
- **Signature moment.** When a scan completes, the map resolves from coarse to fine as data arrives, then the total used figure sets in large Bricolage with a drawn rule beneath. It happens because data is really arriving, never as a fake delay.

**Tone of voice:** plain, exact, a little dry. "11.4 GB of npm cache. Rebuilds itself the next time you install." Never "Boost your PC!"

Tokens and per-screen specification are in `02-FEATURE-SPEC.md`.

## 9. Relationship to WizTree

Inspired by its speed, density and directness. Must not copy its name, icon, art, colour scheme, layout or code. Its binary must never be decompiled for this project. The public fact that it reads the MFT directly is all we rely on, and that is a documented NTFS technique.

## 10. Business and distribution

Personal and portfolio project, free, all rights reserved until the owner decides otherwise. Windows 10/11 x64 is possible; Windows 11 is the only target verified. No macOS or Linux plans: the speed advantage is NTFS-specific.
