# In-process llama.cpp — product stance

**Status:** Decision (deferred / non-goal for current path)  
**Date:** 2026-07-10  
**Related:** [stack-decision.md](./stack-decision.md) · [requirements-local-app.md](./requirements-local-app.md) (`CMD-02`) · map ticket [Spec in-process llama.cpp stance for to-tickets](https://github.com/hens0n/eaglescribe/issues/12)

This document records the **locked stance** for Command Mode hosting so the decision survives without a live session-status handoff. It is **not** a Ready-for-`/to-tickets` behavior spec and does **not** justify implementation issues.

---

## 1. Stance

| Topic | Decision |
| --- | --- |
| Command Mode runtime | **HTTP only** to a **user-configured localhost** OpenAI-compatible server (Ollama, `llama-server`, LM Studio, etc.) |
| In-process llama.cpp | **Deferred / non-goal** on the current product path — do **not** link llama.cpp into the EagleScribe binary for Command Mode |
| Bundled LLM weights | **No** shipping GGUF / llama weights inside EagleScribe for Command Mode |
| External server dependency | Optional for **dictation** (app ships without it); **Command Mode** requires a reachable configured endpoint and **fails clearly** if unreachable |
| Privacy | Command Mode traffic stays on **localhost**; **no** cloud Command Mode without a separate product decision |
| Implementation tickets | **None** from this decision — no `/to-tickets` pass, no `ready-for-agent` work for in-process llama |

**Why:** Command Mode already works via localhost HTTP. Linking llama.cpp **and** whisper.cpp would add dual C++ cost, binary size, build-matrix complexity, and in-app model-download UX without a product gap that HTTP cannot cover today.

---

## 2. Soft reopen

In-process llama may be **reconsidered later** only if someone deliberately reopens:

1. Dual C++ link cost (whisper.cpp + llama.cpp), binary size, and build features (Metal/CUDA/Vulkan for both), **and**
2. Model distribution / download UX inside the app, **and**
3. A concrete product reason that **HTTP-local Command Mode is no longer good enough**.

There is **no** target date and **no** placeholder roadmap ticket. Reopening is a new effort (new map or ADR), not a continuation of a hidden backlog item.

---

## 3. Requirements interpretation

- **CMD-02** means: commands run via a **local LLM** when the user has configured a **localhost OpenAI-compatible endpoint** (and model id as required by that server) — **not** “llama.cpp linked in-process.”
- Historical requirements / early stack language that said “llama.cpp” for Command Mode refer to the **local GGUF / llama.cpp *ecosystem*** (including `llama-server` / Ollama), not a mandatory in-process link.

---

## 4. Intentional non-goals (this decision)

- In-process llama.cpp bindings or spawn-of-bundled-binary inference
- Bundled Command Mode GGUF models
- Ready-for-agent implementation tickets for in-process LLM
- Cloud Command Mode, accounts, or non-localhost defaults
- Near-term HTTP UX upgrades (setup wizard, model picker) — may be separate product work; **not** implied by this stance

---

## 5. What already shipped (context)

- Command Mode: select text → speak instruction → rewrite via `ureq` → localhost `/v1/chat/completions`
- Settings: `llm_base_url`, `llm_model`
- Distinct waiting-LLM status and clear errors when the server is down

No change to that path is required by this document.
