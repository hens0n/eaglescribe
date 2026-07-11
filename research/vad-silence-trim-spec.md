# VAD / silence trim — behavior specification

**Status:** Ready for `/to-tickets`  
**Date:** 2026-07-10  
**Priority:** Medium (STT-05; STATUS suggested slice)  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`STT-05`, `OD-06`)

This document is the implementable product/behavior contract for **post-stop leading/trailing silence trim** before Whisper. It is not an implementation plan beyond acceptance criteria and intentional non-goals.

---

## 1. Goal

After the user **stops** a recording (dictation or Command Mode), EagleScribe **drops quiet padding** at the start and end of the captured audio before STT. That shortens Whisper work, reduces empty-pad hallucinations, and keeps the hotkey / hold / cancel UX unchanged.

This is **not** real-time voice activity detection, auto-stop on silence, or mid-utterance compression. “VAD” in the STATUS backlog name is product shorthand; the locked approach is **energy (RMS) based end-trim**.

Today: full capture buffer → resample to 16 kHz → Whisper with no silence handling.

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| When trim runs | **After stop, before Whisper** — never while the mic is still open |
| What is trimmed | **Leading and trailing silence only** — do not collapse mid-utterance pauses |
| Detector | **Simple energy / RMS threshold** (no neural VAD / Silero in this slice) |
| Default | **On** |
| User control | **Settings toggle** (persist in `settings.json`, same class as `clipboard_restore` / history) |
| Session kinds | **Dictation and Command Mode** — one setting for both |
| Edge pad | **Small fixed pad** kept before first and after last speech frame so consonants are not clipped |
| Near-empty after trim | **Fail with clear error** — no STT, no LLM, no inject; status → `error` (or equivalent clear failure path used today for empty capture) |
| Min remaining audio | **Hardcoded duration floor** after trim (exact ms is an implementation constant; order of ~100–200 ms) |
| Visibility | **Log only** — original duration, post-trim duration, and head/tail removed (or equivalent); no extra UI chrome |
| Mid-stream | Unchanged — cancel / Escape / hold-release behavior not altered by trim |

---

## 3. Behavior

### 3.1 Pipeline position

On a normal stop-into-transcribe path:

1. Stop mic → obtain mono PCM + sample rate (as today).
2. Resample to **16 kHz mono** (as today), unless implementers prove trim-at-native-rate is equivalent — **acceptance is on the audio Whisper receives**.
3. If silence trim is **enabled** → run leading/trailing energy trim (+ pad) on that buffer.
4. If remaining duration **&lt; minimum floor** (or buffer empty) → **fail**: no Whisper, no polish/dictionary/snippets/inject/history write for this take; clear log + status error (e.g. “No speech detected” / equivalent).
5. Else → Whisper → rest of pipeline unchanged.

If silence trim is **disabled**, step 3 is skipped (today’s path). Empty capture before any trim still fails as today (“No audio captured…”).

### 3.2 What counts as silence

- Frame-based energy / RMS vs a threshold (and optional hangover for stability — implementation detail).
- Frames below threshold at the **start** are candidates for head trim; at the **end**, for tail trim.
- Frames in the **middle** of the utterance are **never** removed in this feature, even if quiet for a long time.

Exact frame size, threshold, and dB/linear scale are **implementation constants** tunable in code/tests; they are not user-facing settings in v1. They must be stable enough that normal desktop speech with a short pre-roll is not over-trimmed into the fail path.

### 3.3 Edge pad

After finding the first and last “speech” frames:

- Keep an additional **fixed pad** of audio before the first speech frame and after the last (clamped to buffer bounds).
- Recommended order of magnitude: **~50–150 ms** per side; exact value is a code constant.

Pad exists so trim does not shave plosives / word onsets and offsets.

### 3.4 Settings

- Toggle label intent: **trim silence** / **silence trim** (exact copy flexible).
- Default: **on** for new installs and missing field (serde default `true`).
- Persist under OS app data `settings.json` with other prefs.
- Corrupt / unknown value → treat as default **on**.
- Changing the toggle affects the **next** completed recording (no mid-stream change).

### 3.5 Dictation and Command Mode

- Both use the same trim setting and the same trim step before Whisper.
- Command Mode: trim applies only to the **spoken instruction** audio, not to selected text capture.
- Cancel / Escape before stop: no audio proceeds; trim does not run (same as no STT today).

### 3.6 Failure: no speech after trim

