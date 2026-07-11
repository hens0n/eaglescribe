# Escape cancel — behavior specification

**Status:** Implemented (issues #1–#3)  
**Date:** 2026-07-10  
**Priority:** Medium (DICT-03 / P0 in requirements; STATUS default Mac slice)  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`DICT-03`)

This document is the implementable product/behavior contract for **global Escape cancel while recording**. It is not an implementation plan beyond acceptance criteria and intentional non-goals.

---

## 1. Goal

While EagleScribe is in **`recording`**, pressing **Escape** aborts the capture the same way as the UI **Cancel** control: discard audio, do not run STT, do not call the local LLM, do not inject text.

When EagleScribe is **not** recording, Escape must remain available to the rest of the system (editors, browsers, dialogs).

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| Scope while recording | **Always global** — Escape cancels regardless of which app is focused |
| Pipeline phase | **`recording` only** — not `transcribing`, not `waiting_llm` |
| Session kinds | **Dictation and Command Mode** |
| Cancel key | **Fixed Escape** — not user-rebindable in settings |
| Arming | **Register only while `status === recording`**; unregister on every exit from recording |
| Hold-to-talk | Escape **cancels** (does not act like release); suppress chord until full key-up |

---

## 3. Behavior

### 3.1 When Escape is armed

Escape is armed **if and only if** `DictationStatus` is `Recording`.

- Enter recording (dictation start or Command Mode start) → **register** a global Escape shortcut.
- Leave recording by any path below → **unregister** Escape (idempotent if already unregistered).

**Leave-recording paths that must disarm Escape:**

1. **Cancel** (Escape or UI Cancel)  
2. **Stop / release** that begins transcription (`transcribing`)  
3. Any error path that drops the session back to `idle` or `error` without staying in `recording`  
4. App teardown / quit (no leak of a global Escape handler)

Escape must **not** remain registered across `transcribing` or `waiting_llm`.

### 3.2 On Escape (armed)

Behavior must match existing `cancel_recording` / `cancel_dictation` semantics:

1. If status is not `recording` → no-op at the product level (handler should not be registered; if raced, ignore or return the same “Not recording” error without changing state).
2. Drop the active mic/session; no samples proceed to Whisper.
3. Clear Command Mode session state used for the rewrite (`command_selection`, command session kind → default), consistent with today’s cancel.
4. Set status to **`idle`**.
5. Append a clear log line (today: `"Recording cancelled."` — keep or equivalent).
6. Emit status to the UI so the badge/button state updates without a manual refresh.
7. **Do not** inject, polish, write history, or call the LLM.

### 3.3 Hold / toggle interaction

Applies to dictation **hold** mode and Command Mode’s press/release chord.

| Event order | Required result |
| --- | --- |
| Hotkey down → recording → **Escape** | Cancel (discard). Status `idle`. |
| Escape while hotkey still held | Subsequent **hotkey release must not** stop-into-transcribe or re-start a session. |
| Escape → user fully releases chord → presses again later | Normal start behavior resumes. |

**Implementation intent (normative for acceptance):** after Escape cancel, ignore dictation/command hotkey **Released** (and spurious Pressed if needed) until a clean full release of that chord is observed — same *class* of suppress/debounce already used for Command Mode synthetic copy (`command_ignore_release_until` / release suppression). Dictation hold needs an equivalent if it does not already have one.

### 3.4 Command Mode specifics

- Escape during Command Mode **recording** aborts the spoken-instruction capture.
- Selection was already captured (synthetic copy) before recording began; **this feature does not restore the prior clipboard** (see Out of scope).
- No localhost LLM request; no inject of a rewrite.

### 3.5 Interaction with Settings hotkey capture

- Settings rebind UI already uses **window-level** Escape to cancel capture (`Esc to cancel` in the capture prompt). That path is unchanged.
- Rebind capture and an active mic recording are not a supported concurrent workflow; no special multi-handler merge is required beyond: **global Escape is only registered while `recording`**, and capture UI only listens while `captureTarget` is set in the webview.

