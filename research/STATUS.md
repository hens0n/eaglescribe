# TalonType — project status & session handoff

**Last updated:** 2026-07-10  
**Branch:** `main` (synced with `origin/main` as of latest push)  
**Latest commit (as of write-up):** `99fd3d1` — list formatting in smart polish  

Use this document to resume work in a **new session**. For product research and full requirements, see:

| Doc | Purpose |
| --- | --- |
| [wispr-flow.md](./wispr-flow.md) | Competitor research (Wispr Flow) |
| [requirements-local-app.md](./requirements-local-app.md) | Local-app requirements (P0–P2) |
| [stack-decision.md](./stack-decision.md) | ADR: Rust / Tauri / whisper.cpp / local LLM |
| [../README.md](../README.md) | How to run & user-facing feature notes |

---

## 1. What we are building

**TalonType** is a **fully local** voice-dictation desktop app inspired by Wispr Flow:

- Global hotkey → mic → **on-device Whisper** → **offline polish** → optional dictionary/snippets → paste into the focused app  
- **Command Mode**: select text → speak an instruction → rewrite via a **localhost** OpenAI-compatible LLM (Ollama / llama-server)  
- **No cloud required** for dictation STT/polish; no account  

Primary platforms: **macOS** (validated), **Linux** (intended; less hardened).

---

## 2. Stack (current)

| Layer | Implementation |
| --- | --- |
| App shell | **Tauri 2** + Vite/TS UI (`src/`, `src-tauri/`) |
| Language | **Rust** |
| STT | **whisper.cpp** via `whisper-rs` |
| Audio | `cpal` (dedicated thread; stream not held in shared state) |
| Inject | Clipboard + simulated paste/copy on **main thread** (`enigo`, physical keycodes on macOS) |
| Polish | Rule-based in `polish.rs` (no LLM) |
| Command LLM | HTTP to **localhost** only (`ureq` → `/v1/chat/completions`) |
| Persistence | JSON under OS app data dir (`…/talontype/`) |

**Not used in-process:** llama.cpp linked into the binary (Command Mode uses local HTTP instead, to avoid heavy dual C++ link cost).

### Acceleration features (Cargo)

```toml
# src-tauri/Cargo.toml
metal   # whisper-rs/metal  (Apple Silicon)
cuda    # Linux NVIDIA
vulkan  # Linux portable GPU
```

Default builds are CPU whisper unless `--features metal` (etc.) is passed.

---

## 3. Pipeline

### Dictation

```
Hotkey / UI button
  → mic capture (cpal)
  → Whisper STT (ggml model on disk)
  → smart polish OR verbatim
  → personal dictionary
  → snippets
  → inject (clipboard + Cmd/Ctrl+V on main thread)
```

### Command Mode

```
Select text in any app
  → hold Ctrl+Shift+X (or UI Command Mode button)
  → capture selection (synthesized Cmd/Ctrl+C on main thread)
  → record spoken instruction
  → Whisper STT + light polish on instruction
  → local LLM rewrite (localhost)
  → inject result
```

**Hotkey note:** Command Mode is **Ctrl+Shift+X**, not `C`. Selection capture synthesizes copy (`C`); using `C` as the hotkey caused immediate session end (spurious Released). Releases are also suppressed during capture + ~400 ms debounce.

---

## 4. Features — done

| Feature | Notes / keys |
| --- | --- |
| Global dictation hotkey | `Ctrl+Shift+Space` |
| Hold vs toggle (user choice) | Saved in `settings.json` |
| UI always-toggle button | Independent of hotkey mode |
| Cancel recording | UI Cancel |
| Local Whisper model path | UI + `TALONTYPE_WHISPER_MODEL` + `models/ggml-base.en.bin` |
| Smart polish | Fillers, spoken punct, backtrack, **lists**, cap + period |
| Verbatim mode | Raw-ish STT (whitespace only) |
| Raw + polished shown in UI | After each dictation |
| Personal dictionary | `dictionary.json` |
| Snippets | `snippets.json`; whole-utterance or in-place |
| Command Mode + LLM settings | `settings.json` (`llm_base_url`, `llm_model`) |
| List formatting | Cardinal / ordinal / digit / bullet markers (need ≥2 items) |

### Local data files (macOS example)

Under `~/Library/Application Support/talontype/` (via `dirs::data_local_dir`):

- `settings.json` — hotkey mode, LLM URL/model  
- `dictionary.json`  
- `snippets.json`  

Whisper weights: repo `models/*.bin` (gitignored) or user path.

