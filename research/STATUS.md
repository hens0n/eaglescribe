# EagleScribe — project status & session handoff

**Last updated:** 2026-07-10  
**Branch:** `main`  
**Latest commit:** `20f7214` — global Escape cancel while recording  
**Previous ship:** `48ccad9` — local transcript history + History tab  

Use this document to resume work in a **new session**. For product research and full requirements, see:

| Doc | Purpose |
| --- | --- |
| [wispr-flow.md](./wispr-flow.md) | Competitor research (Wispr Flow) |
| [requirements-local-app.md](./requirements-local-app.md) | Local-app requirements (P0–P2) |
| [stack-decision.md](./stack-decision.md) | ADR: Rust / Tauri / whisper.cpp / local LLM |
| [escape-cancel-spec.md](./escape-cancel-spec.md) | Behavior spec: global Escape cancel while recording (**implemented**) |
| [../README.md](../README.md) | How to run & user-facing feature notes |

---

## 1. What we are building

**EagleScribe** is a **fully local** voice-dictation desktop app inspired by Wispr Flow:

- Global hotkey → mic → **on-device Whisper** → **offline polish** → optional dictionary/snippets → paste into the focused app  
- **Command Mode**: select text → speak an instruction → rewrite via a **localhost** OpenAI-compatible LLM (Ollama / llama-server)  
- **No cloud required** for dictation STT/polish; no account  

Primary platforms: **macOS** (validated / daily-driver), **Linux** (intended; less hardened).

---

## 2. Stack (current)

| Layer | Implementation |
| --- | --- |
| App shell | **Tauri 2** + Vite/TS UI (`src/`, `src-tauri/`) |
| Language | **Rust** |
| STT | **whisper.cpp** via `whisper-rs` (`Arc` engine; STT does **not** hold app state mutex) |
| Audio | `cpal` (dedicated thread; stream not held in shared state) |
| Inject | Clipboard + simulated paste/copy on **main thread** (`enigo`, physical keycodes on macOS) |
| Polish | Rule-based in `polish.rs` (no LLM) |
| Command LLM | HTTP to **localhost** only (`ureq` → `/v1/chat/completions`) |
| Persistence | JSON under OS app data dir (`…/eaglescribe/`) |
| Tray | Tauri `tray-icon`; hide-on-close; menu Show / Hide / Quit |

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
  → status: recording
  → Whisper STT (ggml model on disk)  → status: transcribing
  → smart polish OR verbatim
  → personal dictionary
  → snippets
  → inject (clipboard + Cmd/Ctrl+V on main thread)
  → status: idle
```

### Command Mode

```
Select text in any app
  → hold command hotkey (default Ctrl+Shift+X) or UI button
  → capture selection (synthesized Cmd/Ctrl+C on main thread)
  → record spoken instruction  → status: recording
  → Whisper STT + light polish  → status: transcribing
  → local LLM rewrite (localhost)  → status: waiting_llm
  → inject result  → status: idle
