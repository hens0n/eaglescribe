# Onboarding & permissions copy — product specification

**Status:** Ready for `/to-tickets`  
**Date:** 2026-07-10  
**Priority:** Low in STATUS backlog order; **P0** in requirements for SET-02 / INJ-06 completeness  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`SET-02`, `INJ-06`, `D8`) · [linux-hotkey-paste-spec.md](./linux-hotkey-paste-spec.md) · [packaging-spec.md](./packaging-spec.md)

This document is the implementable product/behavior contract for **first-run onboarding** and **permissions guidance** (mic, Accessibility, related notes). It is not an implementation plan beyond acceptance criteria and intentional non-goals.

---

## 1. Goal

A new user (or anyone who skipped setup) can see a **short, honest checklist** of what EagleScribe needs to work—**microphone**, **Accessibility** (macOS paste/copy simulation), and a **Whisper model**—without a blocking multi-step wizard. When capture or inject fails later, the app surfaces the **same guidance in context**, not only on first launch.

Today: error strings exist (“check microphone permissions”); README mentions mic (+ Accessibility later); **no** dedicated first-run checklist or Settings help hub.

Privacy stance unchanged: permissions are **minimal and explained** (D8); no screen recording; no cloud.

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| When guidance appears | **Both** first-run checklist **and** failure-time contextual help |
| Shape | **Dismissible in-app checklist** (banner/card/panel) — **not** a blocking wizard that gates the main UI |
| Dismiss memory | Persist **`onboarding_dismissed`** (name flexible) in `settings.json`; default false / missing → show on first useful launch |
| Re-open | Always available from **Settings** (e.g. Help / Permissions section) |
| After dismiss | Failure-time help **still** shows; checklist does not auto-nag every launch |
| macOS checklist rows | **Microphone**, **Accessibility**, **Whisper model** |
| Input Monitoring | **Secondary note** under Accessibility or hotkeys — not a third mandatory equal gate |
| Model row | Points at **existing** path / Load / `npm run model:download` — **no** new download wizard in this slice |
| System Settings | Prefer **deep links**; fallback **manual path** text if open fails |
| Grant detection | **Optional** (nice-if-cheap); **not** required to complete the checklist or dismiss |
| Linux | **Short honest notes** only (packages / session / clipboard) — **no** fake macOS Accessibility copy |
| Windows | Out of scope (map default) |
| Screen capture / full disk | **Never** requested or implied |

---

## 3. Behavior

### 3.1 First-run checklist (macOS)

**When to show (auto):**

- App launches (or main window first shown) **and** `onboarding_dismissed` is false/missing.
- Showing the checklist must **not** block:** hotkeys (if registered), Settings tabs, Load model, or dictation attempts. User can ignore it and still try the app.

**Contents (minimum rows):**

| Row | Intent | Primary action |
| --- | --- | --- |
| **Microphone** | Capture needs mic access | Button: open Privacy → Microphone (deep link if possible) |
| **Accessibility** | Paste/copy simulation needs Accessibility | Button: open Privacy → Accessibility |
| **Whisper model** | STT needs a local ggml model | Point to Settings model path + Load; mention `npm run model:download` / README |

**Secondary copy (not a separate hard gate):**

- **Input Monitoring** may be needed for reliable **global shortcuts** on some macOS versions — one short sentence under Accessibility or a “Global hotkeys” footnote. Do not force a third System Settings pilgrimage as required for “setup complete.”

**Dismiss:**

- Controls: **Got it** / **Skip** / **Don’t show again** (exact labels flexible).
- Sets `onboarding_dismissed: true` immediately.
- Corrupt/missing field → treat as not dismissed (show checklist).

**Optional status chips (if cheap):**

- Model: use existing `model_loaded` / path presence.
- Mic / Accessibility: only if APIs are easy; otherwise static “Required for …” without green checkmarks.

### 3.2 Settings re-entry

Under Settings (dedicated subsection is fine):

- **Show setup checklist** / **Permissions help** control that opens the same content even when dismissed.
- Same deep links and manual paths as first-run.
- Linux builds: show the Linux note set (below), not macOS-only rows as if they applied.

### 3.3 Failure-time contextual help

| Failure | Help to show |
| --- | --- |
| No / empty capture, mic permission style errors | Microphone steps (+ link to System Settings) |
| Paste or selection-copy simulation fails (inject path) | Accessibility steps + reminder text stays on **clipboard** for manual paste (INJ-03 / existing behavior) |
| Model missing / load failure | Existing model path errors + pointer to model checklist row / README |
| Global hotkey registration failure (if surfaced) | Secondary Input Monitoring note + “use UI controls”; on Linux defer to linux-hotkey-paste-spec messaging |

Help may be: expandable status line, Log entry + short toast/banner, or inline Settings callout. **Must be user-visible**, not only `eprintln!`.

Failure help **ignores** `onboarding_dismissed` (still show).

### 3.4 Opening System Settings (macOS)

- Prefer documented URL / `open` schemes for:
  - Microphone privacy
  - Accessibility privacy  
  Exact URLs may vary by macOS version — implementer verifies on a current macOS; if a link breaks, fall back to manual path.
