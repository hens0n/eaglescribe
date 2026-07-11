# EagleScribe

Local-first voice dictation for **macOS** and **Linux**. Speak → on-device Whisper transcription → paste into the focused app. No cloud required.

<p align="center">
  <img src="docs/images/gui-main.png" alt="EagleScribe main window — idle dictation UI with Settings, hotkeys, and last transcript" width="420" />
</p>

**Backlog & orientation:** [GitHub issues](https://github.com/hens0n/eaglescribe/issues) · behavior specs and ADRs under [`research/`](./research/) · requirements: [`research/requirements-local-app.md`](./research/requirements-local-app.md) · stack: [`research/stack-decision.md`](./research/stack-decision.md).

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
- Linux: `libasound2-dev` / PipeWire libs as needed for `cpal`; for X11 paste/hotkey stack also `libxdo-dev` / `libxdo` (or distro equivalents — see [Linux hotkeys & paste](#linux-hotkeys--paste-x11-vs-wayland))

## Quick start

```bash
# 1. JS deps
npm install

# 2. Download a small English Whisper model (~140MB)
npm run model:download

# 3. Run the desktop app (CPU STT by default)
npm run desktop
```

Optional: point at any ggml model (models are **not** bundled into the app):

```bash
export EAGLESCRIBE_WHISPER_MODEL=/path/to/ggml-small.en.bin
npm run desktop
```

On **Apple Silicon**, use the Metal dogfood build for faster STT (see [Packaging](#packaging-macos-unsigned-dogfood) below). Day-to-day `npm run desktop` stays CPU so contributors are not surprised.

## Packaging (macOS unsigned dogfood)

macOS is the first-class dogfood path. Builds are **unsigned** — no Developer ID, notarization, or release CI in this repo yet.

**macOS floor:** release builds (`desktop:build` / `desktop:build:metal`) set `bundle.macOS.minimumSystemVersion` to **10.15** so whisper.cpp can use `std::filesystem` (Tauri’s default was 10.13 and fails to compile current whisper-rs). The resulting app requires **macOS 10.15+**.

### CPU build (default)

Stays free of Metal/CUDA toolkits beyond the usual cmake / C++ baseline:

```bash
npm run desktop:build
```

### Metal dogfood build (Apple Silicon)

Preferred for real use on Apple Silicon so Whisper matches the latency goal direction. Requires Xcode CLT (already a prerequisite). **Switching acceleration always requires a rebuild** — there is no Settings toggle for Metal/CUDA/Vulkan.

```bash
npm run desktop:build:metal
```

Equivalent under the hood: `tauri build --features metal` (Cargo feature `metal` → `whisper-rs/metal`).

| When | Which script |
| --- | --- |
| General contrib / CI-friendly / Intel Mac | `npm run desktop:build` (CPU) |
| Apple Silicon dogfood / lower STT latency | `npm run desktop:build:metal` |
| Day-to-day `tauri dev` | `npm run desktop` (CPU default) |

Dev with Metal (optional): `npm run desktop -- --features metal` or `cd src-tauri && cargo run --features metal`.

### Where the artifacts land

After a successful `tauri build` (CPU or Metal), Tauri 2 writes under `src-tauri/target/release/bundle/`:

| Artifact | Typical path |
| --- | --- |
| **`.app`** | `src-tauri/target/release/bundle/macos/EagleScribe.app` |
| **`.dmg`** | `src-tauri/target/release/bundle/dmg/EagleScribe_<version>_<arch>.dmg` (e.g. `EagleScribe_0.1.0_aarch64.dmg`) |

Open the `.app` from Finder or install from the `.dmg`. Product name / identifier come from `src-tauri/tauri.conf.json` (`EagleScribe` / `ai.eaglescribe.app`).

### Gatekeeper (unsigned apps)

macOS may block or quarantine unsigned builds. For a **technical dogfooder**:

1. **Right-click → Open** the `.app` (or the app inside the `.dmg`), then confirm **Open** in the dialog. First launch often needs this instead of a normal double-click.
2. If macOS still refuses: clear the quarantine flag, then open again:
   ```bash
   xattr -dr com.apple.quarantine /path/to/EagleScribe.app
   open /path/to/EagleScribe.app
   ```
3. **System Settings → Privacy & Security** may show a blocked-app notice with an **Open Anyway** control after a failed launch.

This is expected until we add signing/notarization (out of scope for now). Do not expect App Store distribution from these scripts.

### Models are not in the bundle

The `.app` / `.dmg` does **not** need to embed `ggml-*.bin` weights. Load models the same way as in dev:

- **Load** in the UI after placing a model under `models/` (repo checkout) or any path you choose
- `npm run model:download` (small English model into `models/`)
- `EAGLESCRIBE_WHISPER_MODEL=/path/to/ggml-….bin`

You can copy a model next to the installed app or keep it in a shared folder; path / env / download story is unchanged.

### Smoke check (Metal dogfood)

After `npm run desktop:build:metal`, open `EagleScribe.app`, load a model, and run one dictation. STT should work as on CPU; Metal only changes the compiled Whisper backend (faster on Apple Silicon when the feature is linked).

## Linux packaging (contributor notes)

Linux packaging is **docs-only** this pass — not equal to macOS dogfood acceptance. On a Linux host with the [Prerequisites](#prerequisites) (cmake, ALSA/PipeWire, etc.):

```bash
npm run desktop:build
```

Produces whatever Tauri 2 targets your machine supports (e.g. `.deb` / AppImage under `src-tauri/target/release/bundle/` when enabled). AppImage / distro packaging may improve later; there is no CI gate that an AppImage must ship.

### Linux hotkeys & paste (X11 vs Wayland)

Behavior contract: [`research/linux-hotkey-paste-spec.md`](./research/linux-hotkey-paste-spec.md).

Global hotkeys use `tauri-plugin-global-shortcut` → **`global-hotkey` (X11 grabs on Linux)**. Simulated paste uses **enigo** (X11 / `libxdo`). **Pure Wayland global hotkeys are not guaranteed** with the current stack — do **not** expect (or advertise) full Wayland global-hotkey parity until re-verified after an upstream change (e.g. portal support in `global-hotkey`).

#### Check your session

```bash
echo $XDG_SESSION_TYPE
# x11      → committed hard path (hotkeys + paste when deps present)
# wayland  → best-effort; see recommendation below
```

#### Recommendation (daily-driver reliability)

| Prefer | When |
| --- | --- |
| **X11 session** (native Xorg / “GNOME on Xorg” / XFCE X11, etc.) | You want global hotkeys and simulated paste to work reliably today |
| **Pure Wayland** | Accept **UI Start/Stop/Cancel** as the primary control surface and **clipboard-only** inject when paste simulation fails (manual `Ctrl+V`) |

| Session | Product commitment |
| --- | --- |
| **X11** | **Hard acceptance:** global hotkeys + simulated paste when packages below are installed |
| **Wayland** (incl. pure) | **Best-effort only.** If hotkey registration fails the app **does not crash** and shows **hotkeys unavailable — use window controls**. Capture → STT → clipboard still works from the window; paste is best-effort |

#### Documented X11 path (Ubuntu / Debian)

Hard gate for contributors and QA: **Ubuntu or Debian on an X11 session** with the packages below. This is the path we commit to for end-to-end dictation on Linux.

**Build** (compile Tauri / enigo X11 backend):

```bash
sudo apt update
sudo apt install -y build-essential cmake pkg-config \
  libasound2-dev libxdo-dev libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

**Runtime** (inject / paste simulation; often pulled in by `libxdo-dev`, listed explicitly for dogfood machines):

```bash
sudo apt install -y libxdo3
# optional helper tools / peers used by some X11 setups:
# sudo apt install -y xdotool
```

Then verify session and run:

```bash
echo $XDG_SESSION_TYPE   # expect: x11
npm install && npm run model:download && npm run desktop
```

Other families (names may vary by release):

| Family | Packages (indicative) |
| --- | --- |
| Debian/Ubuntu | `libxdo-dev` (build), `libxdo3` (runtime); ALSA/PipeWire for mic (`libasound2-dev`) |
| Fedora | `libX11-devel` `libxdo-devel` (+ cpal/audio deps as needed) |
| Arch | `xdotool` / related (per enigo); base-devel for build |

#### Smoke matrix (implementers / PR notes)

Manual QA matrix lives in the [spec §5](./research/linux-hotkey-paste-spec.md#5-suggested-smoke-matrix-for-implementers--manual-qa). Record results in the PR:

| Gate | Session | Expectation |
| --- | --- | --- |
| **Hard** | **X11** (row 1: GNOME on Xorg / XFCE / etc.) | Hotkey + paste into a native text field (and preferably a browser) all green |
| **Best-effort** | **Wayland** (GNOME / KDE / optional Sway) | Document limits; UI + clipboard path must still work; do not block merge on full hotkey/paste parity |

Minimum for the Linux reliability slice: **X11 hard row green** + honest docs for Wayland rows.

## CUDA / Vulkan (contributor GPU builds)

Cargo features in `src-tauri/Cargo.toml` enable GPU STT backends at **compile time**. They are **not** Settings switches and are not first-class packaging scripts like Metal on macOS.

| Feature | Typical use | Build-time requirement |
| --- | --- | --- |
| `metal` | macOS Apple Silicon | Xcode CLT; use `npm run desktop:build:metal` |
| `cuda` | NVIDIA on Linux | CUDA toolkit installed when building |
| `vulkan` | Portable GPU where whisper-rs supports it | Vulkan SDK / drivers as needed by whisper.cpp |

Examples (pass features through Tauri):

```bash
# NVIDIA (Linux) — toolkit must be present at build time
npm run tauri -- build --features cuda

# Vulkan
npm run tauri -- build --features vulkan
```

Or from the crate: `cd src-tauri && cargo build --release --features cuda` (then bundle separately if needed). Default `desktop:build` stays CPU so contributors without GPU toolkits are not blocked.

## Using the spike

1. Click **Load** (or the first release will load the model).
2. Choose **Hold to talk** or **Toggle**, and optionally **Change** the global hotkeys (saved locally).
3. Focus a text field in another app.
4. Use the dictation hotkey (default **Ctrl+Shift+Space**) according to that mode (or the **Start dictation** window button, which always toggles and works when global hotkeys are unavailable).

If paste fails, the text stays on the clipboard — paste manually (`Cmd+V` / `Ctrl+V`). On a **successful** paste, EagleScribe restores your previous clipboard text by default (toggle under **Settings → Clipboard**).

### System tray

EagleScribe stays in the **menu bar** (macOS) or **system tray** (Linux/Windows):

- **Close** the window → hides to tray; global hotkeys keep working when registered (if registration failed, use **Show Window** and in-window controls)
- **Tray glyph** is a dedicated monochrome mark (same asset on macOS / Linux / Windows). On **macOS** it is a system-tinted **template** image; **left-click** restores the window. Linux/Windows show the monochrome asset without macOS template tinting.
- **Right-click** the tray item → **Show Window**, **Hide Window**, **Quit EagleScribe**
- **Dock** click also restores when the window was hidden (macOS; when Dock is present)
- **Menu bar only** (macOS, opt-in under **Settings → Menu bar**, **default off**): hides the Dock icon after the **next launch** (`ActivationPolicy::Accessory`). Tray remains; the main window still shows on launch. After hide/close, restore with **left-click** or **Show Window** only (no Dock). Turning the toggle off and relaunching restores the Dock. Linux/Windows: control is hidden (no dock-hide feature).

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

Shipped features stay checked below. **Open product work** is tracked as GitHub issues and locked in `research/*-spec.md` (run `/to-tickets` when a Ready-for-`/to-tickets` spec still lacks implementation issues).

| Area | Spec / stance | Implementation issues (when present) |
| --- | --- | --- |
| Mic device picker | [mic-device-picker-spec.md](./research/mic-device-picker-spec.md) | [#4](https://github.com/hens0n/eaglescribe/issues/4), [#5](https://github.com/hens0n/eaglescribe/issues/5) |
| Tray polish | [tray-polish-spec.md](./research/tray-polish-spec.md) | [#14](https://github.com/hens0n/eaglescribe/issues/14)–[#16](https://github.com/hens0n/eaglescribe/issues/16) |
| VAD / silence trim | [vad-silence-trim-spec.md](./research/vad-silence-trim-spec.md) | (ready for `/to-tickets`) |
| Packaging + acceleration UX | [packaging-spec.md](./research/packaging-spec.md) | (ready for `/to-tickets`) |
| Linux hotkey / paste | [linux-hotkey-paste-spec.md](./research/linux-hotkey-paste-spec.md) | [#20](https://github.com/hens0n/eaglescribe/issues/20)–[#22](https://github.com/hens0n/eaglescribe/issues/22) |
| Onboarding / permissions | [onboarding-permissions-spec.md](./research/onboarding-permissions-spec.md) | (ready for `/to-tickets`) |
| In-process llama.cpp | [in-process-llm-stance.md](./research/in-process-llm-stance.md) | Deferred (HTTP Command Mode only) |

- [x] Deterministic polish (fillers, punctuation, backtrack, lists)
- [x] Personal dictionary
- [x] Snippets
- [x] Push-to-talk hold (UI button still toggles)
- [x] Command Mode via local OpenAI-compatible LLM (Ollama / llama-server)
- [x] System tray / hide window (close hides to tray; Quit from tray menu)
- [x] Rebindable hotkeys
- [x] Dense tabbed UI + waiting-LLM status
- [x] Transcript history (last N, local; History tab)
- [x] Global Escape cancel while recording
- [x] Clipboard restore after paste (configurable)
- [ ] Mic device picker ([#4](https://github.com/hens0n/eaglescribe/issues/4), [#5](https://github.com/hens0n/eaglescribe/issues/5))
- [ ] Tray polish ([#14](https://github.com/hens0n/eaglescribe/issues/14)–[#16](https://github.com/hens0n/eaglescribe/issues/16))
- [ ] VAD / silence trim (spec ready)
- [ ] Packaging / acceleration UX (spec ready)
- [ ] Linux hotkey & paste reliability ([#20](https://github.com/hens0n/eaglescribe/issues/20)–[#22](https://github.com/hens0n/eaglescribe/issues/22); inject/hotkey UX shipped, docs in progress)
- [ ] Onboarding / permissions copy (spec ready)

## Command Mode

1. Run a local LLM server (recommended: [Ollama](https://ollama.com) + `ollama pull llama3.2`).
2. In EagleScribe, set base URL `http://127.0.0.1:11434/v1` and model name, click **Save LLM**.
3. Select text in any app.
4. Hold **Ctrl+Shift+X**, speak an instruction (e.g. “make this more professional”), release.
5. Rewritten text is pasted (selection was copied first via Cmd/Ctrl+C).

Works with any OpenAI-compatible local server (`llama-server`, LM Studio, etc.). Traffic stays on localhost.

## License

TBD.