```

**Hotkey note:** Command Mode default is **Ctrl+Shift+X**, not `C`. Selection capture synthesizes copy (`C`); using `C` as the main key ends the session immediately. Rebinds reject `C` for Command Mode. Releases are suppressed during capture + ~400 ms debounce.

**Status badge values:** `idle` · `recording` · `transcribing` · `waiting_llm` · `error` (UI labels; snake_case over the wire).

---

## 4. Features — done

| Feature | Notes / keys |
| --- | --- |
| Global dictation hotkey | Default `Ctrl+Shift+Space` (rebindable) |
| Hold vs toggle (user choice) | Saved in `settings.json` |
| UI always-toggle button | Independent of hotkey mode |
| Cancel recording | UI Cancel + **global Escape** while `recording` only (dictation + Command Mode); hold-safe release suppress; Escape-alone rebinds rejected |
| Local Whisper model path | UI + `EAGLESCRIBE_WHISPER_MODEL` + `models/ggml-base.en.bin` |
| Smart polish | Fillers, spoken punct, backtrack, **lists**, cap + period |
| Verbatim mode | Raw-ish STT (whitespace only) |
| Raw + polished shown in UI | After each dictation |
| Personal dictionary | `dictionary.json` |
| Snippets | `snippets.json`; whole-utterance or in-place |
| Command Mode + LLM settings | `settings.json` (`llm_base_url`, `llm_model`) |
| List formatting | Cardinal / ordinal / digit / bullet markers (need ≥2 items) |
| System tray | Menu bar **ES** + icon; Show / Hide / Quit |
| Hide on close | Window close hides to tray; hotkeys stay active until Quit |
| Rebindable hotkeys | Dictation + Command chords; capture UI; conflict + KeyC checks |
| Dense tabbed UI | Always-on status/actions/transcript; tabs: **Settings · Library · History · Log** |
| Waiting-LLM status | Distinct badge while Command Mode awaits localhost LLM |
| STT / paste deadlock fix | No state-mutex hold during Whisper; busy claim before worker |
| Transcript history | Last N (default 50) in `history.json`; History tab; clear; toggle off |

### Local data files (macOS example)

Under `~/Library/Application Support/eaglescribe/` (via `dirs::data_local_dir`):

- `settings.json` — hotkey mode, bindings, LLM, `history_enabled` / `history_max`  
- `dictionary.json`  
- `snippets.json`  
- `history.json` — transcript history (newest capped by `history_max`)  

Whisper weights: repo `models/*.bin` (gitignored) or user path.

**Note:** Pre-rename data lived under `…/talontype/` — not auto-migrated.

---

## 5. Gaps & recommended next work

### Open backlog (priority order)

| Priority | Gap | Why / acceptance sketch |
| --- | --- | --- |
| **High (Linux)** | Wayland global hotkeys + paste reliability | X11-oriented crates; document distro deps; test X11 vs Wayland; fallbacks (clipboard-only if paste fails) |
| **Medium** | Mic **device picker** | List inputs via `cpal`; persist choice in settings; default device fallback |
| **Medium** | **VAD / silence trim** | Drop leading/trailing silence before STT; optional min-speech gate |
| **Medium** | **Tray polish** | Dedicated monochrome template glyph; optional dock-hide (`ActivationPolicy::Accessory`) |
| **Medium** | **Clipboard restore** after paste | Save prior clipboard; restore after successful inject (with short delay) |
| **Medium** | Metal/CUDA **packaging UX** | First-class “build with Metal” docs/scripts; surface acceleration status in UI |
| **Medium** | **Packaging / distribution** | `tauri build` notes; unsigned dmg/AppImage first; later signed + launch at login |
| Low | In-process llama.cpp | Optional; Command Mode stays HTTP-local by default |
| Low | Onboarding / permissions copy | Mic + Accessibility/Input Monitoring checklist on first run |
| Out of scope (v1) | Cloud sync, accounts, team features, full voice OS (Talon-class) | By design |

### Known footguns

1. **Stale Rust linker errors** (`ld: symbol(s) not found for architecture arm64` with `_anon.*`) after many Tauri rebuilds → `cd src-tauri && cargo clean && cargo build`.  
2. **Command Mode needs a running local LLM** (e.g. Ollama). Clear error if unreachable; badge → `error`.  
3. **Spoken “question mark”** only becomes `?` if Whisper emits those words; check **Raw** panel.  
4. **Accessibility / Input Monitoring** may be required for reliable paste/copy simulation on macOS.  
5. **List formatting** needs ≥2 markers; single “one” stays prose; backtrack runs before lists so “two actually three” does not listify.  
6. **Back-to-back dictation** used to deadlock (state mutex held during Whisper + main-thread paste). Fixed in `7cebf0a` — if UI freezes on `transcribing` again, treat as regression.  
7. **Tray Show menu** on macOS may be flaky; left-click + Dock reopen are primary restore paths.

---

## 6. Key source files

```
src-tauri/src/
  lib.rs          # Tauri commands, hotkey register/rebind, tray, app setup
  state.rs        # Session state, dictation + command pipelines, status emit
  audio.rs        # Mic capture on dedicated thread
  stt.rs          # whisper-rs wrapper
  polish.rs       # Offline cleanup + lists
  dictionary.rs   # Phrase replacements
  snippets.rs     # Cue → expansion
  inject.rs       # Clipboard, paste/copy on main thread
  llm.rs          # Local OpenAI-compatible HTTP client
  settings.rs     # Hotkey mode + bindings + LLM prefs
  hotkey.rs       # Parse/validate rebindable global shortcuts
  history.rs      # Local transcript history (history.json)
  error.rs

src/
  main.ts         # UI wiring, tabs, hotkey capture, history list
  styles.css      # Dense shell + tab panels
index.html

scripts/download-whisper-model.sh
models/           # ggml weights (not in git)
```

---

## 7. How to run (new machine / session)

```bash
cd /path/to/eaglescribe
npm install
npm run model:download          # ~140MB ggml-base.en.bin
npm run desktop                 # tauri dev
```

Optional:

```bash
export EAGLESCRIBE_WHISPER_MODEL=/path/to/ggml-small.en.bin
cd src-tauri && cargo build --features metal   # Apple Silicon STT
```

Command Mode:

```bash
ollama pull llama3.2
# In app Settings: URL http://127.0.0.1:11434/v1 , model llama3.2 → Save LLM
```

Tests:

```bash
cd src-tauri && cargo test
```

---

## 8. Suggested next session goals (slices)

Pick **one vertical slice** per session. Each should ship usable end-to-end.

| # | Slice | Deliverable | Effort (rough) |
| --- | --- | --- | --- |
| **1** | **Mic device picker** | Enumerate devices; save id; use on next recording | M |
| **2** | **VAD / silence trim** | Trim audio before Whisper; log trimmed duration | M |
| **3** | **Tray polish** | Template menu-bar icon; optional “menu bar only” (no dock) | S–M |
| **4** | **Clipboard restore** | Restore previous clipboard after inject (configurable) | S |
| **5** | **Linux pass** | Distro deps doc; X11/Wayland hotkey + paste matrix; fallbacks | M–L |
| **6** | **Packaging** | `tauri build` + dmg/AppImage notes; optional Metal release script | M |
| **7** | **Accel packaging UX** | Surface Metal/CUDA in UI; first-class build docs | S–M |

**Done this session:** Escape cancel ([escape-cancel-spec.md](./escape-cancel-spec.md); issues #1–#3).  

**Default recommendation (Mac dogfooding):** **(1) Mic device picker**.  

**If multi-platform soon:** **(5) Linux pass**.

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
| `dd610d3` | Project STATUS handoff doc |
| `37b7b86` | System tray, rebindable hotkeys, rename → EagleScribe |
| `7cebf0a` | Dense tabbed UI, `waiting_llm` status, STT deadlock fix |
| `48ccad9` | Local transcript history + History tab |
| `20f7214` | Global Escape cancel while recording + hold-safe + reject Esc-alone binds |

---

## 10. Privacy stance (product invariant)

- Default path: **no network** for audio / STT / polish / dictionary / snippets.  
- Command Mode: **localhost HTTP only** (user-configured endpoint).  
- No account. No dictation telemetry of content.

Do not add cloud STT or training uploads without an explicit, opt-in product decision.

---

*When updating this file after a session: bump “Last updated”, latest commit, Done table, Gaps table, and Suggested slices.*
