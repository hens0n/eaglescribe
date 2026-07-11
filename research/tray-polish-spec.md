# Tray polish — behavior specification

**Status:** Ready for `/to-tickets`  
**Date:** 2026-07-10  
**Priority:** Medium (STATUS suggested slice; SET-03)  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`SET-03`, Phase 1 tray/menu-bar)

This document is the implementable product/behavior contract for **menu-bar / system-tray polish**: a dedicated monochrome template glyph, reliable window restore from the tray, and an optional macOS **menu bar only** (no Dock) mode. It is not an implementation plan beyond acceptance criteria and intentional non-goals.

---

## 1. Goal

EagleScribe already lives in the menu bar / system tray (Show / Hide / Quit, close-hides, left-click restore, macOS `"ES"` title + full-color app icon). This slice makes that presence **look and behave like a first-class tray app**:

1. A **legible monochrome template glyph** in the menu bar (macOS) / tray (Linux), without the `"ES"` text crutch.
2. **Reliable restore** from left-click and **Show Window**, including when Dock is absent.
3. An **opt-in** macOS **menu bar only** setting that hides the Dock icon via activation policy, applied on next launch.

Today: full-color default window icon (template deliberately off), `"ES"` title on macOS, Dock reopen as a restore path; tray **Show** may be flaky on macOS.

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| Menu bar only (no Dock) | **Opt-in Settings toggle**, **default off** — Dock stays unless the user enables it |
| When menu-bar-only applies | **Next app launch** — save immediately; UI states restart required |
| Launch with menu-bar-only on | **Show main window** on launch (same as today); only Dock presence changes |
| Tray glyph | **Dedicated monochrome template** asset(s); **drop** macOS tray **title** (`"ES"`) once the glyph ships |
| Art bar | **Any** high-contrast silhouette/mark that stays legible at menu-bar size in **light and dark** appearances; exact motif is implementer/design choice |
| Icons in this slice | **Tray-only** new asset(s); Dock / `.app` / package / window icons **unchanged** |
| Session state in tray | **Static** glyph — no recording / busy / error variants |
| Show reliability | **Hard requirement** — left-click and Show Window must restore after hide/close, with Dock present **and** with menu-bar-only on |
| Interaction model | **Unchanged** — Show Window, Hide Window, Quit EagleScribe; left-click shows; right-click opens menu; Linux: menu is primary (click events unsupported as today) |
| Platforms | **macOS primary** for menu-bar-only (`ActivationPolicy::Accessory` or equivalent); **Linux**: same glyph + menu/click contract where the tray works; no Linux “hide dock/taskbar” story |
| Tooltip | Keep a short static tooltip (e.g. “EagleScribe — local dictation” or equivalent) |
| Hide-on-close | **Unchanged** — window close hides to tray; hotkeys stay active until Quit |
| Persistence | `settings.json` under OS app data (same class as `clipboard_restore`) |

---

## 3. Behavior

### 3.1 Template tray glyph

- Ship **dedicated** tray icon asset(s) designed as a **macOS template image**: black silhouette on transparent background (or the format Tauri/tray-icon expects for `icon_as_template`).
- On **macOS**, set the tray icon **as template** so the system tints it for menu-bar appearance.
- Do **not** set the full-color app icon as template (known to go invisible).
- On **Linux**, use the same monochrome (or high-contrast) tray asset without inventing macOS template semantics.
- After the template glyph is in use, **do not** set a tray **title** string on macOS (remove `"ES"`).
- **Legibility gate:** a human can find and recognize the tray item at normal menu-bar scale in both light and dark menu bars (qualitative acceptance).

### 3.2 Menu bar only (macOS)

- Settings control (label intent: **Menu bar only** / **Hide Dock icon** — exact copy flexible).
- Default: **off** for new installs and missing field (serde default `false`).
- When **off** (default): normal Dock icon; Dock click while windows are hidden still restores (today’s `Reopen` path).
- When **on**: after **next launch**, the app runs without a Dock icon (`ActivationPolicy::Accessory` or Tauri-equivalent). Tray remains the always-visible chrome.
- Persist under OS app data `settings.json`. Corrupt / unknown value → **off**.
- Toggle **saves immediately** but **does not** apply Dock presence until restart. UI must say it takes effect the next time EagleScribe starts.
- Non-macOS builds: setting is **hidden**, disabled, or a documented no-op — not a Linux/Windows dock-hide feature.

### 3.3 Window restore (reliability)

Restore paths that **must** work after the window was hidden (close or Hide):

| Path | Required |
| --- | --- |
| Left-click tray icon | Show + focus main window (macOS/Windows as supported today) |
| Tray menu **Show Window** | Show + focus main window |
| Dock icon click / macOS Reopen when all windows hidden | Show + focus — **only when Dock is present** (menu-bar-only off) |

Implementation detail (existing code already tries multiple menu-event paths): whatever is needed so **Show Window** is not flaky on macOS is **in scope**. Left-click remains first-class, not a substitute for fixing Show.

When **menu-bar-only** is on, left-click + Show are the **only** window restore paths (no Dock). Both must work.

### 3.4 Interaction contract (unchanged)

