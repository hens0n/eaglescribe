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
| Hotkey | `Ctrl+Shift+Space` — **hold** or **toggle** (choose in UI) |

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

1. Click **Load** (or the first release will load the model).
2. Choose **Hold to talk** or **Toggle** under Dictation hotkey (saved locally).
3. Focus a text field in another app.
4. Use **Ctrl+Shift+Space** according to that mode (or the UI button, which always toggles).

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
- **Lists** (`one… two…`, `first… second…`, digit markers, `bullet…`)
- Capitalization + trailing period

Switch to **verbatim** in the UI for raw Whisper output. Raw + polished text both appear in the window after each dictation.

## Dictionary

Add preferred spellings (names, product terms) in the UI. Matching is case-insensitive with word boundaries; longer phrases win. Applied after polish. Stored only on disk under the OS app data dir (`…/talontype/dictionary.json`).

## Snippets

Map a short **cue** to a longer **expansion** (signatures, links, templates). If the whole utterance is the cue (ignoring trailing `.`/`?`), the expansion replaces it. Cues inside a sentence expand in place. Applied after dictionary. File: `…/talontype/snippets.json`.

## What’s next

- [x] Deterministic polish (fillers, punctuation, backtrack)
- [x] Personal dictionary
- [x] Snippets
- [x] Push-to-talk hold (UI button still toggles)
- [x] Command Mode via local OpenAI-compatible LLM (Ollama / llama-server)
- [ ] Linux Wayland hotkey/paste hardening
- [ ] System tray / hide window

## Command Mode

1. Run a local LLM server (recommended: [Ollama](https://ollama.com) + `ollama pull llama3.2`).
2. In TalonType, set base URL `http://127.0.0.1:11434/v1` and model name, click **Save LLM**.
3. Select text in any app.
4. Hold **Ctrl+Shift+X**, speak an instruction (e.g. “make this more professional”), release.
5. Rewritten text is pasted (selection was copied first via Cmd/Ctrl+C).

Works with any OpenAI-compatible local server (`llama-server`, LM Studio, etc.). Traffic stays on localhost.

## License

TBD.
