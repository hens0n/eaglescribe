# Packaging & acceleration UX — product specification

**Status:** Ready for `/to-tickets`  
**Date:** 2026-07-10  
**Priority:** Medium (STATUS packaging + Metal/CUDA packaging UX; STT-06, NFR-08)  
**Related:** [requirements-local-app.md](./requirements-local-app.md) (`STT-06`, `NFR-08`, Phase 1 packaging dmg) · [stack-decision.md](./stack-decision.md)

This document is the implementable product/docs contract for **distribution packaging** and **STT acceleration build + UI status** as **one** combined slice. It is not an implementation plan beyond acceptance criteria and intentional non-goals.

---

## 1. Goal

Contributors and dogfooders can:

1. **Build a macOS installable artifact** (`tauri build` → `.app` / `.dmg`) with a **first-class Metal path** for Apple Silicon STT, without inventing a public release pipeline or Apple signing.
2. **See which STT acceleration backend** the running binary was compiled with (Metal / CUDA / Vulkan / CPU), in Settings next to Whisper model setup.
3. **Follow README** for unsigned Gatekeeper install notes, default CPU builds, Metal dogfood builds, and Linux + CUDA/Vulkan contributor notes.

Today: Cargo features `metal` / `cuda` / `vulkan` exist; default is CPU; README has a partial Metal note; `desktop:build` is plain `tauri build`; the UI does not report acceleration; bundle config is minimal with no signing.

---

## 2. Decisions (locked)

| Topic | Decision |
| --- | --- |
| v1 productized packaging | **macOS** unsigned `.app` / `.dmg` via `tauri build` is the dogfood path |
| Linux packaging | **Documented only** this pass — contributor `tauri build` / AppImage notes; not equal acceptance to macOS |
| Default `desktop:build` | Stays **CPU** (no Metal/CUDA/Vulkan features) |
| Metal release / dogfood path | **First-class** npm (or equivalent) script + README: build with **`--features metal`** |
| Models in package | **Not required** in dmg/app — keep `npm run model:download` / path / env (NFR-09) |
| Code signing / notarization | **Out of scope** — document Gatekeeper workarounds for unsigned builds |
| Launch at login | **Out of scope** (future non-goal) |
| Accel in UI | **Read-only** compile-time feature report; **no** runtime enable/disable |
| UI home | **Settings → Whisper model** section (required); header chip optional, not required |
| CPU-only on Apple Silicon | Soft one-line **hint** to rebuild with Metal; never block dictation |
| Linux GPU | Document `cuda` / `vulkan` feature builds at contributor level; UI labels same contract; **no** CUDA hardware CI gate |
| CI release pipeline | **Out of scope** — no automated dmg on every push |
| Windows | Not first-class in this spec (map default) |

---

## 3. Behavior

### 3.1 macOS packaging (unsigned dogfood)

- Document and support building with Tauri 2:
  - **CPU path:** existing `npm run desktop:build` (or documented equivalent) without GPU features.
  - **Metal dogfood path:** a dedicated script (name flexible; e.g. `desktop:build:metal`) that runs `tauri build` with **`--features metal`** (and any flags needed so whisper-rs Metal is linked).
- Artifact expectations (acceptance-level, not exact path orthodoxy):
  - A runnable **EagleScribe `.app`** and/or **`.dmg`** is produced under the usual Tauri `target/release/bundle/…` layout (or whatever the current Tauri 2 default is — docs must state the real path after a smoke build).
- **Unsigned first:**
  - No requirement for Developer ID, notarization, or staple.
  - README must explain that macOS may block unsigned apps (Gatekeeper): e.g. right-click → Open, or clearing quarantine — enough that a technical dogfooder can launch without a support ticket.
- **Models:** package need **not** embed `ggml-*.bin`. First-run / model load continues via path UI, `EAGLESCRIBE_WHISPER_MODEL`, and/or repo `models/` + `npm run model:download` as today.
- Bundle identity already set (`productName`, `identifier` in `tauri.conf.json`) — no product rename required this slice.
- Version remains the crate/app version; no requirement for auto-updater.

