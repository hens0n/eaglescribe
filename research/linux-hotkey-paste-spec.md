# Linux hotkey & paste reliability — product specification

**Status:** Ready for `/to-tickets`  
**Date:** 2026-07-10  
**Priority:** High (STATUS Linux gap; Phase 2; persona P-C)  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`DICT-01`, `INJ-01`–`INJ-03`, Phase 2) · [packaging-spec.md](./packaging-spec.md)

This document is the implementable product/behavior contract for **Linux global hotkeys and text injection reliability** (X11 vs Wayland, distro deps, fallbacks, docs, smoke matrix). It is not an implementation plan beyond acceptance criteria and intentional non-goals.

Research sources (primary): `global-hotkey` 0.8.0 (used via `tauri-plugin-global-shortcut`), `enigo` 0.3.0, `arboard` 3.x README, EagleScribe `inject.rs` / `lib.rs` / `hotkey.rs`, Tauri global-shortcut plugin docs.

---

## 1. Goal

Linux users can run the **same core loop** as macOS dogfood: start/stop dictation (global hotkey **or** in-window UI), get transcript into the focused field via **clipboard + paste**, and when paste fails still have text on the **clipboard** with a clear notice (INJ-03).

Today the stack is **X11-shaped**:

| Layer | Crate / path | Linux reality (as of this research) |
| --- | --- | --- |
| Global hotkeys | `tauri-plugin-global-shortcut` → **`global-hotkey` 0.8.0** | Official support: **Windows, macOS, Linux (X11 only)**. Linux backend is X11 grab via `x11rb`. Open Wayland issue [#28](https://github.com/tauri-apps/global-hotkey/issues/28); error text if no X11 connection: other window systems “not supported”. |
| Paste / copy simulation | **`enigo` 0.3** (default features) | Default: **Linux X11**. Wayland / libei backends exist behind **feature flags** and are labeled **experimental** with known bugs. Runtime dep often **`libxdo`** / `xdotool` packages. |
| Clipboard | **`arboard` 3** (default) | Default Linux backend: **X11 / XWayland**. Optional **`wayland-data-control`** feature for pure Wayland compositors that implement data-control protocols; not enabled in EagleScribe today. Clipboard **ownership** is process-local until something reads it (unlike macOS). |
| App inject path | `inject.rs` | Clipboard set → main-thread simulate **Ctrl+V** (non-macOS); on paste failure keep text on clipboard and surface message (already). |

**Product stance (locked without further grill):** do **not** claim “full Wayland global hotkeys” in this slice. **Commit hard acceptance on X11 sessions.** Treat pure Wayland as **honest best-effort** with documented limits and **mandatory** clipboard + in-app UI fallbacks so the product never silently fails.

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| Hard acceptance session | **X11** (native X11 session, or environment where `global-hotkey` / enigo X11 paths work — typically `$XDG_SESSION_TYPE=x11` or XWayland-backed apps under mixed setups where X11 grabs still work) |
| Pure Wayland global hotkeys | **Not a hard acceptance criterion** this pass — document as known limit until upstream `global-hotkey` (or portal) support lands |
| Wayland paste | **Best-effort**; when simulation fails, **clipboard-only** path is the supported outcome (already product rule) |
| In-app controls | **UI Start/Stop / Cancel must work** without global hotkeys (primary Wayland-safe control surface) |
| Clipboard on inject fail | **Keep transcript on clipboard** + status/log message (INJ-03) — already shipped; must remain correct on Linux |
| arboard ownership | On Linux, inject path must not drop clipboard data before paste can consume it — **long-lived ownership or `wait`-style semantics** as arboard docs require |
| arboard Wayland feature | **In scope to enable** `wayland-data-control` (or document why not) so pure Wayland clipboard set is more likely where the compositor supports data-control |
| enigo Wayland/libei features | **Optional investigation** — may enable behind Linux feature flag if smoke tests improve paste; **not** required if X11 path + clipboard fallback meet acceptance |
| XDG Global Shortcuts portal | **Out of scope** to implement a parallel portal-based hotkey stack in this slice; track as future when `global-hotkey` / Tauri gains it |
| Distro matrix for hard pass | At least **one** documented X11 path (e.g. Ubuntu/Fedora/Arch under X11 or Xorg session); second DE is best-effort |
| Permissions UX | Linux has no macOS Accessibility prompt; failures are **dependency / session-type / compositor** — document packages and `$XDG_SESSION_TYPE` |
| Windows | Out of scope (map default) |

---

## 3. Behavior

### 3.1 Session support matrix (product commitments)

| Environment | Global hotkeys | Simulated paste (Ctrl+V) | Clipboard set | Product commitment |
| --- | --- | --- | --- | --- |
| **X11 session** | Expected to work via current plugin | Expected with `libxdo` (or enigo X11 backend) | Expected | **Hard acceptance** — end-to-end dictation |
| **Wayland + XWayland** | Often works if X11 connection exists for grabs; **verify** | May work for X11 clients; Wayland-native targets vary | Often via X11/XWayland clipboard | **Documented best-effort**; smoke if available |
| **Pure Wayland** (no usable X11 for grabs) | **Likely fail to register / never fire** with current stack | Often fails without experimental enigo features + compositor support | Needs data-control or fails | **Supported product outcome:** UI-driven capture + **clipboard-only** inject; clear messaging |
| Headless / no display | N/A | N/A | N/A | Dev/CI: soft-fail tests only (as today) |

### 3.2 Global hotkeys (Linux)

- Registration path remains `tauri-plugin-global-shortcut` (same as macOS).
- On **registration failure** (no X11, grab denied, etc.):
  - App **must not crash**.
  - Log a **clear Linux-specific** line (session type if known, “global hotkeys require X11 with current build”, point to README).
  - Surface in UI/status or Log tab: global hotkeys unavailable; **use window controls**.
  - In-window dictation / Command Mode buttons and Escape-via-UI Cancel remain usable.
- On **X11**, rebind, hold/toggle, Escape-while-recording, Command Mode chord behavior match the existing product contract (same as macOS where possible).
- Do **not** silently claim hotkeys are active when registration failed.

### 3.3 Text injection (Linux)

Pipeline stays:

1. Optional snapshot previous clipboard (if restore enabled).
2. Set transcript on clipboard (`arboard`).
3. Simulate paste (**Ctrl+V** on Linux today).
4. If paste succeeds and restore enabled → restore previous after delay.
5. If paste fails → **leave transcript on clipboard**, notify user (existing status strings / log).

Linux hardening for this slice:

- **Clipboard lifetime:** fix any path that constructs a short-lived `Clipboard`, sets text, and drops immediately before paste — arboard documents ownership races on Linux. Prefer app-held clipboard helper, `SetExtLinux::wait` where appropriate, or equivalent so paste targets can read data.
- **Failure visibility:** paste failure must remain user-visible (status/log), not only `eprintln!`.
- **No macOS Accessibility copy** on Linux — instead, if paste repeatedly fails, docs/UI can point to session type + packages + “paste manually from clipboard.”

### 3.4 Distro / runtime dependencies (docs contract)

README (or `research/` linked from README) **must** document install hints for building/running with working X11 inject:

| Family | Packages to document (adjust names as verified) |
| --- | --- |
| Debian/Ubuntu | `libxdo-dev` (build), `libxdo3` / xdotool runtime as needed; build toolchain + `libasound2-dev` / PipeWire for cpal already noted |
| Fedora | `libX11-devel` `libxdo-devel` (per enigo README) |
| Arch | `xdotool` / related (per enigo README) |

Also document:

- How to check session: `echo $XDG_SESSION_TYPE` (`x11` vs `wayland`).
- Recommendation for **daily-driver reliability today:** X11 session **or** accept UI + clipboard-only on pure Wayland.
- That **Metal/CUDA** are unrelated; GPU features are packaging-spec.

### 3.5 Optional engineering improvements (in scope if cheap)

Implementers may ship any of these under the same `/to-tickets` work if they improve the matrix without expanding scope:

1. Enable **`arboard` `wayland-data-control`** feature for Linux builds.
2. Probe session type at startup; set a status field `linux_session: x11 | wayland | unknown` for UI/docs.
3. On hotkey register failure, one-shot Settings/Log banner with README link.
4. Experiment with `enigo` `wayland` / `libei` features behind a Cargo feature — only promote to default if X11 path remains intact and Wayland paste smoke improves.

### 3.6 Interaction with other features

| Feature | Interaction |
| --- | --- |
| Clipboard restore (INJ-04) | Same rules; Linux ownership bugs can break restore — fix ownership first |
| Escape cancel | Global Escape only if hotkeys registered; UI Cancel always |
| Command Mode selection copy | Same enigo chord (`Ctrl+C`); same X11/Wayland limits |
| Tray | Linux tray already best-effort; not this slice’s focus |
| Packaging | AppImage notes stay packaging-spec; this spec is runtime reliability |

---

## 4. Acceptance criteria

### 4.1 Hard (must pass on at least one documented **X11** Linux environment)

1. **Deps documented:** README lists packages for the test distro and how to verify `$XDG_SESSION_TYPE`.
2. **Hotkey register:** App starts; default dictation hotkey registers without crash; Log confirms shortcuts active (or equivalent).
3. **E2E dictation:** Focus a text field in a native app (e.g. Gedit / Kate / browser) → hotkey dictation → transcript **pasted** (or, if paste fails, fails AC 4 only — paste is expected on X11).
4. **Paste path:** Successful inject reports pasted; text appears in focused field.
5. **Clipboard fallback:** Force or simulate paste failure (or use a non-pasteable target if needed) → text remains on clipboard; user-visible message.
6. **UI without relying on luck:** Window Start/Stop still works on X11 (regression).
7. **No crash** on shortcut rebind and app quit (unregister_all).

### 4.2 Wayland honesty (must pass as product behavior, not “full parity”)

8. **Docs state** pure Wayland global hotkeys are **not** guaranteed with current stack; link to session recommendation.
9. **If** hotkeys fail to register under Wayland: clear message + UI path still allows record → STT → **clipboard has text** (manual paste).
10. **No false advertising** in README “What’s next” / Linux section that claims full Wayland global hotkeys until re-verified after an upstream change.

### 4.3 Engineering quality

11. **Clipboard ownership:** A focused code path for Linux inject does not drop clipboard content before the paste target can read it (review + smoke).
12. **Tests:** Existing inject unit tests keep soft-skipping headless; add unit/doc tests for any new session-type helpers if introduced.

---

## 5. Suggested smoke matrix (for implementers / manual QA)

Record results in the PR (table can be filled once):

| # | Session | DE (example) | Hotkey | Paste into GTK text | Paste into browser | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | X11 | GNOME on Xorg / XFCE / etc. | ☐ | ☐ | ☐ | Hard gate |
| 2 | Wayland | GNOME Wayland | ☐ | ☐ | ☐ | Expect limits |
| 3 | Wayland | KDE Plasma Wayland | ☐ | ☐ | ☐ | Best-effort |
| 4 | Wayland | wlroots (Sway) | ☐ | ☐ | ☐ | Optional |

Minimum for merge of this slice: **row 1 all green** + docs for rows 2–3 limits.

---

## 6. Suggested implementation seams (non-binding)

| Area | Notes |
| --- | --- |
| `inject.rs` | Linux clipboard ownership / wait; keep Ctrl+V; improve error surfacing to status |
| `lib.rs` hotkey register | On `Err`, set flag + log + emit to frontend |
| `settings` / status snapshot | Optional `global_hotkeys_ok: bool`, `linux_session: …` |
| `Cargo.toml` | Optional `arboard` feature `wayland-data-control`; optional enigo wayland behind feature |
| README | Linux section: packages, X11 vs Wayland table, fallback UX |
| Manual QA | Fill smoke matrix in PR |

---

## 7. Out of scope

| Item | Why |
| --- | --- |
| Implementing XDG Desktop Portal Global Shortcuts client end-to-end | Upstream `global-hotkey` / ecosystem; dual stacks too large for this map ticket |
| Guaranteeing global hotkeys on all Wayland compositors | Impossible with current crate; honesty > aspirational AC |
| Accessibility-API style inject (AT-SPI full rewrite) | Beyond clipboard+paste strategy (INJ-02); OD-10 class |
| Windows packaging/hotkeys | Map out of scope |
| Full AppImage CI on every compositor | packaging-spec + this matrix are manual/dev |
| Changing default macOS inject path | Unrelated |
| Flatpak portal permission story as hard product | Note only if packaging later targets Flatpak |

---

## 8. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **DICT-01** | Global hotkey start/stop | Hard on X11; UI fallback when registration fails |
| **DICT-07** | Rebind hotkeys | Same; rebind errors must be clear on Linux |
| **INJ-01** | Insert into focused field when possible | X11 hard; Wayland best-effort |
| **INJ-02** | Clipboard + simulated paste | Unchanged strategy; Linux hardening |
| **INJ-03** | Paste fail → clipboard + notify | Mandatory on all Linux sessions |
| **INJ-05** | Best-effort app matrix | Smoke table |
| Persona **P-C** / Phase 2 | Linux productized | X11 committed path + documented Wayland limits |
| STATUS High | Wayland/X11 hotkey + paste | This contract |

---

## 9. Research appendix (facts for implementers)

Do not re-derive these from STATUS alone:

1. **`global-hotkey` 0.8.0** documents **Linux (X11 Only)**; platform impl path is `platform_impl/x11`. Wayland support issue remains open (portal/RFC history).
2. **Tauri global-shortcut** plugin lists Linux as supported at a high level but **inherits** `global-hotkey` limits; community notes Wayland unavailability.
3. **`enigo` 0.3** default = X11; Wayland/libei experimental behind features; may need **`libxdo-dev`** / distro packages for X11.
4. **`arboard`**: default X11; enable `wayland-data-control` for compositors implementing ext/wlr data-control; clipboard ownership requires long-lived process serving paste requests.
5. EagleScribe already implements paste-fail → clipboard keep + user message; Linux work is **reliability, messaging, docs, ownership**, not a new inject strategy.

---

## 10. Handoff

**Next step:** run **`/to-tickets`** against this document to publish tracer-bullet GitHub issues (label `ready-for-agent`), then implement with `/implement`.

Suggested ticket split (non-binding):

1. README Linux deps + X11/Wayland honesty table + session checks  
2. Hotkey registration failure → status/UI + no crash  
3. Linux clipboard ownership harden for inject/copy  
4. Optional: arboard `wayland-data-control` + session probe  
5. Manual X11 smoke checklist issue (fill matrix)

