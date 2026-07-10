# TalonType — Requirements for a Fully Local Dictation App

**Status:** Working requirements (v1 restore)  
**Date:** 2026-07-10  
**Derived from:** [wispr-flow.md](./wispr-flow.md)  
**Stack ADR:** [stack-decision.md](./stack-decision.md)

---

## 1. Product thesis

**TalonType** is a system-wide voice dictation app for people who want **Wispr Flow–class speed and polish** without sending audio or transcripts to a vendor cloud.

**One-liner:** Speak → on-device STT → local polish → paste into any app. Offline by default.

**Core promise:**

> Nothing you say leaves your machine on the default path. No account required for core dictation. Unlimited words, hardware-bound only.

**Non-goals (v1):**

- Matching Flow’s full multi-platform mobile suite on day one  
- Enterprise SSO / admin portal  
- Cloud sync of history  
- Guaranteeing Flow-level multilingual quality for 100+ languages out of the box  

---

## 2. Design principles

| # | Principle | Meaning |
| --- | --- | --- |
| D1 | **Local by construction** | STT and polish run on-device; network is never required for the happy path |
| D2 | **Honesty over marketing privacy** | No “Privacy Mode” that still uploads audio; claims match architecture |
| D3 | **Flow-familiar UX** | Hold/toggle hotkey, inject text, polish fillers—users should feel oriented |
| D4 | **Graceful degradation** | If paste fails → clipboard; if model missing → clear setup; if GPU missing → CPU |
| D5 | **Small surface, deep reliability** | Prefer one rock-solid dictation loop over half-finished Hub features |
| D6 | **User-owned models & data** | Models on disk; dictionary/history in OS config dir; exportable |
| D7 | **Rules first, LLM second** | Deterministic polish before shipping local LLM Command Mode |
| D8 | **Permissions minimal & explained** | Mic + accessibility only when needed; no silent screen capture |

---

## 3. Personas

### P-A — Privacy-conscious professional

Uses Slack, email, docs with client or regulated content. Distrusts cloud STT even with “ZDR.” Wants unlimited dictation without a subscription.

**Success:** Dictates a sensitive email offline; verifies no network calls during session.

### P-B — Developer / power user

Mac or Linux, lives in terminal + IDE + browser. Wants commit messages, PR text, LLM prompts via voice. Comfortable downloading models.

**Success:** Hotkey in Cursor/VS Code; dictionary for project names; low latency on Apple Silicon / NVIDIA.

### P-C — Linux desktop user

Flow does not serve them well. Needs Wayland/X11-aware hotkeys and paste.

**Success:** End-to-end dictation on a modern Linux desktop without cloud.

### P-D — Accessibility-first user

Prefers speech over long keyboard sessions. Needs reliable hands-free mode and clear status.

**Success:** Toggle listen, long-form notes, visible listening/processing state.

### P-E — Flow refugee (cost / limits)

Hit free word cap or refuses $12–15/mo. Wants “good enough” polish offline.

**Success:** Daily unlimited use after one-time model download.

---

## 4. Functional requirements

Priority legend:

| Priority | Definition |
| --- | --- |
| **P0** | MVP must-have; ship-blocker |
| **P1** | Near-term product completeness |
| **P2** | Competitive parity with Flow differentiators |
| **P3** | Stretch / later phases |

### 4.1 Dictation session control

| ID | Requirement | Priority |
| --- | --- | --- |
| **DICT-01** | User can start/stop listening via a **global hotkey** (toggle mode) | P0 |
| **DICT-02** | User can use **push-to-talk** (hold to speak, release to process) | P1 |
| **DICT-03** | User can **cancel** an in-progress capture (e.g. Escape) without pasting | P0 |
| **DICT-04** | App shows clear state: idle / listening / processing / error | P0 |
| **DICT-05** | Listening auto-stops after a configurable max duration (default ≤ 10–20 min) | P2 |
| **DICT-06** | Double-tap or dedicated shortcut for hands-free equivalent of Flow | P2 |
| **DICT-07** | User can rebind hotkeys in settings | P1 |
| **DICT-08** | Hotkeys avoid common OS reserved combos; warn on conflicts | P1 |

### 4.2 Text injection

