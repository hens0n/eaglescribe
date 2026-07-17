# Guided Tuning acceptance contract

**Status:** Behavior specification

**Date:** 2026-07-16

**Related:** [Design guided user-specific Tuning](https://github.com/hens0n/eaglescribe/issues/25) · [Lock the end-to-end Tuning acceptance contract](https://github.com/hens0n/eaglescribe/issues/27)

## Purpose

This is the implementation-ready end-to-end contract for EagleScribe's guided Tuning flow. It composes the fixed phrase corpus, conservative Candidate Correction inference, Personal Dictionary lifecycle, Verification Pass, recovery, diagnostics, and regression decisions into observable behavior.

The key words **must**, **must not**, **required**, and **may** are normative. If this contract conflicts with a linked upstream decision, this contract controls the end-to-end behavior and the upstream decision remains the source for its lower-level algorithm or data details.

## Product and platform boundary

- Tuning is an explicit Settings flow in the existing EagleScribe window. It never learns passively from normal dictation or dictation history.
- All capture, transcription, inference, checkpointing, verification, diagnostics, and Personal Dictionary updates happen locally. No Tuning path may perform a network request.
- V1 uses only the project-owned built-in English corpus in [V1 Tuning Phrase corpus](./tuning-phrase-corpus.md). It does not accept user-authored phrases and cannot learn unseen names or jargon.
- The release-blocking platform/model rows are Apple Silicon macOS with Metal and Linux x64 with CPU Whisper using the pinned `ggml-base.en` model. The in-window behavior is the same on both rows.
- Linux display-server limitations on global shortcuts or paste injection do not weaken Tuning acceptance: Tuning is an in-window Settings flow. Other compatible models and compiled Whisper backends are best-effort compatibility paths.
- While the Tuning screen is active, Tuning exclusively owns microphone and model operations. An ordinary-dictation start command must be rejected with a clear explanation. Leaving Tuning pauses it at the last durable checkpoint; ordinary dictation can then use only the last committed Personal Dictionary state.

## Recognition and checkpoint identity

A **Recognition Fingerprint** identifies the model content hash plus decoder and audio-preprocessing behavior that can change raw recognition. A Tuning Session checkpoint also records the corpus, normalization, inference, Verification Pass, and dictionary-matcher contract versions.

- Completed evidence is valid only while the entire checkpoint compatibility envelope matches.
- A mismatch in the Recognition Fingerprint or any recorded behavior-contract version makes the checkpoint incompatible. EagleScribe must explain the incompatibility and require explicit **Start over**; it must never silently discard or reinterpret evidence.
- Changing microphones discards only an interrupted attempt. It does not invalidate completed evidence.
- An unmodified Tuning-origin Correction Rule applies only when the current Recognition Fingerprint belongs to that rule's verified-fingerprint set.
- Under another Recognition Fingerprint, the rule remains visible in the Personal Dictionary as **Needs verification for this model** and must not be applied.
- Returning to a previously verified Recognition Fingerprint reactivates the rule automatically.
- Manual entries and explicitly edited Tuning-origin entries are user-authored mappings. They remain active across Recognition Fingerprints; editing a Tuning-origin entry preserves its stable ID and origin, clears its verified-fingerprint set, and marks it `modified_after_verification`.

## Session entry and preflight

V1 permits at most one unfinished Tuning Session and gives it no time-based expiry.

1. Opening Tuning with no unfinished session shows **Ready** and the expected 4–6-minute target, local-only processing, fixed lexical ceiling, two-reading protocol, explicit approval, and required verification for approved rules.
2. Opening Tuning with an unfinished session offers **Resume** or **Start over**, identifies the last durable stage, and explains whether an interrupted attempt must be repeated.
3. Before creating a new session, EagleScribe must prove that the selected model can load, a microphone can open, a Recognition Fingerprint can be computed, and a checkpoint can be atomically written.
4. A failed prerequisite leaves the user outside the session with targeted remediation. It creates no scored evidence or Candidate Correction.
5. Starting Tuning while ordinary dictation is recording or transcribing is not allowed; the existing operation must finish or be cancelled first.

The persistent stage rail is **Ready**, **Practice**, **First reading**, **Second reading**, **Review**, **Verify**, and **Result**. It shows completed, current, remaining, and not-needed stages but is not navigation and cannot bypass required work.

## Practice

- Every new Tuning Session requires one successful Practice attempt using the fixed unscored prompt.
- A successful attempt completes local capture and transcription without an operational error. It is not compared with the expected prompt and cannot create evidence or a Candidate Correction.
- Practice audio and its complete raw transcript must be discarded immediately after transcription.
- Practice completion becomes visible only after its checkpoint save succeeds. A resumed session does not repeat a durably completed Practice stage.

## First and second readings

- The user must produce one valid reading of every fixed Tuning Phrase in Pass A order and then one valid reading of every phrase in the rotated Pass B order.
- The UI uses neutral language such as **Read naturally** and never labels pronunciation wrong.
- During the reading stages it must not reveal raw transcripts, mismatch locations, Candidate Correction hints, or why a phrase did or did not produce evidence. Pass B must not look like an error retry.
- **Retry phrase** discards only the current attempt.
- V1 has no reserve phrase corpus. **Do later** moves the current phrase to the end of its current pass; it does not remove the phrase or relax the requirement for two complete passes.
- A valid attempt is acknowledged only after the minimal derived evidence and new progress are durably checkpointed.
- Audio and complete raw transcripts must never be persisted. An unfinished checkpoint may contain only the minimum derived state needed to resume: stable phrase/probe IDs, attempt state, rejection codes, qualifying normalized `from -> to` signatures, Review decisions, staged-rule state, and verification progress.

Candidate Correction inference must use [Safe Candidate Correction inference](./tuning-inference-spec.md) without a test-only or UI-specific variant. In particular, both separated readings must independently yield the same normalized, context-bearing, multi-word substitution for the same eligible probe span. A Tuning Phrase yields at most one inactive Candidate Correction.

## Review

Review always appears after the second reading, even when there are no actionable Candidate Corrections. It is the first stage allowed to reveal Candidate Correction mappings.

### Review rows

- Each actionable row shows the recognized whole word or phrase, intended text, and that the mismatch recurred in both readings.
- Every actionable Candidate Correction must receive an explicit **Approve** or **Decline** decision. Nothing is preselected, an unanswered row is not an implicit decline, and the primary continue action remains disabled until all rows are resolved.
- Equivalent Candidate Corrections within the session coalesce into one row while retaining every supporting Tuning Phrase ID.
- Candidates with the same canonical `from` and different `to` values are rejected as context-ambiguous; the user is not asked to choose an unsafe global mapping.
- A mapping equivalent to a manual entry, an explicitly edited Tuning-origin entry, or a Tuning-origin rule verified under the current Recognition Fingerprint appears as non-actionable **Already covered by Personal Dictionary** and is not re-verified.
- A mapping equivalent to an unmodified Tuning-origin rule that is not verified under the current Recognition Fingerprint appears as **Verify existing rule for this model**. Approval stages the existing stable entry for the normal Verification Pass; success adds the current fingerprint without creating a duplicate entry.
- If a Candidate Correction shares an existing canonical `from` but proposes another `to`, Review shows both mappings and requires **Keep existing** or **Verify proposed replacement**. There is no silent overwrite.
- Rejected evidence may be summarized only through the neutral grouped meanings defined by [Define local Tuning diagnostics and observability](https://github.com/hens0n/eaglescribe/issues/34). Complete rejected transcripts are never displayed.

### Leaving Review

- If at least one new, replacement, or existing rule is approved for the current Recognition Fingerprint, the primary action is **Continue to verification**.
- If none is approved, the primary action is **Continue to results** and Verify is marked **Not needed**. This is not a skipped Verification Pass because no rule is eligible to verify.
- The unchanged Result distinguishes **No safe corrections found**, **Already covered by Personal Dictionary**, and **Candidate Corrections found, but none approved**.

## Verification Pass

Verification uses only the distinct held-out rows paired with approved rules. It must use the production audio preprocessing, transcription, dictionary matcher, and rule ordering up to a Tuning-scoped overlay; staged rules remain inactive in ordinary dictation.

The complete scoring contract is [Specify Verification Pass and rollback outcomes](https://github.com/hens0n/eaglescribe/issues/28). The end-to-end requirements are:

- Score each staged Correction Rule independently against the same pre-overlay normalized text.
- A rule succeeds only when its exact trigger occurs at the intended tagged span, produces its expected target, and changes no normalized words elsewhere.
- A first valid **Not exercised** outcome permits exactly one additional valid reading of that row. A second non-exercise rolls the rule back as **Could not verify**.
- **Target not corrected**, **Harmful change**, and **Rule interaction** are terminal for the affected rule and receive no same-session retry.
- A coalesced rule must succeed on every distinct supporting held-out row.
- Apply the complete approved-rule overlay to every held-out row. Unexpected overlap, cascading, or ordering changes roll back every participating rule while leaving unrelated rules eligible.
- Continue verifying unaffected rules after an individual rollback. A rolled-back rule cannot be force-kept or rediscovered without a later Tuning Session and fresh two-reading evidence.
- Operational microphone, model, transcription, or storage errors are not valid Verification Attempts and never count as rule failure or consume the one non-exercise retry.

If a staged Personal Dictionary key changes outside Tuning after verification begins, only that rule returns to conflict Review. Completing Review produces a new approved-rule set and restarts the entire Verification Pass; all earlier Verification Attempts are discarded because their combined-overlay safety result no longer describes the staged set.

When verifying an existing inactive rule for a new Recognition Fingerprint:

- Success adds the current fingerprint to the same stable entry's verified-fingerprint set.
- Failure or decline leaves the entry, mapping, provenance, and previously verified fingerprints unchanged.
- No duplicate canonical `from` entry is created.

## Commit and Result

After every approved rule has a terminal verification outcome, EagleScribe performs one atomic Personal Dictionary update:

- Commit only rules marked **Kept** and verified-fingerprint additions that succeeded.
- Discard rolled-back staged rules.
- A kept conflicting replacement atomically removes the old entry and creates the new Tuning-origin entry; the old mapping is never partially overwritten.
- Unrelated dictionary edits merge normally. A concurrent change to a staged key cannot be overwritten and must return that rule to Review.
- The Result stage must not appear until the atomic dictionary write and terminal session-state transition both succeed.
- If no rule is kept, the session still completes successfully with the Personal Dictionary unchanged.

Result lists every approved rule separately as **Kept**, **Rolled back — could not verify**, **Rolled back — did not correct the intended phrase**, **Rolled back — changed other text**, or **Rolled back — interacted with another rule**. It supports partial success, groups interacting rules, and links kept rules to the Personal Dictionary for editing or removal. It provides no force-keep or same-session retry control.

Terminal success deletes the unfinished checkpoint and its derived evidence. The committed Personal Dictionary retains only the mapping, stable ID, origin, verified-fingerprint set, verification timestamp, and edit status required by the lifecycle contract; it stores no audio, transcript, phrase, or probe evidence.

## Pause, interruption, cancellation, and storage failure

- Leaving Tuning, quitting, crashing, sleeping, or losing a device pauses the session at its last durable checkpoint. An interrupted or partially persisted attempt is discarded and must be read again.
- Cancelling a recording aborts only the current attempt. **Cancel Tuning** ends the Tuning Session.
- **Cancel Tuning** and **Start over** require confirmation after any scored reading, Review decision, or Verification Attempt has been durably recorded. Ready and Practice alone may be abandoned without confirmation.
- Confirmation says that unfinished Tuning evidence will be deleted and committed Personal Dictionary entries will remain unchanged.
- Successful cancellation/start-over atomically deletes the checkpoint, approvals, staged rules, and derived content. If cleanup cannot be saved, EagleScribe reports a storage failure rather than false success.
- Model, microphone, and transcription failures pause the current task, discard the failed attempt, retain completed evidence, and offer targeted remediation plus user-initiated retry or cancellation. There is no automatic or fixed retry limit.
- A storage failure hard-stops forward progress. The UI must not show the pending phrase, Review decision, Verification Attempt, or stage transition as complete. Recoverable in-memory state may be saved with **Retry Save**; after restart, only the last acknowledged durable checkpoint is trusted.

No phrase, Review decision, Verification Attempt, or stage transition becomes visibly complete before its atomic checkpoint write succeeds.

## Diagnostics and privacy

[Define local Tuning diagnostics and observability](https://github.com/hens0n/eaglescribe/issues/34) is normative. At minimum:

- Tuning Diagnostic Events are structured, local-only, and content-free. They never contain audio, expected or observed transcript text, Candidate Correction text, Personal Dictionary mappings, arbitrary errors, paths, device names, or secrets.
- Diagnostics are not authoritative session state and cannot change inference, verification, recovery, or dictionary commit behavior.
- Events for an unfinished session may remain while it is resumable. Terminal events are retained for 30 days or the latest 20 terminal sessions, whichever removes them sooner.
- **Clear Tuning Diagnostics** does not alter the checkpoint, staged rules, or Personal Dictionary. **Export Tuning Diagnostics** writes only the approved content-free schema to a user-chosen path and never uploads it.
- Cancellation deletes Tuning evidence but may retain its content-free diagnostic trail under the bounded policy.
- No Tuning audio or complete raw transcript is written to diagnostics, logs, stderr, crash reports, analytics, or network traffic.

## Acceptance scenarios

An implementation is complete only when these observable scenarios and the lower-level fixture suites all pass.

| ID | Scenario | Required result |
| --- | --- | --- |
| TUN-01 | Start with valid model, microphone, and storage | Ready creates one local checkpoint and enters required Practice. |
| TUN-02 | Preflight model, microphone, fingerprint, or storage failure | No session evidence is created; targeted remediation is shown. |
| TUN-03 | Complete Practice | Capture/transcription succeeds, content is discarded, and First reading begins only after save. |
| TUN-04 | Complete both reading passes | All ten phrases have one durable valid attempt in each prescribed order; Pass B reveals no inference hints. |
| TUN-05 | Choose Do later | Phrase moves to the end of its pass and remains required. |
| TUN-06 | Retry or interrupt a phrase | Only the current attempt is discarded; no partial attempt counts. |
| TUN-07 | Produce a safe repeated mismatch | Exactly one inactive Candidate Correction appears in Review under the inference contract. |
| TUN-08 | Produce unsafe, inconsistent, or no mismatch evidence | No Candidate Correction appears; only permitted neutral grouped explanations are available. |
| TUN-09 | Reach Review with zero actionable rows | Review still appears; Verify is Not needed; Result explains the unchanged outcome. |
| TUN-10 | Leave a Review row unanswered | Continue remains disabled and no approval is inferred. |
| TUN-11 | Encounter an equivalent active mapping | It appears as Already covered and is not re-verified. |
| TUN-12 | Encounter an equivalent rule inactive for this fingerprint | Approval verifies the existing stable entry; success adds the fingerprint without duplication. |
| TUN-13 | Encounter a conflicting dictionary mapping | Keep existing or verify replacement is required; no silent overwrite occurs. |
| TUN-14 | Approve no candidates | Verify is Not needed and the dictionary remains unchanged. |
| TUN-15 | Verify one rule successfully | The rule is Kept and committed only at the final atomic update. |
| TUN-16 | First verification reading does not exercise a rule | Exactly one additional valid reading is allowed. |
| TUN-17 | Rule fails or twice is not exercised | Only that rule rolls back with the canonical reason; unaffected rules continue. |
| TUN-18 | Approved rules interact | Every participant rolls back; unrelated rules remain eligible. |
| TUN-19 | Staged key changes during verification | That rule returns to Review and the entire Verification Pass restarts after Review. |
| TUN-20 | Some rules pass and others fail | One atomic commit keeps only successful rules and Result reports partial success. |
| TUN-21 | All approved rules roll back | Session completes successfully with the dictionary unchanged. |
| TUN-22 | Pause and resume | Last durable stage and progress are restored; interrupted attempt is repeated. |
| TUN-23 | Recognition or behavior compatibility changes | Resume is refused with an explanation and explicit Start over. |
| TUN-24 | Microphone changes | Completed evidence remains; only an in-flight attempt is discarded. |
| TUN-25 | Cancel after scored progress | Confirmation appears; successful cleanup leaves no checkpoint or unverified active rule. |
| TUN-26 | Checkpoint save fails | Forward progress stops, pending progress is not acknowledged, and Retry Save/Cancel is offered. |
| TUN-27 | Switch away from a rule's verified fingerprint | Unmodified Tuning-origin rule is visible but inactive; switching back reactivates it. |
| TUN-28 | Explicitly edit a Tuning-origin rule | Stable ID/origin remain, verification scope clears, and the user-authored mapping becomes globally active. |
| TUN-29 | Use Tuning on required macOS and Linux rows | The same flow and outcomes pass; Linux hotkey/paste limitations do not affect it. |
| TUN-30 | Inspect storage, logs, export, and traffic with sentinels | No forbidden Tuning content or network request is present. |

## Regression and release gates

[Define the Tuning regression fixture strategy](https://github.com/hens0n/eaglescribe/issues/33) is normative:

- Every pull request runs platform-independent corpus, inference, dictionary-matcher, Verification Pass, diagnostics, and full-session fixtures on macOS and Linux CI.
- Changes to Tuning, STT, preprocessing, Whisper dependencies, the pinned model, fixture audio, or decoding replay the project-owned audio corpus on Apple Silicon macOS/Metal and Linux x64/CPU.
- Every tagged release repeats both shipped replay rows and a live-microphone Tuning smoke on each shipped platform.
- Exact transcript drift requires reviewed fixture changes. An unsafe Candidate Correction can never be blessed by updating a snapshot.
- The 4–6-minute duration is a measured target, not a hard pass threshold. An ordinary session exceeding seven minutes requires documented investigation before release.