- Tray menu: **Show Window**, **Hide Window**, separator, **Quit EagleScribe**.
- Left-click → show (does not toggle hide).
- Right-click → menu (platform norms).
- Linux: tray click events may be unsupported — user uses the tray menu; document in README if needed.
- No tray items for start/stop dictation, status, or other actions in this slice.
- Quit unregisters global shortcuts and exits (as today).

### 3.5 Launch and hide-on-close

- App launch shows the main window (with or without menu-bar-only).
- Close button / window close → hide to tray; do not quit.
- Global hotkeys remain active while hidden until Quit.

### 3.6 Settings UI

Minimum:

- macOS: toggle for menu-bar-only, default unchecked.
- Copy that restart is required for Dock change.
- No requirement for a live “apply without restart” path.

Optional polish (same ticket if cheap; else follow-on): short help text that tray left-click / Show restores the window when the Dock is hidden.

### 3.7 Interaction with other features

| Feature | Interaction |
| --- | --- |
| Global hotkeys | Unchanged while hidden or menu-bar-only |
| Dictation / Command Mode | Unchanged; no tray status glyph |
| Mic picker / silence trim / inject | Independent |
| Escape cancel | Unchanged |
| Packaging / signed builds | Separate packaging spec; this slice may add tray assets under `src-tauri/icons/` (or equivalent) |

---

## 4. Acceptance criteria

An implementation is done when all of the following pass on **macOS** (daily driver). Linux criteria apply where a system tray is available.

1. **Template glyph:** Menu bar shows a monochrome template tray icon (not the full-color app icon as template); no `"ES"` title string.
2. **Legibility:** Glyph is findable in light and dark menu bars at default scale (human check).
3. **Dock unchanged:** With menu-bar-only **off**, Dock icon still appears and reopening from Dock while hidden restores the window.
4. **Default off:** Fresh settings / missing field → menu-bar-only disabled; Dock present after launch.
5. **Toggle + restart:** Enable menu-bar-only → quit → relaunch → no Dock icon; tray still present; main window still shows on launch.
6. **Disable + restart:** Turn menu-bar-only off → quit → relaunch → Dock returns.
7. **Show after hide:** Hide or close window → **Show Window** restores and focuses the main window (repeatable; not flaky).
8. **Left-click after hide:** Same as (7) via left-click on the tray icon.
9. **Restore without Dock:** With menu-bar-only on, after hide/close, both left-click and Show restore without using Dock.
10. **Quit:** Quit from tray exits; hotkeys no longer fire.
11. **Linux (best-effort):** Tray shows the monochrome glyph; Show/Hide/Quit still work via menu; menu-bar-only is absent or no-op.
12. **No regression:** With default settings, end-to-end dictation still works with window hidden to tray.

---

## 5. Suggested implementation seams (non-binding)

Pointers only — implementers may choose equivalent structure:

| Area | Notes |
| --- | --- |
| Assets | e.g. `icons/tray-template.png` (+ `@2x` if useful); wire via Tauri tray builder, not `default_window_icon` as template |
| `setup_tray` | `icon` + `icon_as_template(true)` on macOS; omit `.title("ES")`; keep menu + click handlers |
| Show reliability | Keep/consolidate menu event listeners so `tray-show` always hits `show_main_window` (including `app.show()` on macOS before window show) |
| `settings` | e.g. `menu_bar_only: bool` default `false` |
| Startup | On macOS, if `menu_bar_only`, set activation policy to Accessory before/at run (next-launch semantics) |
| UI Settings | Toggle + “restart required” hint; macOS-only visibility |
| README | Update System tray section: template glyph, optional menu bar only, restore paths |

---

## 6. Out of scope

| Item | Why |
| --- | --- |
| Full brand / Dock / package icon redesign | Locked to tray-only assets this slice |
| Dynamic tray status (recording, error, etc.) | Locked static glyph |
| New tray menu actions (dictate, cancel, …) | Interaction model unchanged |
| Launch-hidden / remember last visibility | Locked: always show window on launch |
| Linux/Windows “hide taskbar / panel icon” | No solid product API story; macOS Accessory only |
| Windows as first-class acceptance matrix | Map: not first-class unless a ticket needs a note |
| Live activation-policy flip without restart | Locked next-launch only |
| Autostart / launch at login | Packaging slice |
| Changing hide-on-close or Quit semantics | Already shipped; not this gap |

---

## 7. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **SET-03** | System tray / menu bar; hide main window | Template glyph + reliable restore + optional menu-bar-only |
| Phase 1 | Polish, tray, macOS focus | Menu-bar template + Accessory option |
| **NFR-03** | Idle tray footprint | Unchanged goal; no extra processes |
| STATUS gap | “Tray polish” | Template glyph; optional dock-hide |
| STATUS footgun | Flaky tray Show | Hard acceptance on Show + left-click |

---

## 8. Handoff

**Next step:** run **`/to-tickets`** against this document to publish tracer-bullet GitHub issues (label `ready-for-agent`), then implement the frontier with `/implement`.

**Do not** expand into full brand refresh, recording-state tray icons, or Linux dock-hiding without a new product decision.