| ID | Requirement | Priority |
| --- | --- | --- |
| **INJ-01** | Final text is inserted into the **currently focused** text field when possible | P0 |
| **INJ-02** | Primary strategy: **clipboard + simulated paste** (Cmd/Ctrl+V) | P0 |
| **INJ-03** | If paste fails, text remains on clipboard and user is notified | P0 |
| **INJ-04** | Optional: restore previous clipboard contents after paste (configurable) | P1 |
| **INJ-05** | Works across browsers, native apps, Electron/Tauri apps (best-effort matrix) | P0 |
| **INJ-06** | On macOS, guide user to grant **Accessibility** if inject fails | P0 |
| **INJ-07** | “Paste last transcript” secondary hotkey | P2 |
| **INJ-08** | Optional simulated Enter after paste (“press enter” style) for chat UIs | P3 |

### 4.3 Speech-to-text (local)

| ID | Requirement | Priority |
| --- | --- | --- |
| **STT-01** | Transcription runs **entirely on-device** using whisper.cpp (or successor) | P0 |
| **STT-02** | User can select / path-configure a local ggml/gguf Whisper model | P0 |
| **STT-03** | Bundled or one-click download of a small default English model | P0 |
| **STT-04** | Mic capture via `cpal` (or equivalent) at quality suitable for Whisper | P0 |
| **STT-05** | Optional VAD or silence detection to trim ends | P1 |
| **STT-06** | Acceleration: Metal (macOS AS), CUDA/Vulkan (Linux), CPU fallback | P1 |
| **STT-07** | Multilingual model support (user-selected) | P2 |
| **STT-08** | Streaming / partial transcripts in UI (nice-to-have) | P3 |
| **STT-09** | No audio written to network; optional local debug WAV only if user enables | P0 |
| **STT-10** | Model load status and errors surfaced in UI | P0 |

### 4.4 Polish (Smart Formatting / Backtrack)

| ID | Requirement | Priority |
| --- | --- | --- |
| **POL-01** | Strip common English fillers (um, uh, like-as-filler—conservative list) | P0 |
| **POL-02** | Basic auto-punctuation and capitalization | P0 |
| **POL-03** | Normalize whitespace / paragraph breaks from pauses if feasible | P1 |
| **POL-04** | Spoken list cues → bullets/numbers (simple heuristics) | P2 |
| **POL-05** | **Backtrack**: recognize correction phrases (“scratch that”, “no I mean …”) | P1 |
| **POL-06** | Style presets: Raw / Clean / Formal (rules-based) | P2 |
| **POL-07** | Optional local LLM rewrite pass (user opt-in, model path) | P2 |
| **POL-08** | Polish can be disabled (raw STT only) | P0 |
| **POL-09** | User-defined polish rules (regex / phrase replace) | P3 |

### 4.5 Command Mode (local)

| ID | Requirement | Priority |
| --- | --- | --- |
| **CMD-01** | Separate hotkey: voice command transforms **selected** text | P2 |
| **CMD-02** | Commands run via **local LLM** (llama.cpp) when model configured | P2 |
| **CMD-03** | Without selection, insert model response at cursor / clipboard | P2 |
| **CMD-04** | Cancel in-flight command | P2 |
| **CMD-05** | Clear errors if no LLM model loaded | P2 |
| **CMD-06** | Built-in command templates: shorten, expand, fix grammar, translate | P3 |
| **CMD-07** | Max selection length guard for latency | P3 |

### 4.6 Dictionary & snippets

| ID | Requirement | Priority |
| --- | --- | --- |
| **DICTN-01** | User can add custom words / proper nouns (forced spellings) | P1 |
| **DICTN-02** | Dictionary applied at polish or decoding post-process stage | P1 |
| **DICTN-03** | Snippets: trigger phrase → expansion text | P1 |
| **DICTN-04** | Import/export dictionary & snippets (JSON) | P2 |
| **DICTN-05** | Auto-learn from user edit of last paste (opt-in) | P3 |

### 4.7 Context awareness (local only)

| ID | Requirement | Priority |
| --- | --- | --- |
| **CTX-01** | Detect **foreground app name** and use for style heuristics | P2 |
| **CTX-02** | Optional: read nearby text via accessibility for capitalization continuity | P2 |
| **CTX-03** | Context never leaves device | P0 (when feature exists) |
| **CTX-04** | Context features off by default or clearly disclosed | P2 |
| **CTX-05** | Never read password fields (best effort) | P2 |
| **CTX-06** | No screenshot/OCR in MVP | P0 (exclude) |