### 3.6 Fixed key and hotkey conflicts

- Cancel is always **Escape** (no settings field).
- **Should-reject (implement with or immediately after Escape cancel):** refuse binding dictation or Command Mode to **Escape alone** (Esc with no modifiers), so start and cancel cannot share one key.
- Chords that merely *include* Esc with modifiers (e.g. `Ctrl+Shift+Esc`) are not banned in this spec; optional hardening can come later.

### 3.7 Idle and non-recording

| Status | Escape |
| --- | --- |
| `idle`, `error` | Not registered by EagleScribe |
| `transcribing`, `waiting_llm` | Not registered; Escape does **not** abort processing |
| `recording` | Global cancel |

UI **Cancel** remains available whenever the UI enables it for recording (parity with Escape for the recording phase only).

---

## 4. Acceptance criteria

An implementation is done when all of the following pass on **macOS** (daily driver); Linux should follow the same contract where global shortcuts work.

1. **Dictation + toggle:** Start recording → press Escape (focus in another app) → status `idle`, no paste, log shows cancel.  
2. **Dictation + hold:** Hold dictation hotkey → Escape before release → no transcript/inject; releasing the hotkey afterward does not start processing.  
3. **Command Mode:** Start command recording → Escape → no LLM call, no inject; status `idle`.  
4. **Arming:** With app idle, Escape in Terminal/Vim/browser is **not** consumed by EagleScribe.  
5. **Phase:** During `transcribing` (and Command `waiting_llm`), Escape does not cancel (optional: still free for other apps).  
6. **UI Cancel:** Still works; Escape and Cancel produce the same end state for an active recording.  
7. **Status UI:** Badge/actions update promptly after Escape without reopening the window.  
8. **Teardown:** Quit while recording does not leave a system-wide Escape grab after the process exits.

---

## 5. Suggested implementation seams (non-binding)

Pointers only — implementers may choose equivalent structure:

| Area | Notes |
| --- | --- |
| `state::cancel_recording` | Keep as single cancel primitive; Escape and UI both call it. |
| `lib.rs` hotkey registration | Today `register_app_hotkeys` registers dictation + command. Extend with dynamic Escape register/unregister on recording enter/leave, **or** a dedicated helper invoked from start/cancel/stop paths. |
| Status transitions | Every transition **out of** `Recording` must disarm Escape; every transition **into** `Recording` must arm it. Prefer centralizing in status-set helpers to avoid missed paths. |
| Hold suppress | Mirror Command Mode release suppression for dictation after cancel. |
| Tests | Unit-test cancel state machine + “ignore release after cancel” if extractable; manual checklist for global shortcut / focus. |

Platform note: global shortcuts may require the same macOS **Accessibility / Input Monitoring** permissions already needed for paste simulation. Failures to register Escape should log clearly without bricking dictation start.

---

## 6. Out of scope

| Item | Why |
| --- | --- |
| Abort during `transcribing` / `waiting_llm` | Separate cancellation-token feature |
| Rebindable cancel key | Settings surface; not needed for DICT-03 |
| Window-focused-only Escape | Rejected — useless when dictating into other apps |
| Clipboard restore after Command Mode / inject | Separate STATUS slice |
| Always-on global Escape with no-op handler | Rejected — risks stealing Escape when idle |
| Full ban of Esc inside multi-key chords | Optional later hardening |
| Linux Wayland hotkey matrix | Separate Linux pass; contract is the same, delivery may lag |

---

## 7. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **DICT-03** | User can **cancel** an in-progress capture (e.g. Escape) without pasting | Escape + UI Cancel while `recording` |
| **DICT-04** | Clear state idle / listening / processing / error | Cancel → `idle` + emit |
| STATUS gap | “Global Escape cancel; define scope” | Scope = global only while recording |

---

## 8. Handoff

**Implemented** (issues #1–#3): global Escape arm/disarm while `recording`, hold-safe release suppress (cleared on next release **or** new session start), Escape-alone rebind rejection.

**Do not** expand into STT/LLM abort or clipboard restore without a new product decision.
