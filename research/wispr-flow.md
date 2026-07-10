# Wispr Flow — Product Research

**Purpose:** Competitive and product research for designing **TalonType**, a fully local dictation alternative.  
**Date:** 2026-07-10  
**Related:** [requirements-local-app.md](./requirements-local-app.md), [stack-decision.md](./stack-decision.md)

---

## 1. Summary

**Wispr Flow** is a cloud-first AI voice dictation product for **Mac, Windows, iPhone, and Android**. Users hold (or toggle) a global hotkey, speak into any text field, and receive **polished, auto-edited** text inserted into the focused app—claimed at roughly **4× typing speed** (marketing: ~220 wpm voice vs ~45 wpm keyboard).

Core product loop:

1. Capture speech system-wide  
2. **Transcribe always in the cloud** (no offline mode)  
3. AI polish (fillers, punctuation, backtrack, styles)  
4. Inject formatted text into the active app  
5. Optionally transform text via **Command Mode** (Pro)

**Strategic takeaway for TalonType:** Flow’s UX bar (hotkeys, polish, dictionary, snippets, context-aware style) is excellent, but its **architecture is cloud-only STT**. Privacy Mode and Cloud Sync only govern *training* and *storage*—not whether audio leaves the device. That gap is TalonType’s primary wedge: **Wispr-class UX with on-device inference by default**.

---

## 2. Product Overview

| Attribute | Detail |
| --- | --- |
| **Product name** | Wispr Flow (Wispr AI, Inc.) |
| **Category** | System-wide AI voice-to-text / “Voice OS” |
| **Platforms** | macOS, Windows, iOS, Android; enterprise Admin Portal |
| **HQ** | San Francisco; founded 2023 |
| **Positioning** | “Don’t type, just speak” — polished writing in every app |
| **Claimed speed** | ~4× faster than typing |
| **Languages** | 100+ (English most mature) |
| **STT architecture** | **Cloud always** — explicitly documented; no on-premise / offline product |
| **Business model** | Free tier + Pro subscription + Enterprise / Teams |

### Target personas (from marketing)

- Knowledge workers (email, Slack, Notion, docs)  
- Developers (Cursor, VS Code, commit messages, natural-language coding prompts)  
- Students, creators, sales, support, lawyers, leaders  
- Accessibility users who prefer speech over keyboard  

### Value proposition

Speak naturally → get clean, formatted prose (or code-aware text) without cleaning up raw ASR. Personal dictionary, snippets, and styles make the product feel “trained” to the user over time. Cross-device sync of prefs and (optionally) notes reinforces habit and lock-in.

---

## 3. Features

### 3.1 System-wide dictation

- Works in **any app with a text field**: Gmail, Slack, Notion, Cursor, messaging apps, browsers, IDEs, etc.  
- Desktop: menu-bar / tray app; mobile: native apps / keyboard-style flow.  
- Marketing emphasizes **whisper-level** speech still works (shared offices).  
- Dictation **auto-stops after 20 minutes** (warning at 19).  
- If paste into the field fails, text is copied to the clipboard with a notification.

### 3.2 Hotkeys & interaction modes

Defaults and behaviors (from official docs):

| Action | macOS default | Windows default |
| --- | --- | --- |
| **Push-to-talk** | `Fn` (or `Ctrl+Opt` if no Apple Fn) | `Ctrl+Win` |
| **Hands-free** | `Fn+Space` | `Ctrl+Win+Space` |
| **Command Mode** | `Fn+Ctrl` (or `Cmd+Ctrl+Opt`) | `Ctrl+Win+Alt` |
| **Cancel** | `Esc` | `Esc` |
| **Paste last transcript** | `Cmd+Ctrl+V` | `Shift+Alt+Z` |
| **Copy last transcript** | `Cmd+Ctrl+C` | `Shift+Alt+X` |

Additional product behavior:

- **Hold** for push-to-talk; **double-tap** or dedicated shortcut for hands-free.  
- Up to **4 shortcuts per binding**, ≤3 keys, must include a modifier or valid mouse button.  
- Mouse: middle click, Mouse 4–10 (not left/right).  
- Transforms (Polish, Prompt Engineer, etc.) get optional number shortcuts.  
- Scratchpad open/dictation shortcut is opt-in.  
- Large reserved-key lists on both platforms to avoid OS conflicts.

**Implication for local clone:** Hotkey quality and conflict handling are a major product surface—not an afterthought.