### 4.8 History & Hub

| ID | Requirement | Priority |
| --- | --- | --- |
| **HIST-01** | Optional local history of recent transcripts | P1 |
| **HIST-02** | User can disable history entirely | P0 |
| **HIST-03** | Delete single entry / clear all | P1 |
| **HIST-04** | Search history | P3 |
| **HIST-05** | Scratchpad window for dictation without target app focus | P2 |

### 4.9 Settings & onboarding

| ID | Requirement | Priority |
| --- | --- | --- |
| **SET-01** | Settings UI: model path, hotkey, polish on/off, mic device | P0 |
| **SET-02** | First-run: model download or path, mic permission, accessibility guide | P0 |
| **SET-03** | System tray / menu bar; hide main window | P1 |
| **SET-04** | Launch at login (optional) | P2 |
| **SET-05** | About / version / open research docs locally | P3 |
| **SET-06** | Diagnostics log view (no secrets; local only) | P1 |

### 4.10 Privacy & security product requirements

| ID | Requirement | Priority |
| --- | --- | --- |
| **PRIV-01** | Default path: **zero network** for audio, transcripts, polish | P0 |
| **PRIV-02** | Any future optional cloud feature is **opt-in**, labeled, off by default | P0 |
| **PRIV-03** | No telemetry of dictation content | P0 |
| **PRIV-04** | Optional anonymous usage telemetry only if explicitly designed later (default off) | P3 |
| **PRIV-05** | Clear in-app statement: “Fully local — audio stays on this device” | P0 |
| **PRIV-06** | Local data paths documented; uninstall/cleanup guidance | P1 |
| **PRIV-07** | No account / login required for core features | P0 |

---

## 5. Platform phases

| Phase | Platform | Scope |
| --- | --- | --- |
| **Phase 0 (spike)** | macOS + Linux (dev) | Hotkey, mic, Whisper, clipboard paste — **done / in progress** |
| **Phase 1 — MVP** | **macOS** primary | Polish, tray, settings, dictionary, reliable inject, packaging dmg |
| **Phase 2** | **Linux** productized | Wayland/X11 hardening, AppImage/Flatpak, GPU paths |
| **Phase 3** | **Windows** | Hotkeys (Ctrl+Win class), inject, installer |
| **Phase 4** | Mobile / other | Out of scope unless strategy changes |

### macOS MVP notes

- Microphone permission  
- Accessibility for paste reliability  
- Metal acceleration path for Whisper  
- Menu bar presence  

### Windows notes (later)

- Global shortcuts without fighting reserved combos  
- Clipboard + SendInput paste  
- Optional CUDA  

### Linux notes

- `cpal` backend (ALSA/PipeWire)  
- Global hotkeys under X11 vs Wayland compositors  
- Paste reliability varies by desktop environment  

---

## 6. Non-functional requirements (NFRs)

| ID | Category | Requirement | Target |
| --- | --- | --- | --- |
| **NFR-01** | Latency | Short utterance (≤ 5 s audio) end-to-end on Apple Silicon + small model | ≤ 2–4 s after release preferred |
| **NFR-02** | Latency | CPU-only baseline remains usable | Documented; no freeze of UI |
| **NFR-03** | Memory | Idle tray footprint | Strive &lt; 150 MB without model loaded; document with model |
| **NFR-04** | Reliability | Dictation session does not crash app on STT errors | Errors → UI message |
| **NFR-05** | Offline | Airplane mode full dictation after models installed | Required |
| **NFR-06** | Privacy | No outbound connections on default path | Verifiable (docs + optional debug) |
| **NFR-07** | Accessibility | Keyboard-operable settings; visible status | Required |
| **NFR-08** | Packaging | Single-user install without Docker/Python runtime | Required |
| **NFR-09** | Models | User can replace models without recompile | Required |
| **NFR-10** | Security | No shelling out unsafely with user text; least privilege | Required |

---

## 7. Architecture

Aligned with stack ADR:

