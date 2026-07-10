# EagleScribe

Local-first voice dictation for **macOS** and **Linux**. Speak → on-device Whisper transcription → paste into the focused app. No cloud required.

**Session handoff / status & gaps:** [`research/STATUS.md`](./research/STATUS.md)  
Research and requirements: [`research/`](./research/).

## Stack

| Layer | Choice |
| --- | --- |
| Language | Rust |
| Shell | Tauri 2 |
| STT | whisper.cpp (`whisper-rs`) |
| Offline polish | Rules in `polish.rs` (fillers, punct, backtrack, lists) |
| Command Mode LLM | Local OpenAI-compatible HTTP (Ollama / llama-server) |
| Hotkey | Rebindable in UI (defaults: `Ctrl+Shift+Space` dictation, `Ctrl+Shift+X` Command Mode) |

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
export EAGLESCRIBE_WHISPER_MODEL=/path/to/ggml-small.en.bin
npm run desktop
```

## Using the spike

1. Click **Load** (or the first release will load the model).
2. Choose **Hold to talk** or **Toggle**, and optionally **Change** the global hotkeys (saved locally).
3. Focus a text field in another app.
4. Use the dictation hotkey (default **Ctrl+Shift+Space**) according to that mode (or the UI button, which always toggles).

If paste fails, the text stays on the clipboard — paste manually (`Cmd+V` / `Ctrl+V`).

### System tray

EagleScribe stays in the **menu bar** (macOS) or **system tray** (Linux/Windows):

- **Close** the window → hides to tray; global hotkeys keep working
- **macOS menu bar** shows **ES** — **left-click** restores the window
- **Right-click** the tray item → Show Window, Hide Window, **Quit EagleScribe**
- **Dock** click also restores when the window was hidden

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

Add preferred spellings (names, product terms) in the UI. Matching is case-insensitive with word boundaries; longer phrases win. Applied after polish. Stored only on disk under the OS app data dir (`…/eaglescribe/dictionary.json`).

## Snippets

Map a short **cue** to a longer **expansion** (signatures, links, templates). If the whole utterance is the cue (ignoring trailing `.`/`?`), the expansion replaces it. Cues inside a sentence expand in place. Applied after dictionary. File: `…/eaglescribe/snippets.json`.

## What’s next

See [research/STATUS.md](./research/STATUS.md) for the full status, backlog, and suggested session slices.

- [x] Deterministic polish (fillers, punctuation, backtrack, lists)
- [x] Personal dictionary
- [x] Snippets
- [x] Push-to-talk hold (UI button still toggles)
- [x] Command Mode via local OpenAI-compatible LLM (Ollama / llama-server)
- [x] System tray / hide window (close hides to tray; Quit from tray menu)
- [x] Rebindable hotkeys
- [x] Dense tabbed UI + waiting-LLM status
- [x] Transcript history (last N, local; History tab)
- [ ] Global Escape cancel while recording
- [ ] Mic device picker
- [ ] Linux Wayland hotkey/paste hardening
- [ ] Packaging (dmg / AppImage)

## Command Mode

1. Run a local LLM server (recommended: [Ollama](https://ollama.com) + `ollama pull llama3.2`).
2. In EagleScribe, set base URL `http://127.0.0.1:11434/v1` and model name, click **Save LLM**.
3. Select text in any app.
4. Hold **Ctrl+Shift+X**, speak an instruction (e.g. “make this more professional”), release.
5. Rewritten text is pasted (selection was copied first via Cmd/Ctrl+C).

Works with any OpenAI-compatible local server (`llama-server`, LM Studio, etc.). Traffic stays on localhost.

## License

TBD.
