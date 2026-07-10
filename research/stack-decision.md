# ADR: Language & Local Model Stack

**Status:** Accepted  
**Date:** 2026-07-10  
**Context:** Mac + Linux local dictation app (EagleScribe), Wispr Flow–class UX, fully offline by default.  
**Related:** [wispr-flow.md](./wispr-flow.md), [requirements-local-app.md](./requirements-local-app.md)

---

## Decision

| Layer | Choice |
| --- | --- |
| **Language** | **Rust** |
| **App shell** | **Tauri 2** (tray + settings UI) |
| **Speech-to-text** | **whisper.cpp** (via `whisper-rs` / linked lib) |
| **LLM polish / Command Mode** | **llama.cpp** (in-process or local `llama-server`; GGUF) |
| **Polish MVP** | **Deterministic rules first**; LLM optional |
| **Audio** | `cpal` |
| **Hotkeys** | Tauri global-shortcut plugin + platform modules |
| **Text inject** | Clipboard + simulated paste; clipboard-only fallback |
| **Storage** | SQLite or JSON under OS config dir |

---

## Why this stack

1. **One codebase for macOS and Linux** — Swift/MLX alone cannot be the core path.
2. **Native bindings to ggml** — whisper.cpp and llama.cpp are the portable, hardware-accelerated standard for local STT/LLM.
3. **Lean runtime** — no mandatory Python or full Chromium app shell for inference.
4. **Offline by construction** — models load from disk; network is not on the critical path.

### Acceleration targets

| Platform | STT / LLM backend |
| --- | --- |
| macOS Apple Silicon | Metal (whisper.cpp / llama.cpp) |
| Linux NVIDIA | CUDA preferred |
| Linux AMD / other GPU | Vulkan |
| Any | CPU fallback (GGUF / ggml) |

### Explicit non-choices (as primary)

| Rejected as core | Reason |
| --- | --- |
| Swift-only | No Linux |
| MLX-only | Apple Silicon only; optional later Mac accelerator |
| Python-only app | Packaging, tray, hotkeys, inject are harder for end users |
| Electron | Wrong default for RAM / always-on tool |
| Ollama as hard dependency | Fine as optional backend; not required for shipping |
| vLLM | Server / multi-GPU; overkill for personal tray app |

---

## Architecture (spike → product)

```
UI (Tauri) ──► Rust core (hotkey, mic, VAD, inject)
                    │
                    ├─► whisper.cpp  ──► raw transcript
                    │
                    └─► polish: rules (± llama.cpp) ──► insert text
```

**Hard rule:** no audio/transcript leaves the device on the default path.

---

## Spike status (implemented)

Prove end-to-end on developer machines:

1. Global toggle hotkey (start/stop listen) — **done** (`Ctrl+Shift+Space`)
2. Capture mic → PCM on dedicated thread — **done** (`cpal`)
3. Transcribe with whisper.cpp (user-provided model path) — **done** (`whisper-rs`)
4. Copy result to clipboard and attempt paste — **done** (`arboard` + `enigo`)
5. Settings UI: model path, status log — **done** (Tauri frontend)

**Run:** `npm install && npm run model:download && npm run desktop`

On Apple Silicon for faster STT: `cd src-tauri && cargo run --features metal` (or pass features through Tauri).

Out of spike: full Smart Formatting, dictionary UI, llama.cpp polish, Wayland edge cases, hold-to-talk.

---

## Consequences

- **Positive:** Shared Mac/Linux binary path; reuse GGUF/ggml model ecosystem; aligns with Superwhisper-class tools.
- **Negative:** Longer compile times (C++ deps); Linux Wayland hotkey/paste needs care; users must download models.
- **Follow-ups:** Wire llama.cpp for Command Mode; rule-based polish; platform inject hardening; packaging (dmg / AppImage).

---

## References

- [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
- [Tauri 2](https://v2.tauri.app/)