```
┌─────────────────────────────────────────────────────────────┐
│  UI (Tauri 2)                                               │
│  Settings · status · dictionary · history (optional)        │
└────────────────────────────┬────────────────────────────────┘
                             │ IPC
┌────────────────────────────▼────────────────────────────────┐
│  Rust core                                                  │
│  ┌──────────┐  ┌─────────┐  ┌────────┐  ┌────────────────┐  │
│  │ Hotkeys  │→ │ Audio   │→ │ STT    │→ │ Polish         │  │
│  │ global   │  │ cpal    │  │whisper │  │ rules ± LLM    │  │
│  └──────────┘  │ + VAD   │  │ .cpp   │  └────────┬───────┘  │
│                └─────────┘  └────────┘           │          │
│  ┌───────────────────────────────────────────────▼───────┐  │
│  │ Inject: arboard clipboard + enigo/simulated paste     │  │
│  └───────────────────────────────────────────────────────┘  │
│  Storage: SQLite/JSON under OS config dir                   │
└─────────────────────────────────────────────────────────────┘

Hard rule: no audio/transcript leaves the device on the default path.
```

### Components

| Component | Responsibility |
| --- | --- |
| `audio` | Mic stream, buffering, stop/cancel |
| `stt` | whisper-rs load/transcribe |
| `polish` | Fillers, punctuation, dictionary, snippets |
| `cmd` (later) | llama.cpp transform |
| `inject` | Clipboard + paste; fallback notify |
| `state` | Session FSM: idle/listen/process |
| `ui` | Settings, logs, status |

---

## 8. User stories

### MVP

1. **As a** user, **I want** a global hotkey to start/stop listening **so that** I can dictate into any app without switching focus to TalonType.  
2. **As a** user, **I want** my speech transcribed on-device **so that** I can work offline and keep content private.  
3. **As a** user, **I want** filler words removed and basic punctuation **so that** output is usable without heavy editing.  
4. **As a** user, **I want** text pasted into the focused field or left on the clipboard **so that** dictation never “disappears.”  
5. **As a** user, **I want** to pick my Whisper model path **so that** I control quality vs speed.  
6. **As a** user, **I want** to cancel listening **so that** accidental captures are not pasted.  

### Post-MVP

7. **As a** developer, **I want** a personal dictionary of API and product names **so that** STT spelling matches my work.  
8. **As a** user, **I want** snippets for repeated phrases **so that** I speak a cue instead of full text.  
9. **As a** user, **I want** push-to-talk hold **so that** I avoid accidental long recordings.  
10. **As a** user, **I want** Command Mode with a local LLM **so that** I can rewrite selected text without a cloud.  
11. **As a** Linux user, **I want** the same core loop on Wayland **so that** I’m not forced onto Mac/Windows SaaS tools.  

---

## 9. MVP acceptance criteria

MVP is accepted when **all** of the following pass on a developer Mac (and documented Linux attempt):

| # | Criterion |
| --- | --- |
| A1 | Fresh install / clone: download default model → run app without cloud account |
| A2 | Global toggle hotkey starts and stops capture |
| A3 | Spoken English sentence appears as text in TextEdit / Notes / a browser field |
| A4 | With network disabled after model load, A3 still succeeds |
| A5 | Polish removes at least a documented set of fillers from a test phrase |
| A6 | Cancel prevents paste |
| A7 | Paste failure leaves text on clipboard with visible notice |
| A8 | Settings can change model path and hotkey (or hotkey documented default) |
| A9 | No dictation audio/transcript transmitted on default path (architecture review + no required network) |
| A10 | README documents permissions and how to run |

**Spike already covers** partial A2–A3, A8-ish; MVP closes polish, tray, reliability, packaging readiness.

---

## 10. Phased delivery

| Phase | Deliverables | Exit |
| --- | --- | --- |
| **0 — Spike** | Hotkey, mic, Whisper, paste, basic UI | End-to-end on dev machines |
| **1 — MVP** | Polish P0, cancel, clipboard fallback UX, tray, model download UX, privacy copy, macOS focus | Acceptance A1–A10 |
| **2 — Daily driver** | PTT hold, dictionary, snippets, history opt-in, hotkey rebind, Metal/CUDA docs | Internal dogfood ≥ 1 week |
| **3 — Parity features** | Command Mode local LLM, context app-name styles, Scratchpad | Feature flags stable |
| **4 — Platforms** | Linux product packaging, then Windows | Platform smoke tests |
| **5 — Delight** | Auto dictionary learn, advanced backtrack, multi-language packs | Roadmap refresh |

---

## 11. Competitive positioning

