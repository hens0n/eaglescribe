---
status: accepted
---

# Scope Tuning-origin rules to verified Recognition Fingerprints

An automatically inferred Correction Rule is safe evidence only for the Whisper model, decoder, and audio-preprocessing behavior that produced and verified its recurring mismatch. EagleScribe therefore keeps a set of verified Recognition Fingerprints on each unmodified Tuning-origin Personal Dictionary entry and applies that rule only under a member of the set; under another fingerprint the entry remains visible but inactive until the normal Tuning Verification Pass adds it. Manual entries and explicitly edited Tuning-origin entries are deliberate user-authored mappings and remain active across fingerprints.

This chooses safety over silently carrying inferred behavior between recognizers. The alternative—leaving all rules globally active after a model or decoder change—would preserve convenience but could turn a once-correct raw-recognition substitution into a harmful replacement without new held-out evidence. One stable entry may accumulate multiple verified fingerprints so re-verification does not create duplicate canonical keys, and a failed verification under a new fingerprint cannot invalidate fingerprints that already succeeded.