- Manual path text (always available under the buttons), e.g.:
  - **System Settings → Privacy & Security → Microphone** → enable EagleScribe  
  - **System Settings → Privacy & Security → Accessibility** → enable EagleScribe  
- Opening Settings must not crash if the URL fails; show manual path instead.

### 3.5 Linux (and non-macOS)

No Accessibility checklist rows.

Minimum in-app or Settings help:

- Microphone access / PipeWire-Pulse as relevant to the host.
- Global hotkeys and paste reliability depend on **session type** (X11 vs Wayland) and packages — short bullets + pointer to README / [linux-hotkey-paste-spec.md](./linux-hotkey-paste-spec.md).
- On paste failure: text remains on clipboard; paste manually.
- First-run dismiss + Settings re-open still apply so Linux users can hide the tip card.

### 3.6 Tone and privacy

- Explain **why** each permission: “so we can hear you,” “so we can paste into other apps.”
- Explicitly **do not** request or describe screen recording, full disk access, or network for core dictation.
- Local-first one-liner may appear once on the checklist: audio and transcripts stay on this machine for default dictation (Command Mode uses user-configured localhost LLM only).

### 3.7 Interaction with other features

| Feature | Interaction |
| --- | --- |
| Mic device picker | Independent; mic permission still required for any device |
| Inject / clipboard restore | Failure help for paste; restore rules unchanged |
| Escape cancel / hotkeys | Input Monitoring note only; Escape registration failures already log |
| Packaging | Unsigned Gatekeeper is packaging-spec, not this checklist (optional one-line cross-link in README only) |
| Model download | No new bundling; packaging-spec: models not required in dmg |

---

## 4. Acceptance criteria

An implementation is done when all of the following pass on **macOS** (daily driver). Linux criteria are docs/UI honesty.

1. **First show:** Fresh settings (no dismiss flag) → checklist visible without blocking main UI or Settings.
2. **Dismiss:** Got it / Skip → flag persisted → relaunch → checklist does **not** auto-show.
3. **Re-open:** Settings control shows the same checklist content after dismiss.
4. **Mic row:** Deep link or manual path for Microphone privacy is present and usable.
5. **Accessibility row:** Same for Accessibility privacy.
6. **Model row:** Points user at existing model path/Load (and/or download script docs); no requirement for a new downloader UI.
7. **Input Monitoring:** Appears only as secondary note, not as a third required peer row.
8. **Mic failure help:** Trigger empty-capture / permission-style failure → user-visible mic guidance (even if onboarding was dismissed).
9. **Paste failure help:** On inject paste failure path → user-visible Accessibility guidance (and clipboard still holds text per existing inject rules).
10. **Linux:** No macOS Accessibility deep links presented as applicable; session/package/clipboard notes present in help surface or README cross-link.
11. **No regression:** Dictation still works for a user who already granted permissions and loaded a model, with checklist dismissed.
12. **Privacy:** No new permission types (screen recording, etc.) requested by this slice.

---

## 5. Suggested implementation seams (non-binding)

| Area | Notes |
| --- | --- |
| `settings` | `onboarding_dismissed: bool` default `false` |
| UI | Checklist component + Settings “Permissions / Setup” section |
| macOS open | `open` URL / `NSWorkspace` for privacy panes; catch errors |
| Status / inject / audio | On known errors, emit a structured hint code the UI maps to copy |
| Copy source | Keep strings in one module or frontend constants for consistency first-run vs failure |
| Tests | Serde default for dismiss flag; pure string/hint mapping unit tests |

---

## 6. Out of scope

| Item | Why |
| --- | --- |
| Blocking multi-page wizard | Locked out — power-user friendly |
| Live AX/mic grant polling as hard requirement | Optional only |
| In-app Whisper model CDN / progress downloader | SET-02 satisfied via path + existing script |
| Input Monitoring as mandatory third gate | Secondary note only |
| Screen Recording / Full Disk / network “permission” UX | Not needed for default path |
| Full Linux compositor wizard | linux-hotkey-paste-spec owns reliability; this is copy only |
| Windows onboarding | Map default |
| Changing inject strategy (AT-SPI, etc.) | OD-10 / separate |

---

## 7. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **SET-02** | First-run: model, mic, accessibility guide | Checklist rows + dismiss/re-open |
| **INJ-06** | Guide Accessibility if inject fails | Failure-time help |
| **D8** | Permissions minimal & explained | Mic + Accessibility; no screen capture |
| **INJ-03** | Paste fail → clipboard + notify | Unchanged; help pairs with notify |
| **NFR-07** | Keyboard-operable settings | Checklist/Settings must remain usable via UI |
| STATUS Low | Onboarding / permissions copy | This contract (not deferred) |

---

## 8. Handoff

**Next step:** run **`/to-tickets`** against this document to publish tracer-bullet GitHub issues (label `ready-for-agent`), then implement with `/implement`.

Suggested ticket split (non-binding):

1. Settings flag + first-run checklist UI + dismiss/re-open  
2. macOS deep links + manual path copy (Mic + Accessibility)  
3. Wire failure paths (audio empty / inject paste fail) to same copy  
4. Linux help notes in Settings  

**Do not** expand into a full marketing onboarding or model store without a new product decision.