| Dimension | Wispr Flow | TalonType |
| --- | --- | --- |
| STT location | **Always cloud** | **Always local (default)** |
| Offline | No | **Yes** |
| E2E encryption | No (provider decrypts) | N/A — no provider |
| Free limits | 2000 words/week desktop | Unlimited |
| Price | $12–15/mo Pro | TBD (OSS / paid optional) |
| Platforms | Mac, Win, iOS, Android | Mac + Linux first; Windows later |
| Polish | Strong cloud AI | Rules → local LLM |
| Command Mode | Cloud Pro feature | Local LLM later |
| Account | Required | **Not required** |
| Linux | — | **First-class goal** |

**Positioning statement:**

> TalonType is local-first dictation with Flow-inspired UX: hold a hotkey, speak, get clean text in any app—without a subscription meter and without uploading your voice.

---

## 12. Open decisions

| ID | Decision | Options | Notes |
| --- | --- | --- | --- |
| OD-01 | License | MIT / Apache / GPL / source-available | Affects model bundling & distribution |
| OD-02 | Monetization | Free OSS, paid builds, donations | Not blocking MVP |
| OD-03 | Default model | tiny/base/small.en | Trade latency vs accuracy |
| OD-04 | History default | Off vs on with retention | Privacy posture: prefer **off** |
| OD-05 | Auto-update | None / Sparkle / tauri updater | Network for updates ≠ dictation path |
| OD-06 | VAD library | energy threshold vs silero | Spike complexity |
| OD-07 | Command Mode model size | 1B–8B class GGUF | Hardware floor |
| OD-08 | Windows priority vs Linux polish | Sequence | ADR currently Mac+Linux |
| OD-09 | Brand name “TalonType” | Keep / rename | Legal check |
| OD-10 | Accessibility API inject (Mac) | Beyond paste | Reliability vs complexity |

---

## 13. Traceability: Wispr capabilities → TalonType requirements

| Wispr capability | Local response | Requirement IDs |
| --- | --- | --- |
| System-wide dictation | Global hotkey + inject | DICT-*, INJ-* |
| Cloud STT | whisper.cpp on-device | STT-* |
| AI Auto Edits / Smart Formatting | Rules polish ± LLM | POL-* |
| Backtrack | Phrase heuristics | POL-05 |
| Command Mode | Local llama.cpp | CMD-* |
| Personal dictionary | Local store | DICTN-01–02 |
| Snippets | Local expansions | DICTN-03 |
| Context Awareness | Local app/text only | CTX-* |
| Hub / history | Optional local only | HIST-* |
| Privacy Mode / Cloud Sync / ZDR | **Superseded** by offline-default | PRIV-* |
| Free word limits | Unlimited local | Product thesis |
| Enterprise SSO/HIPAA | Out of scope v1 | — |
| Mobile apps | Out of scope v1 | — |
| Fn / Ctrl+Win hotkeys | Configurable; platform defaults | DICT-01, DICT-07 |
| Clipboard fallback | Same pattern | INJ-03 |

---

## 14. Requirement ID index (quick)

| Prefix | Area |
| --- | --- |
| DICT- | Dictation control |
| INJ- | Injection |
| STT- | Speech-to-text |
| POL- | Polish |
| CMD- | Command Mode |
| DICTN- | Dictionary / snippets |
| CTX- | Context |
| HIST- | History |
| SET- | Settings |
| PRIV- | Privacy |
| NFR- | Non-functional |
| OD- | Open decisions |

---

## 15. Glossary

| Term | Definition |
| --- | --- |
| **PTT** | Push-to-talk |
| **Polish** | Post-STT cleanup (fillers, punctuation, style) |
| **Inject** | Insert text into focused application |
| **ZDR** | Flow term: no training + no storage; still cloud process |
| **Local-first** | Core features work without network after install |
| **ggml / GGUF** | On-disk model formats for whisper.cpp / llama.cpp |

---

## 16. References

- [wispr-flow.md](./wispr-flow.md) — competitive research  
- [stack-decision.md](./stack-decision.md) — language & model stack ADR  
- [whisper.cpp](https://github.com/ggml-org/whisper.cpp)  
- [llama.cpp](https://github.com/ggml-org/llama.cpp)  
- [Tauri 2](https://v2.tauri.app/)  
- Flow primary docs: https://wisprflow.ai · https://docs.wisprflow.ai · https://wisprflow.ai/data-controls  

---

*Living document. Update priorities when spike learnings change feasibility.*