When trim is enabled and remaining audio is below the minimum floor:

| Must | Must not |
| --- | --- |
| Status reflects failure (`error` or clear idle+error log consistent with other capture failures) | Call Whisper |
| Log a clear reason (no speech / all silence after trim) | Call local LLM (Command Mode) |
| Leave clipboard / focused app unchanged | Inject text |
| | Append a successful history entry |

User can immediately record again. This is **not** treated as Escape cancel (no “Recording cancelled” semantics required), but the end state for inject/LLM is the same: nothing pasted.

### 3.7 Logging (when trim runs and succeeds)

At least one log line (or structured equivalent) including:

- Duration (or sample count) **before** trim  
- Duration **after** trim  
- Amount removed from **head** and **tail** (ms or samples)

When trim is disabled, no trim-specific log is required. When trim fails the min-floor check, log that path explicitly.

### 3.8 Interaction with other features

| Feature | Interaction |
| --- | --- |
| Mic device picker | Independent — trim runs on whatever buffer was captured |
| Escape cancel | Unchanged; cancel never reaches trim |
| Polish / dictionary / snippets | Unchanged; still after Whisper |
| History | Only on successful path that produces a transcript (as today) |
| Clipboard restore | Unchanged; only after successful inject |

---

## 4. Acceptance criteria

An implementation is done when all of the following pass on **macOS** (daily driver); Linux should follow the same contract where capture works.

1. **Default on:** Fresh settings (or missing field) → silence trim enabled; a take with long quiet pre-roll still yields correct dictation text.
2. **Leading trim:** Record with ≥1 s silence, then speech, then stop → log shows head removed; Whisper receives shorter audio; text still correct for the spoken words.
3. **Trailing trim:** Speech then ≥1 s silence before stop → log shows tail removed; text still correct.
4. **Mid pauses preserved:** Speech, long pause, more speech in one take → both phrases still in the transcript (internal silence not collapsed).
5. **Toggle off:** Disable in Settings → save → next take keeps full buffer path (log shows no trim / full duration matches capture); still end-to-end OK.
6. **All silence:** With trim on, record only silence long enough to open a session → fail with clear error; **no** paste; **no** Whisper success path.
7. **Command Mode:** Same trim setting applies to Command Mode instruction audio; happy path still rewrites + injects when LLM is up.
8. **Cancel / Escape:** Cancel mid-recording still discards audio without trim/STT/inject.
9. **Pad:** Spoken words starting immediately are not systematically clipped into wrong text solely due to zero-pad hard cuts (pad present; qualitative check acceptable).

---

## 5. Suggested implementation seams (non-binding)

Pointers only — implementers may choose equivalent structure:

| Area | Notes |
| --- | --- |
| Pure function | `trim_silence_16k(samples, config) -> TrimResult { samples, head_ms, tail_ms }` unit-testable without mic |
| `settings` | e.g. `silence_trim: bool` default `true` |
| `state` stop path | After resample, before `transcribe_16k_mono`; both dictation and command |
| UI Settings | Toggle + persist with other Settings controls |
| Tests | Synthetic sine/speech-like bursts with known silence pads; empty/near-empty → error path |

---

## 6. Out of scope

| Item | Why |
| --- | --- |
| Real-time silence endpointing (auto-stop) | Different product; STATUS hands-free / DICT-06 territory |
| Neural VAD (Silero, etc.) | OD-06 alternative deferred; energy is enough for pad trim |
| Collapsing internal pauses | Locked out for v1 |
| User-tunable threshold / pad / min duration | YAGNI; constants in code |
| Streaming / partial transcripts | STT-08 later |
| Writing debug WAVs by default | Privacy; only if existing optional debug path is reused |
| Per-mode different trim settings | One setting for dictation + Command Mode |

---

## 7. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **STT-05** | Optional VAD or silence detection to trim ends | Post-stop leading/trailing energy trim + toggle |
| **OD-06** | Energy vs Silero | **Energy** locked for this slice |
| **D1** | Local by construction | No network; no extra model download |
| **D4** | Graceful degradation | All-silence → clear error, no junk paste |
| STATUS gap | “VAD / silence trim” | Trim before Whisper; log trimmed duration |

---

## 8. Handoff

**Next step:** run **`/to-tickets`** against this document to publish tracer-bullet GitHub issues (label `ready-for-agent`), then implement the frontier with `/implement`.

**Do not** expand into Silero, auto-stop endpointing, or mid-utterance compression without a new product decision.