### 3.3 Smart Formatting & Backtrack

Flow’s “AI Auto Edits” layer goes beyond raw transcription:

- Remove **filler words** (“um”, “uh”, etc.)  
- **Auto punctuation** and capitalization  
- **Numbered / bulleted lists** from spoken structure  
- **Backtrack**: spoken corrections (“actually…”, “scratch that…”) rewrite the in-progress result  
- Mid-sentence continuation aware of surrounding text (casing, spaces, trailing punctuation by style)  
- **Styles**: Formal / Casual / Excited / Very Casual, often driven by app category (desktop; English emphasis)

This polish layer is as important as STT accuracy for perceived quality.

### 3.4 Command Mode (Pro / trial)

Experimental, desktop-only (Mac & Windows), **paid or free-trial**:

- Highlight text → hold Command Mode shortcut → speak a command → selection is **replaced**  
- Or no selection → generate / answer **inline** at cursor  
- Examples: “Make this more concise”, “Translate to Polish”, “Turn this outline into an essay”  
- Voice-driven **Polish settings** changes (apply only after confirmation)  
- Recall past dictations/notes (when history exists)  
- Max selection ~**1,000 words**  
- Cancel with `Esc`; undo with system undo  
- Separate from normal dictation path; can hit “servers busy”

Also related: say **“press enter”** at end of dictation to submit chat/prompt UIs (experimental).

### 3.5 Dictionary & snippets

| Capability | Behavior |
| --- | --- |
| **Personal dictionary** | Custom names, jargon, spellings; can **auto-add** when user edits pasted transcription |
| **Snippets** | Speak a cue → expand to full formatted text (calendar links, FAQs, intros) |
| **Team shared dictionary/snippets** | Pro/team collaboration |
| **Always sync** | Dictionary + snippets sync across devices **regardless** of Cloud Sync |

These are high-retention personalization features and relatively cheap to implement locally.

### 3.6 Context Awareness

Desktop feature (Mac primary; Windows more limited); **on by default**:

- Reads **active app / site** and nearby text (accessibility APIs; optional screen OCR off by default)  
- Improves proper nouns (e.g. email recipients)  
- Maps apps to style categories: Email, Work messaging, Personal messaging, Other  
- Special handling for Notion placeholders, AI chat hints, code editors (Cursor/Windsurf file tagging; VS Code filename memory)  
- Password fields excluded (best-effort on Mac)  
- Context may be **sent with cloud transcription** for accuracy  
- Enterprise can disable org-wide  

**Privacy note:** Context is a dual-edged sword—great UX, but local clones must keep context **on-device** by default if positioning on privacy.

### 3.7 Hub, Scratchpad, Transforms

Product surface beyond pure dictation:

- **Hub / history** of dictations (depends on storage settings)  
- **Scratchpad / notes** with optional cross-device sync (Cloud Sync)  
- **Transforms**: Polish, Prompt Engineer, custom transform slots, View Diff  
- Meeting-related features (Notetaker) gated on Cloud Sync / plan  
- Usage dashboards for teams  

### 3.8 Developer-oriented features

Marketed to engineers:

- Natural language → structured text in Cursor, VS Code, etc.  
- Syntax / file-name awareness, dev jargon in dictionary  
- File tagging in Cursor/Windsurf  
- Useful for commit messages, PR descriptions, chat-with-AI coding  

### 3.9 Enterprise / Business

| Feature | Notes |
| --- | --- |
| Centralized billing / admin | Teams + Enterprise |
| Shared dictionary & snippets | Pro+ |
| Usage dashboards | Basic → Advanced (Enterprise) |
| SSO / SAML / OIDC | Enterprise |
| Enforced Privacy Mode / ZDR | Enterprise admin |
| HIPAA BAA | Available; locks ZDR-style handling |
| SOC 2 / ISO 27001 | Compliance program (see privacy section) |
| MSA / DPA | Enterprise contracts |

No on-premise deployment; multi-tenant SaaS only.

---

## 4. User Flows

### 4.1 First-run (desktop)

1. Download / install from wisprflow.ai  
2. Sign in / create account  
3. Grant **Microphone** (+ **Accessibility** on Mac for inject/context)  
4. 14-day **Pro trial** starts (no card required for trial path)  
5. Onboarding: hotkey demo, optional style preferences, dictionary hints  
6. Ready: tray/menu-bar presence; hold hotkey to dictate  