---

## 5. Gaps & recommended next work

### Explicitly open (README)

| Priority | Gap | Why |
| --- | --- | --- |
| **High (macOS UX)** | System tray / menu bar; hide main window | Always-on dictation without a large window |
| **High (Linux)** | Wayland global hotkeys + paste reliability | X11-oriented crates; needs explicit Wayland path |
| Medium | Rebindable hotkeys | Hardcoded combos today |
| Medium | Escape cancel (global) | Cancel exists in UI only |
| Medium | Mic device picker | Uses default input only |
| Medium | Optional transcript history | Not stored beyond “last” in UI |
| Medium | VAD / silence trim | Full hold duration always transcribed |
| Medium | Metal/CUDA default packaging | Features exist; not first-class UX |
| Medium | Clipboard restore after paste | Optional polish |
| Low | In-process llama.cpp | Command Mode is HTTP-local today |
| Low | Signed dmg / AppImage, launch at login | Packaging / distribution |
| Out of scope (v1) | Cloud sync, accounts, team features, full voice OS (Talon-class) | By design |

### Known footguns

1. **Stale Rust linker errors** (`ld: symbol(s) not found for architecture arm64` with `_anon.*`) after many Tauri rebuilds → `cd src-tauri && cargo clean && cargo build`.  
2. **Command Mode needs a running local LLM** (e.g. Ollama). Clear error if unreachable.  
3. **Spoken “question mark”** only becomes `?` if Whisper emits those words; check **Raw** panel.  
4. **Accessibility / Input Monitoring** may be required for reliable paste/copy simulation on macOS.  
5. **List formatting** needs ≥2 markers; single “one” stays prose; backtrack runs before lists so “two actually three” does not listify.

---

## 6. Key source files

```
src-tauri/src/
  lib.rs          # Tauri commands, hotkeys, app setup
  state.rs        # Session state, dictation + command pipelines
  audio.rs        # Mic capture on dedicated thread
  stt.rs          # whisper-rs wrapper
  polish.rs       # Offline cleanup + lists
  dictionary.rs   # Phrase replacements
  snippets.rs     # Cue → expansion
  inject.rs       # Clipboard, paste/copy on main thread
  llm.rs          # Local OpenAI-compatible HTTP client
  settings.rs     # Hotkey mode + LLM prefs
  error.rs

src/
  main.ts         # UI wiring
  styles.css
index.html

scripts/download-whisper-model.sh
models/           # ggml weights (not in git)
```

---

## 7. How to run (new machine / session)

```bash
cd /path/to/talontype
npm install
npm run model:download          # ~140MB ggml-base.en.bin
npm run desktop                 # tauri dev
```

Optional:

```bash
export TALONTYPE_WHISPER_MODEL=/path/to/ggml-small.en.bin
cd src-tauri && cargo build --features metal   # Apple Silicon STT
```

Command Mode:

```bash
ollama pull llama3.2
# In app: URL http://127.0.0.1:11434/v1 , model llama3.2 → Save LLM
```

Tests:

```bash
cd src-tauri && cargo test
```

---

## 8. Suggested next session goals

Pick one vertical slice:

1. **System tray** (macOS first): tray icon, show/hide window, quit; keep hotkeys while “hidden”.  
2. **Linux pass**: document distro deps; test hotkey + paste on X11 vs Wayland; fallbacks.  
3. **Hotkey rebinding** + conflict warnings.  
4. **History** of last N transcripts (local, optional, clearable).  
5. **Packaging**: `tauri build`, dmg/appimage notes.

Default recommendation: **(1) system tray** for daily-driver feel on the Mac where the app is already proven.

---

## 9. Commit history (feature arc)

| Commit (short) | Theme |
| --- | --- |
| `91c859a` | Initial spike: Tauri + Whisper + inject |
| `002212f` | Smart polish |
| `bc06ead` | Dictionary |
| `1b32677` | Snippets |
| `0052a79` / `1d65f9d` | Hold-to-talk + GUI hold/toggle |
| `cbf0849` / `dc33936` | Command Mode + hotkey fix |
| `99fd3d1` | List formatting |

---

## 10. Privacy stance (product invariant)

- Default path: **no network** for audio / STT / polish / dictionary / snippets.  
- Command Mode: **localhost HTTP only** (user-configured endpoint).  
- No account. No dictation telemetry of content.

Do not add cloud STT or training uploads without an explicit, opt-in product decision.

---

*When updating this file after a session: bump “Last updated”, latest commit, and the Done / Gaps tables.*