### 3.2 Metal as first-class dogfood build

- Apple Silicon dogfood builds **should** use the Metal script/docs path so STT matches the latency goal direction (NFR-01 / STT-06).
- Default development (`npm run desktop`) and default `desktop:build` **remain CPU** so contributors without Metal expectations are not surprised.
- Intel Mac: CPU path is fine; Metal script may still work where supported — no separate Intel matrix required for acceptance.
- Docs must state clearly: **switching acceleration requires a rebuild**, not a Settings toggle.

### 3.3 Linux packaging (docs-only)

README (or a short subsection linked from README) covers:

- Prerequisites already expected for dev (cmake, audio libs, etc.) at a high level.
- `tauri build` for Linux targets available to the developer’s host.
- Optional mention of AppImage / distro packaging as **Phase 2 / best-effort** — not an acceptance criterion that an AppImage must be produced in CI.
- Full Wayland/X11 hotkey/paste reliability is **not** this spec — see Linux reliability work / STATUS High gap.

### 3.4 CUDA / Vulkan (contributor GPU path)

- Document enabling Cargo features:
  - `cuda` — NVIDIA toolkit required at build time.
  - `vulkan` — portable GPU path where whisper-rs supports it.
- No requirement to ship a prebuilt CUDA binary from this repo in v1.
- No acceptance criterion that requires a physical NVIDIA GPU in CI.
- If multiple features are enabled in one binary (unusual), UI may show a combined label (e.g. list enabled backends) — exact multi-feature string is implementer choice as long as it is truthful.

### 3.5 Acceleration status in UI

- **Required surface:** Settings → **Whisper model** area.
- Show a **read-only** line reflecting **compile-time** features of the running binary, for example:
  - `Acceleration: Metal`
  - `Acceleration: CPU`
  - `Acceleration: CUDA`
  - `Acceleration: Vulkan`
  - (or equivalent clear wording)
- Implementation: expose via existing status snapshot / Tauri command using `cfg!(feature = "…")` (or equivalent), not runtime GPU probes.
- **No toggle** that claims to enable Metal/CUDA at runtime.
- **Apple Silicon + CPU-only:** add a **soft one-line hint** near the label pointing at the Metal rebuild docs/script (e.g. “For faster STT on Apple Silicon, rebuild with Metal — see README”). Must not modal, must not block Load model / dictation.
- Linux CPU-only: extra CUDA/Vulkan hint is **optional**, not required for acceptance.
- Corrupt/missing UI fields: never crash Settings; if the backend fails to report, show `unknown` or omit with a log line — prefer always reporting from compile-time flags so this is rare.

### 3.6 README contract (minimum sections)

README must gain or expand:

1. **Package (macOS)** — how to produce unsigned app/dmg; where output lands; Gatekeeper unsigned notes.
2. **Metal dogfood** — the first-class script + when to use it vs CPU default.
3. **Linux build** — high-level `tauri build` / deps pointer; GPU features as advanced.
4. **CUDA / Vulkan** — feature flags + “toolkit required at build; not a Settings switch.”
5. Pointer that acceleration status appears in **Settings → Whisper model**.

Do not invent a second living STATUS-style packaging tracker; this spec + issues are the contract.

### 3.7 Interaction with other features

| Feature | Interaction |
| --- | --- |
| Whisper model path / load | Unchanged; accel line is adjacent context only |
| Dictation / Command Mode | Unchanged; faster STT when Metal/CUDA build is used |
| Mic picker / silence trim / tray | Independent |
| Escape cancel / clipboard restore | Unchanged |
| Privacy (NFR-06) | Packaging does not add network; models remain local |

---

## 4. Acceptance criteria

An implementation is done when all of the following pass. **macOS is the gate**; Linux items are docs/UI honesty unless noted.