### 4.2 Everyday dictation (push-to-talk)

```
Focus text field
    → Hold PTT hotkey (e.g. Fn / Ctrl+Win)
    → Speak (optional visual waveform / Flow Bar)
    → Release
    → Cloud STT + polish
    → Text injected (or clipboard fallback)
```

### 4.3 Hands-free long form

```
Double-tap or hands-free shortcut
    → Speak at length (until stop / timeout)
    → Stop
    → Polished paste
```

### 4.4 Correction / backtrack mid-utterance

User speaks correction phrases; polish layer rewrites draft before paste (product differentiator vs dumb ASR).

### 4.5 Command Mode edit

```
Select text (optional)
    → Hold Command Mode shortcut
    → Speak instruction
    → Release
    → Cloud LLM transform
    → Replace selection / insert result
```

### 4.6 Snippet expansion

```
PTT → speak snippet trigger phrase → release
    → Expanded body pasted (may combine with polish)
```

### 4.7 Privacy configuration

```
Settings → Data & Privacy
    → Privacy Mode (training on/off)
    → Private Cloud Sync (storage on/off)
    → Optional: Context Awareness, local store policy
```

---

## 5. Privacy & Data Architecture

This section is critical for competitive positioning. Sources: [Data Controls](https://wisprflow.ai/data-controls), [Privacy Mode / Cloud Sync docs](https://docs.wisprflow.ai/articles/4709791908-understanding-privacy-mode-and-cloud-sync), [Security FAQ](https://docs.wisprflow.ai/articles/3467817258-security-and-compliance-faq).

### 5.1 Hard architectural facts

| Fact | Detail |
| --- | --- |
| **Transcription always cloud** | Documented explicitly: “Transcription always occurs on the cloud.” |
| **No offline STT** | No local model path for core dictation |
| **No on-prem product** | Multi-tenant SaaS only |
| **Not end-to-end encrypted** | TLS + at-rest encryption; **Wispr must decrypt audio to transcribe** |
| **US hosting** | Customer data processed/stored in the United States |

### 5.2 Two independent privacy controls

| Control | What it governs |
| --- | --- |
| **Privacy Mode** | Whether dictation data (audio, transcript, edits) may be used to **train / improve** Wispr models |
| **Private Cloud Sync** | Whether transcription data (transcripts, audio, history) is **stored** on Wispr servers for history / Scratchpad sync / related features |

**Zero Data Retention (ZDR)** = **Privacy Mode ON + Cloud Sync OFF**  
→ no training + no server-side storage of dictation pipeline artifacts (after real-time processing).

| Privacy Mode | Cloud Sync | Outcome |
| --- | --- | --- |
| OFF | ON | Training possible + stored; full features |
| ON | ON | No training, but stored for sync/history features |
| ON | OFF | **ZDR** — no training, no storage |
| OFF | OFF | No storage of sync-gated data, but training may still use real-time data |

**Important misconceptions to avoid in our marketing:**

- Privacy Mode is **not** “audio never leaves device.”  
- ZDR is **not** offline.  
- Cloud Sync OFF still allows **real-time cloud processing**.

### 5.3 What always syncs

Independent of Cloud Sync:

- Custom **dictionary**  
- **Snippets** / custom prompts  
- Account settings & preferences  

### 5.4 Context, analytics, local data

- App name / surrounding textbox content used for formatting even in normal operation.  
- Context Awareness may send nearby text / proper nouns with the request.  
- Usage stats (e.g. word counts) may be collected regardless of privacy toggles.  
- Desktop **local storage** is a third axis: store normally / delete after 24h / never store.  
- Enterprise can enforce Privacy Mode, Cloud Sync, local policy, and Context Awareness.

### 5.5 Encryption & compliance posture

- **In transit:** TLS 1.2+  
- **At rest:** AES-256 (when stored)  
- **E2E:** Not offered for dictation content  
- HIPAA: BAA available; typically locks Privacy Mode on + Cloud Sync off  
- SOC 2 / ISO 27001 program (see Security FAQ for current attestation status)  
- Third-party LLM/STT subprocessors: Wispr claims ZDR agreements so *subprocessors* don’t train/store—**Wispr itself** still processes cloud audio  

### 5.6 Mental model for TalonType

```
Wispr Flow (default path):
  Mic → client → CLOUD STT → CLOUD polish/LLM → client inject
  Privacy toggles only affect training + retention, not the hop off-device.

TalonType (target path):
  Mic → local STT → local polish (± local LLM) → inject
  Network never on the critical path by default.
```

---

## 6. Pricing Signal

Primary source: [wisprflow.ai/pricing](https://wisprflow.ai/pricing) (as of research date).

| Plan | Price | Core limits / extras |
| --- | --- | --- |
| **Flow Basic (Free)** | $0 | **2,000 words/week** desktop (Mac/Windows); **1,000 words/week** iPhone; Android unlimited for a limited time; dictionary & snippets; 100+ languages; Privacy Mode; HIPAA-ready |
| **Flow Pro** | **$15/user/mo** monthly, or **$12/user/mo** annual (~$144/yr) | Unlimited words all platforms; **Command Mode**; priority support; early access; team collab features |
| **Enterprise** | Contact sales | SSO/SAML, enforced HIPAA / Privacy Mode, SOC2/ISO, advanced dashboards, dedicated support, bulk discounts |
| **Student** | Discount path | Marketing: months free + reduced Pro (verify current student page) |
| **Trial** | 14 days Pro | New accounts; no card required for trial messaging |

**Product implication:** Free tier is a real habit-former but **exhausts quickly** for daily users (~2k words ≈ a few short sessions). Pro is the serious-user floor. Local unlimited dictation is a strong counter-offer if latency/accuracy are good enough.

---

## 7. Technical Implications for a Local Clone

### 7.1 Must-match UX (table stakes)

1. **Global hotkey** with push-to-talk + toggle/hands-free  
2. **System-wide paste/inject** into focused field + clipboard fallback  
3. **Polish layer** (fillers, punctuation, simple backtrack) — not raw Whisper dump  
4. **Dictionary + snippets**  
5. **Tray/menu-bar** app; low friction permissions onboarding  
6. **Status feedback** (listening / processing / error)  

### 7.2 Differentiating architecture

| Flow | TalonType target |
| --- | --- |
| Cloud STT always | **whisper.cpp** (or equivalent) on-device |
| Cloud LLM for Command Mode | **llama.cpp** / local GGUF optional |
| Account + subscription gate | Offline-first; no account required for core path |
| Context sent to cloud | Context stays local |
| Word limits | Unlimited (hardware-bound) |

### 7.3 Hard engineering problems Flow already solved

- Reliable **text injection** across Electron, browsers, native apps (Accessibility / simulated paste)  
- Hotkey stacks that don’t fight OS reserved combos  
- Latency UX while network STT runs (we trade network RTT for local compute)  
- Mid-utterance correction language  
- App-category style heuristics  

### 7.4 Local-specific challenges

- Model download / size / Metal-CUDA-Vulkan acceleration  
- First-token and full-utterance latency on CPU-only machines  
- Multilingual models vs English-only small models  
- Optional LLM polish quality vs speed  
- Linux Wayland hotkey/paste edge cases  
- User education: permissions, model selection, offline guarantees  

### 7.5 Suggested stack alignment (see ADR)

Already decided for TalonType:

- Rust + Tauri 2  
- whisper.cpp via whisper-rs  
- llama.cpp for later Command Mode  
- Rules-first polish for MVP  
- cpal audio; clipboard + paste inject  

---

## 8. Competitor Landscape

| Product | Stance | Notes vs Flow |
| --- | --- | --- |
| **Wispr Flow** | Cloud AI polish, multi-platform, freemium | Category leader UX; privacy not offline |
| **Superwhisper** | Local / hybrid Mac focus | Closer privacy story; often cited as Flow alternative |
| **macOS / Windows built-in dictation** | Free, limited polish | Baseline Flow beats on formatting & cross-app AI edits |
| **Dragon / Nuance** | Enterprise classic | Heavy, expensive, legacy positioning |
| **Otter / Fireflies / etc.** | Meeting notes, not system dictation | Adjacent, not direct |
| **Whisper-based open tools** | DIY local | Accuracy/UX gap; TalonType productizes this |
| **TalonType (this project)** | **Local-first** Mac + Linux | Compete on privacy + unlimited offline; match Flow UX over time |

Positioning options:

1. **Privacy maximalist** — never leaves device; for regulated / security-sensitive users  
2. **Power-user unlimited** — no word caps, no subscription tax  
3. **Linux-friendly** — Flow is weak/absent on Linux desktop  
4. **Open / inspectable stack** — models and pipeline user-controlled  

---

## 9. Gaps & Opportunities

### Flow gaps (opportunities for TalonType)

1. **No true offline / local STT** — airplane mode, air-gapped, sensitive orgs  
2. **Not E2E encrypted** — provider can read content in transit processing  
3. **Word-limited free tier** — daily users hit walls  
4. **Linux desktop** — not a Flow platform; open field  
5. **Account required / SaaS dependency** — outages, region latency, policy changes  
6. **Privacy Mode marketing confusion** — users may believe ZDR = local; we can be clearer  
7. **Vendor lock-in on dictionary/history** — export/portability as a feature  
8. **Cost at scale** — $12–15/user/mo for teams adds up  

### Flow strengths (don’t underestimate)

1. Polish quality from large cloud models  
2. Multilingual breadth (100+)  
3. Mobile parity + cross-device sync  
4. Command Mode convenience  
5. Context Awareness maturity on Mac  
6. Enterprise sales motion (HIPAA, SSO, admin)  
7. Brand and distribution  

### Priority bets for TalonType

| Priority | Bet |
| --- | --- |
| P0 | Offline-by-default pipeline with Flow-like hotkey + inject |
| P0 | Deterministic polish that makes Whisper output usable |
| P1 | Dictionary + snippets parity |
| P1 | Hold-to-talk + toggle modes; tray UX |
| P2 | Local Command Mode (small LLM) |
| P2 | Local context (app name + nearby text) without network |
| P3 | Windows; advanced history UI; multi-language packs |

---

## 10. Sources

### Primary — product & marketing

| Resource | URL |
| --- | --- |
| Home | https://wisprflow.ai |
| Features | https://wisprflow.ai/features |
| Pricing | https://wisprflow.ai/pricing |
| Data Controls | https://wisprflow.ai/data-controls |
| Privacy Policy | https://wisprflow.ai/privacy |
| Business / Teams | https://wisprflow.ai/business |
| Developers | https://wisprflow.ai/developers |
| Students | https://wisprflow.ai/students |
| What’s New | https://wisprflow.ai/whats-new |
| Demo | https://wisprflow.ai/demo |
| Trust Center | https://trust.wispr.ai |

### Primary — Help Center (docs.wisprflow.ai)

| Resource | URL |
| --- | --- |
| Help Center home | https://docs.wisprflow.ai |
| Privacy Mode & Cloud Sync | https://docs.wisprflow.ai/articles/4709791908-understanding-privacy-mode-and-cloud-sync |
| Private Cloud Sync preferences | https://docs.wisprflow.ai/articles/9609615338-private-cloud-sync-and-data-sharing-preferences-in-wispr-flow |
| Security & compliance FAQ | https://docs.wisprflow.ai/articles/3467817258-security-and-compliance-faq |
| Command Mode | https://docs.wisprflow.ai/articles/4816967992-how-to-use-command-mode |
| Hotkey shortcuts | https://docs.wisprflow.ai/articles/2612050838-supported-unsupported-keyboard-hotkey-shortcuts |
| Context Awareness | https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness |
| Privacy collection | https://docs.wisprflow.ai/collections/8370538680-privacy_security_data |
| Billing collection | https://docs.wisprflow.ai/collections/9999370675-billing_plans |
| Status | https://statuspage.incident.io/wispr-flow |

### Internal

| Resource | Path |
| --- | --- |
| Local app requirements | [requirements-local-app.md](./requirements-local-app.md) |
| Stack ADR | [stack-decision.md](./stack-decision.md) |

---

## 11. One-page cheat sheet

| Topic | Answer |
| --- | --- |
| What is Flow? | Cloud AI system-wide dictation, multi-platform |
| Offline? | **No** — STT always cloud |
| Privacy Mode? | Controls **training**, not local processing |
| Cloud Sync? | Controls **storage** of audio/transcripts/history |
| ZDR? | Privacy Mode ON + Cloud Sync OFF |
| E2E encrypted? | **No** |
| Free limit? | **2000 words/week** desktop |
| Pro price? | **$15/mo** or **$12/mo** annual |
| Mac hotkey? | **Fn** (PTT) |
| Windows hotkey? | **Ctrl+Win** (PTT) |
| Command Mode? | Pro; desktop; transform selected/inline text |
| TalonType wedge? | Same UX goals, **fully local** STT/polish |

---

*Research compiled for TalonType product design. Pricing and policy details change; re-verify primary URLs before external claims.*
