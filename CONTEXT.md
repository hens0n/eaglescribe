# EagleScribe

EagleScribe is a local-first system-wide dictation product that turns a user's speech into text without sending dictation content to a vendor cloud.

## Language

**Tuning**:
A guided, fully local process that compares known spoken phrases with Whisper's raw transcripts to learn user-specific correction rules. It does not modify model weights or microphone settings.
_Avoid_: Model training, microphone calibration

**Tuning Session**:
A single user-initiated attempt to complete Tuning. It may pause and resume after an unexpected interruption, but ends when the user explicitly cancels it or reaches a final outcome.
_Avoid_: Tuning run, training session

**Tuning Phrase**:
A curated, built-in English phrase with known expected text that the user reads during Tuning. V1 does not derive these phrases from dictation history or user-authored content.
_Avoid_: Training phrase, custom phrase, history sample

**Candidate Correction**:
An inactive correction proposed only after the same mismatch is observed in two readings during Tuning. It cannot affect normal dictation until the user approves it.
_Avoid_: Automatic correction, learned rule

**Correction Rule**:
An approved user-specific mapping from a recurring raw recognition to the text the user intended. An unmodified Tuning-origin rule applies only under one of its verified Recognition Fingerprints; under another fingerprint it remains visible but inactive until verified there. Explicitly editing it makes it a user-authored mapping that applies without that verification scope. It becomes an editable, removable Personal Dictionary entry after explicit user approval.
_Avoid_: Candidate correction, model update

**Personal Dictionary**:
The user's single collection of preferred-text mappings, whether entered manually or approved through Tuning. A Tuning-origin Correction Rule may be visibly inactive when it has not been verified under the current Recognition Fingerprint.
_Avoid_: Tuning rule store, hidden corrections

**Recognition Fingerprint**:
The identity of the Whisper model plus the decoder and audio-preprocessing behavior that can affect its raw recognition. Tuning evidence is valid only under the Recognition Fingerprint that produced it, and an unmodified Tuning-origin Correction Rule applies only under one of the fingerprints that verified it.
_Avoid_: Model path, device fingerprint

**Verification Pass**:
The required final stage of Tuning in which different built-in phrases exercise approved Correction Rules to determine whether they improve recognition without introducing wrong replacements.
_Avoid_: Training pass, optional test

**Verification Attempt**:
A single held-out phrase reading within the Verification Pass. It scores each staged Correction Rule independently as successful, failed, or not exercised against the same pre-overlay text; unrelated recognition errors do not determine that rule's outcome.
_Avoid_: Verification result, training attempt

**Tuning Diagnostic Event**:
A content-free local record of a meaningful Tuning outcome or transition, identified by stable codes and non-speech metadata. It never contains audio, transcripts, Candidate Correction text, or Personal Dictionary mappings.
_Avoid_: Tuning log entry, telemetry event

**Tuning Health Summary**:
An on-device aggregate of retained Tuning Diagnostic Events that describes recent Tuning reliability and outcomes without becoming a lifetime activity record.
_Avoid_: Telemetry dashboard, lifetime analytics
