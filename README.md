# TalonType

Local-first voice dictation for **macOS** and **Linux**. Speak → on-device Whisper transcription → paste into the focused app. No cloud required.

Research and requirements live under [`research/`](./research/).

## Stack (spike)

| Layer | Choice |
| --- | --- |
| Language | Rust |
| Shell | Tauri 2 |
| STT | whisper.cpp (`whisper-rs`) |
| LLM polish | *not in spike* — llama.cpp next |
| Hotkey | `Ctrl+Shift+Space` (toggle) |

See [research/stack-decision.md](./research/stack-decision.md).

## Prerequisites

- **Rust** (stable)
- **Node.js** 20+
- **cmake**, a C/C++ toolchain (for whisper.cpp)
- macOS: Xcode CLT; grant **Microphone** (and later Accessibility for paste reliability)
- Linux: `libasound2-dev` / PipeWire libs as needed for `cpal`

## Quick start

```bash
# 1. JS deps
npm install

# 2. Download a small English Whisper model (~140MB)
npm run model:download

# 3. Run the desktop app
npm run desktop
```

On **Apple Silicon**, rebuild with Metal for faster STT:

```bash
cd src-tauri && cargo build --features metal
# or: npm run desktop -- -- --features metal
```

Optional: point at any ggml model:

```bash
export TALONTYPE_WHISPER_MODEL=/path/to/ggml-small.en.bin
npm run desktop
```

## Using the spike

1. Click **Load** (or the first stop will load the model).
2. Focus a text field in another app.
3. Press **Ctrl+Shift+Space** (or the in-app button) → speak.
4. Press the hotkey again → on-device transcription → clipboard + paste.

If paste fails, the text stays on the clipboard — paste manually (`Cmd+V` / `Ctrl+V`).

## Project layout

```
src/                 # Tauri frontend (settings / status UI)
src-tauri/src/       # Rust core: audio, STT, inject, state
models/              # ggml weights (gitignored)
research/            # product research + requirements + ADR
scripts/             # model download helper
```

## Polish (smart cleanup)

After STT, **smart** mode (default) runs offline rules:

- Filler removal (`um`, `uh`, `you know`, …)
- Spoken punctuation (`question mark` → `?`)
- Backtrack (`scratch that`, `2 actually 3`)
- Capitalization + trailing period

Switch to **verbatim** in the UI for raw Whisper output. Raw + polished text both appear in the window after each dictation.

## Dictionary

Add preferred spellings (names, product terms) in the UI. Matching is case-insensitive with word boundaries; longer phrases win. Applied after polish. Stored only on disk under the OS app data dir (`…/talontype/dictionary.json`).

## What’s next

- [x] Deterministic polish (fillers, punctuation, backtrack)
- [x] Personal dictionary
- [ ] llama.cpp Command Mode / optional rewrite
- [ ] Push-to-talk hold (not only toggle)
- [ ] Linux Wayland hotkey/paste hardening
- [ ] Snippets
- [ ] System tray / hide window

## License

TBD.
