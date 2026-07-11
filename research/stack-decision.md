# ADR: Language & Local Model Stack

**Status:** Accepted  
**Date:** 2026-07-10  
**Context:** Mac + Linux local dictation app (EagleScribe), Wispr Flow–class UX, fully offline by default.  
**Related:** [wispr-flow.md](./wispr-flow.md), [requirements-local-app.md](./requirements-local-app.md), [in-process-llm-stance.md](./in-process-llm-stance.md)

---

## Decision

| Layer | Choice |
| --- | --- |
| **Language** | **Rust** |
| **App shell** | **Tauri 2** (tray + settings UI) |
| **Speech-to-text** | **whisper.cpp** (via `whisper-rs` / linked lib) |
| **LLM polish / Command Mode** | **HTTP localhost** OpenAI-compatible (Ollama / `llama-server` / etc.); **not** in-process llama.cpp — see [in-process-llm-stance.md](./in-process-llm-stance.md) |
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
| macOS Apple Silicon | Metal (whisper.cpp; external LLM server may use Metal separately) |
| Linux NVIDIA | CUDA preferred (whisper feature; external LLM server separate) |
| Linux AMD / other GPU | Vulkan (whisper feature; external LLM server separate) |
| Any | CPU fallback (ggml for STT; LLM via external local server) |

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
                    └─► polish: rules; Command Mode → localhost HTTP LLM ──► insert text
```

**Hard rule:** no audio/transcript leaves the device on the default path.  
**Command Mode LLM:** user-configured **localhost** only; in-process llama.cpp is a **deferred non-goal** ([in-process-llm-stance.md](./in-process-llm-stance.md)).

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

Out of spike: full Smart Formatting, dictionary UI, Wayland edge cases, hold-to-talk.  
**Later productized:** Command Mode via localhost HTTP (not in-process llama.cpp).

---

## Consequences

- **Positive:** Shared Mac/Linux binary path; reuse ggml STT ecosystem; Command Mode reuses any OpenAI-compatible local server without dual C++ link cost.
- **Negative:** Longer compile times (whisper C++ deps); Linux Wayland hotkey/paste needs care; users must download STT models; Command Mode needs a separate local LLM process when used.
- **Follow-ups:** Rule-based polish; platform inject hardening; packaging (dmg / AppImage). In-process llama.cpp remains deferred — [in-process-llm-stance.md](./in-process-llm-stance.md).

---

## References

- [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
- [Tauri 2](https://v2.tauri.app/)