1. **CPU build path:** `npm run desktop:build` (or documented default) still builds without requiring Metal/CUDA toolkits beyond today’s baseline.
2. **Metal dogfood path:** Named script (e.g. `desktop:build:metal`) builds with Metal feature enabled; binary reports Metal in Settings.
3. **Artifact:** Metal (or CPU) `tauri build` produces a runnable macOS `.app` and/or `.dmg` on a developer machine; README names the output location correctly.
4. **Unsigned install note:** README documents Gatekeeper/unsigned launch well enough for a technical user.
5. **Models not forced in bundle:** App runs model load via existing path/download story; no new hard dependency on embedding ggml in the dmg for acceptance.
6. **UI label — Metal:** A Metal build shows Acceleration: Metal (or equivalent) under Settings → Whisper model.
7. **UI label — CPU:** A CPU-only build shows Acceleration: CPU.
8. **AS hint:** On Apple Silicon with a CPU-only build, the soft Metal rebuild hint is visible; dictation still works.
9. **No fake toggle:** There is no control that claims to switch Metal/CUDA without rebuild.
10. **Linux GPU docs:** README documents `cuda` / `vulkan` features and that toolkits are build-time requirements.
11. **No regression:** Dictation end-to-end still works on a Metal dogfood build with model loaded (smoke).

---

## 5. Suggested implementation seams (non-binding)

Pointers only — implementers may choose equivalent structure:

| Area | Notes |
| --- | --- |
| `package.json` | e.g. `desktop:build:metal`: `tauri build -- --features metal` (exact CLI form per Tauri 2) |
| `src-tauri` status command | Add fields e.g. `stt_accel: "metal" \| "cuda" \| "vulkan" \| "cpu"` from `cfg!` |
| UI Settings | Read-only row under Whisper model; conditional hint for AS + CPU |
| README | Packaging + acceleration sections; keep Quick start CPU-friendly |
| Optional script | Thin `scripts/build-macos-metal.sh` only if npm args are awkward |
| Smoke | Manual once: Metal build → open Settings → confirm label → one dictation |

---

## 6. Out of scope

| Item | Why |
| --- | --- |
| Apple Developer ID signing / notarization / staple | Locked “unsigned first”; later product phase |
| Launch at login / LaunchAgent / “Open at Login” | Explicit later non-goal |
| Public download site / GitHub Releases automation | No CI release pipeline this pass |
| Bundling default Whisper weights in the dmg | Size/license; NFR-09 path-based models |
| Runtime GPU feature switching | whisper-rs features are compile-time |
| Equal-first-class Linux AppImage CI | Docs-only Linux packaging this pass |
| Wayland/X11 hotkey & paste hardening | Separate Linux reliability spec/ticket |
| Windows installer / packaging matrix | Map: not first-class unless retargeted |
| Auto-updater | Not in STATUS packaging gap for this map |
| Changing default Cargo features to Metal always-on | Would surprise non-AS / CI |

---

## 7. Requirements traceability

| ID | Requirement | This spec |
| --- | --- | --- |
| **STT-06** | Acceleration: Metal / CUDA-Vulkan / CPU fallback | Metal dogfood path + compile-time UI + CUDA/Vulkan docs |
| **NFR-08** | Single-user install without Docker/Python | Unsigned macOS app/dmg dogfood path |
| **NFR-09** | Replace models without recompile | Models stay path/download, not forced into package |
| **NFR-01 / NFR-02** | Latency goals / CPU baseline usable | Metal path for dogfood; CPU default remains |
| Phase 1 | Packaging dmg (macOS primary) | Unsigned dmg/app first-class |
| Phase 2 | Linux product packaging | Docs only here |
| STATUS gaps | Packaging + accel packaging UX | Combined contract |

---

## 8. Handoff

**Next step:** run **`/to-tickets`** against this document to publish tracer-bullet GitHub issues (label `ready-for-agent`), then implement the frontier with `/implement`.

Suggested ticket split (non-binding for `/to-tickets`):

1. npm Metal build script + README packaging / Gatekeeper / GPU feature docs  
2. Compile-time accel field in status API + Settings read-only line + AS CPU hint  
3. Optional: one manual smoke checklist issue for Metal dmg + Settings label  

**Do not** expand into signing, notarization, launch-at-login, or model-in-bundle without a new product decision.

