# Mic device picker — behavior specification

**Status:** Ready for `/to-tickets`  
**Date:** 2026-07-10  
**Priority:** Medium (STATUS default Mac slice; SET-01)  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`SET-01`, `STT-04`)

This document is the implementable product/behavior contract for **choosing which microphone EagleScribe records from**. It is not an implementation plan beyond acceptance criteria and intentional non-goals.

---

## 1. Goal

Users with more than one input device (built-in mic, headset, USB, virtual cables) can **see available microphones**, **pick one**, and have EagleScribe use that device for the next dictation or Command Mode recording. If the preferred device is missing, the app falls back to the system default and makes that visible — it does not silently fail or capture from a surprise device without feedback.

Today capture always uses `cpal` **default input** only (`RecordingSession::start` with no device argument).

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| Scope | **One preferred input** shared by dictation and Command Mode |
| Persistence | Save in `settings.json` under OS app data (same path as other prefs) |
| Default | **System default** when unset, empty, or first run (today’s behavior) |
| When selection applies | **Next recording start only** — no mid-stream device switch |
| Enumeration | Via **cpal** host input devices; refresh on demand from Settings |
| Missing preferred device | **Fall back to system default** for that session; log + surface in UI |
| Identity | Persist a **stable-enough device name** (and optional host-local id if available); match preferred name first, then fall back |
| UI home | **Settings** tab — list + save, consistent with hotkey/LLM prefs |
| Privacy | Device names/ids stay local; no network |

---

## 3. Behavior

### 3.1 Enumeration

- The app can list **input** devices from the current cpal host.
- Each row shows a human-readable **name** (and may carry an opaque id for matching).
- List includes a first option: **System default** (do not pin a device; use host default at start time).
- Enumeration failures (no devices, permission, host error) produce a clear error message; they must not crash the app or leave Settings unusable for other prefs.

### 3.2 Persist preference

- User selects a device (or System default) and saves (same pattern as other Settings saves: explicit save or immediate persist — match existing Settings UX for LLM/hotkeys).
- Preference survives app restart via `settings.json`.
- Unknown / corrupt preference field → treat as System default (serde defaults; no hard fail on load).

### 3.3 Recording start

When a recording starts (dictation or Command Mode):

1. If preference is **System default** (or unset) → open capture on host **default input** (current behavior).
2. If preference is a **named device** → resolve against the current device list:
   - **Found** → open capture on that device.
   - **Not found** → fall back to system default for **this** session; log a clear line (e.g. preferred mic unavailable, using default); status path still proceeds if default works.
3. If **no** input device exists at all → fail with the same class of error as today (“No default microphone found” / permissions), surfaced in status/log.

### 3.4 Settings UI

Minimum:

- Current preferred device shown when Settings is open.
- Control to **refresh** the device list (devices plug/unplug).
- Control to choose **System default** or a specific device.
- After save, the next recording uses the new preference (no restart required).

Optional polish (same ticket if cheap; else follow-on): show which device was actually used on the last recording when fallback occurred.

### 3.5 Permissions and empty capture

- macOS mic permission failures remain clear (“No audio captured — check microphone permissions”).
- Picking a different device does not change the permission story; user may need to grant access per OS rules.

### 3.6 Interaction with pipelines

- Dictation and Command Mode **both** honor the same preferred input.
- Cancel, Escape, hold/toggle, STT, polish, inject, history — **unchanged**.
- Device choice does not alter sample-rate/resample-to-16 kHz contract after capture.

---

## 4. Acceptance criteria

An implementation is done when all of the following pass on **macOS** (daily driver); Linux should follow the same contract where cpal enumeration works.

1. **List:** Settings shows at least System default plus any host input devices cpal reports.
2. **Select + persist:** Choose a non-default device → save → quit → relaunch → preference still selected.
3. **Use preferred:** With preferred device present, a dictation recording captures from that device (verifiable by speaking into only that mic, or by log of resolved device name).
4. **Command Mode:** Same preferred device is used for Command Mode recording.
5. **System default:** Selecting System default restores host-default capture (parity with pre-feature behavior).
6. **Missing device:** Unplug / rename preferred device → start recording → falls back to default without hanging; log (and ideally UI) indicates fallback.
7. **Refresh:** Plug a new input → refresh list → new device appears without restarting the app.
8. **No regression:** With a single mic / default only, hotkey dictation still works end-to-end as before.

---

## 5. Suggested implementation seams (non-binding)

Pointers only — implementers may choose equivalent structure:

| Area | Notes |
| --- | --- |
| `audio` module | `list_input_devices()`; `RecordingSession::start(preferred: Option<DevicePref>)` resolving name → cpal `Device` |
| `settings` | Field e.g. `input_device_name: Option<String>` (empty = system default); load/save already JSON |
| Tauri commands | `list_mic_devices`, extend status/settings snapshot with current preference; `set_input_device` |
| `state` | Pass preference into both dictation and command `RecordingSession::start` call sites |
| UI Settings | Select + refresh + save wiring in Settings tab |
| Tests | Unit-test preference resolve: exact match, missing → default, empty → default; no need for real hardware in CI |

---

## 6. Out of scope

| Item | Why |
| --- | --- |
| VAD / silence trim | Separate STATUS slice |
| Per-mode different mics (dictation vs Command) | YAGNI for v1 |
| Mid-recording device switch | Stream rebuild complexity; next-start is enough |
| Output / playback device selection | Dictation is input-only |
| Windows device matrix | Not primary platform yet |
| Automatic “best” mic heuristics | Explicit user choice only |
| Cloud / remote mics | Privacy invariant |

---

## 7. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **SET-01** | Settings UI: model path, hotkey, polish on/off, **mic device** | Mic list + persist + use on record |
| **STT-04** | Mic capture via cpal at quality suitable for Whisper | Still cpal; device selectable |
| **D4** | Graceful degradation | Missing preferred → default + clear feedback |
| STATUS gap | “Mic device picker” | Enumerate, persist, default fallback |

---

## 8. Handoff

**Next step:** run **`/to-tickets`** against this document to publish tracer-bullet GitHub issues (label `ready-for-agent`), then implement the frontier with `/implement`.

**Do not** expand into VAD, tray polish, or clipboard restore without a new product decision.

