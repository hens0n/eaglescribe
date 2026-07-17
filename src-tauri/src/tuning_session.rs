//! Durable state for the guided Tuning Session through its required Practice stage.

use crate::recognition::RecognitionFingerprint;
use crate::tuning::{
    builtin_corpus, derive_reading_evidence, infer_candidate_correction_from_evidence,
    ReadingEvidence, SessionInferenceResult,
};
use crate::tuning_diagnostics::{SessionId, TuningStage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

pub const CHECKPOINT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingPass {
    First,
    Second,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingProgress {
    pub pass: ReadingPass,
    pub phrase_id: String,
    pub phrase_text: &'static str,
    pub position: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReadingEvidence {
    phrase_id: String,
    pass: ReadingPass,
    evidence: ReadingEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityEnvelope {
    recognition_fingerprint: RecognitionFingerprint,
    corpus_version: String,
    normalization_version: String,
    inference_version: String,
    verification_version: String,
    dictionary_matcher_version: String,
}

impl CompatibilityEnvelope {
    pub fn current(recognition_fingerprint: RecognitionFingerprint) -> Self {
        Self {
            recognition_fingerprint,
            corpus_version: crate::tuning::CORPUS_VERSION.into(),
            normalization_version: crate::tuning::NORMALIZATION_VERSION.into(),
            inference_version: crate::tuning::INFERENCE_VERSION.into(),
            verification_version: "tuning-verification-v1".into(),
            dictionary_matcher_version: "dictionary-matcher-v1".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuningCheckpoint {
    schema_version: u32,
    session_id: SessionId,
    envelope: CompatibilityEnvelope,
    stage: TuningStage,
    interrupted_attempt: bool,
    reading_queue: Vec<String>,
    reading_evidence: Vec<StoredReadingEvidence>,
    inference_results: Vec<SessionInferenceResult>,
}

impl TuningCheckpoint {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn stage(&self) -> TuningStage {
        self.stage
    }

    pub fn interrupted_attempt(&self) -> bool {
        self.interrupted_attempt
    }

    pub fn reading_progress(&self) -> Option<ReadingProgress> {
        let pass = match self.stage {
            TuningStage::FirstReading => ReadingPass::First,
            TuningStage::SecondReading => ReadingPass::Second,
            _ => return None,
        };
        let phrase_id = self.reading_queue.first()?.clone();
        let phrase = builtin_corpus().phrase(&phrase_id)?;
        let completed = self
            .reading_evidence
            .iter()
            .filter(|evidence| evidence.pass == pass)
            .count();
        Some(ReadingProgress {
            pass,
            phrase_id,
            phrase_text: phrase.text,
            position: completed + 1,
            total: builtin_corpus().phrases.len(),
        })
    }

    pub fn inference_results(&self) -> &[SessionInferenceResult] {
        &self.inference_results
    }

    pub fn candidate_count(&self) -> usize {
        self.inference_results
            .iter()
            .filter(|result| {
                matches!(result.decision, crate::tuning::InferenceDecision::Candidate(_))
            })
            .count()
    }
    fn with_interrupted_attempt(&self) -> Self {
        let mut candidate = self.clone();
        candidate.interrupted_attempt = true;
        candidate
    }

    fn with_practice_completed(&self) -> Self {
        let mut candidate = self.clone();
        candidate.stage = TuningStage::FirstReading;
        candidate.interrupted_attempt = false;
        candidate.reading_queue = builtin_corpus()
            .pass_a
            .iter()
            .map(|phrase_id| (*phrase_id).to_owned())
            .collect();
        candidate
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointState {
    None,
    Compatible(TuningCheckpoint),
    Incompatible { reason: String },
}

#[derive(Debug, Clone)]
pub struct TuningCheckpointStore {
    path: PathBuf,
}

impl TuningCheckpointStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    /// Read the one saved session without claiming compatibility. Entry uses
    /// this to display its last durable stage before preflight computes the
    /// current Recognition Fingerprint.
    pub fn load_saved(&self) -> Result<Option<TuningCheckpoint>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path)
            .map_err(|_| "Saved Tuning progress could not be read. Start over to continue.")?;
        let checkpoint = serde_json::from_slice::<TuningCheckpoint>(&bytes).map_err(|_| {
            "Saved Tuning progress has an unsupported format. Start over to continue."
        })?;
        if checkpoint.schema_version != CHECKPOINT_SCHEMA_VERSION {
            return Err(
                "The saved Tuning checkpoint version changed. Start over to continue.".into(),
            );
        }
        Ok(Some(checkpoint))
    }

    pub fn start(&self, envelope: CompatibilityEnvelope) -> Result<TuningCheckpoint, String> {
        let checkpoint = TuningCheckpoint {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            session_id: SessionId::new(),
            envelope,
            stage: TuningStage::Practice,
            interrupted_attempt: false,
            reading_queue: Vec::new(),
            reading_evidence: Vec::new(),
            inference_results: Vec::new(),
        };
        self.save(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn inspect(&self, current: &CompatibilityEnvelope) -> CheckpointState {
        let checkpoint = match self.load_saved() {
            Ok(None) => return CheckpointState::None,
            Ok(Some(checkpoint)) => checkpoint,
            Err(reason) => return CheckpointState::Incompatible { reason },
        };
        if checkpoint.envelope.recognition_fingerprint != current.recognition_fingerprint {
            return CheckpointState::Incompatible {
                reason: "The Recognition Fingerprint changed, so saved evidence cannot be reused. Start over to continue."
                    .into(),
            };
        }
        if checkpoint.envelope != *current {
            return CheckpointState::Incompatible {
                reason: "A Tuning behavior contract changed, so saved evidence cannot be reinterpreted. Start over to continue."
                    .into(),
            };
        }
        CheckpointState::Compatible(checkpoint)
    }

    /// Persist that an attempt began before microphone capture starts. On a
    /// later resume, the UI can truthfully say that the attempt must be repeated.
    pub fn begin_attempt(&self, checkpoint: &TuningCheckpoint) -> Result<TuningCheckpoint, String> {
        let candidate = checkpoint.with_interrupted_attempt();
        self.save(&candidate)?;
        Ok(candidate)
    }

    /// Practice becomes visible as complete only after this durable replacement succeeds.
    pub fn complete_practice(
        &self,
        checkpoint: &TuningCheckpoint,
    ) -> Result<TuningCheckpoint, String> {
        if checkpoint.stage != TuningStage::Practice {
            return Err("Practice is not the current durable Tuning stage".into());
        }
        let candidate = checkpoint.with_practice_completed();
        self.save(&candidate)?;
        Ok(candidate)
    }

    /// Move the current phrase to the end of this pass without changing its
    /// required-reading count or affecting the other pass.
    pub fn defer_current_phrase(
        &self,
        checkpoint: &TuningCheckpoint,
    ) -> Result<TuningCheckpoint, String> {
        if !matches!(
            checkpoint.stage,
            TuningStage::FirstReading | TuningStage::SecondReading
        ) || checkpoint.reading_queue.is_empty()
        {
            return Err("No current Tuning Phrase can be deferred".into());
        }
        let mut candidate = checkpoint.clone();
        let phrase_id = candidate.reading_queue.remove(0);
        candidate.reading_queue.push(phrase_id);
        candidate.interrupted_attempt = false;
        self.save(&candidate)?;
        Ok(candidate)
    }

    /// Discard only the in-flight attempt. Already completed reading evidence
    /// and the current pass queue remain unchanged.
    pub fn discard_current_attempt(
        &self,
        checkpoint: &TuningCheckpoint,
    ) -> Result<TuningCheckpoint, String> {
        if !matches!(
            checkpoint.stage,
            TuningStage::FirstReading | TuningStage::SecondReading
        ) {
            return Err("No Tuning Phrase attempt can be retried".into());
        }
        let mut candidate = checkpoint.clone();
        candidate.interrupted_attempt = false;
        self.save(&candidate)?;
        Ok(candidate)
    }

    /// Derive evidence from one successful local transcript, then atomically
    /// save that evidence and the resulting pass progress. The raw transcript
    /// is never placed in the checkpoint.
    pub fn complete_reading(
        &self,
        checkpoint: &TuningCheckpoint,
        raw_transcript: &str,
    ) -> Result<TuningCheckpoint, String> {
        let pass = match checkpoint.stage {
            TuningStage::FirstReading => ReadingPass::First,
            TuningStage::SecondReading => ReadingPass::Second,
            _ => return Err("A Tuning reading is not the current durable stage".into()),
        };
        let phrase_id = checkpoint
            .reading_queue
            .first()
            .ok_or_else(|| "The current reading pass has no pending phrase".to_string())?;
        let phrase = builtin_corpus()
            .phrase(phrase_id)
            .ok_or_else(|| "The current Tuning Phrase is not in the built-in corpus".to_string())?;
        let evidence = derive_reading_evidence(phrase, raw_transcript);

        let mut candidate = checkpoint.clone();
        candidate
            .reading_evidence
            .push(StoredReadingEvidence {
                phrase_id: phrase_id.clone(),
                pass,
                evidence,
            });
        candidate.reading_queue.remove(0);
        candidate.interrupted_attempt = false;

        if candidate.reading_queue.is_empty() {
            match pass {
                ReadingPass::First => {
                    candidate.stage = TuningStage::SecondReading;
                    candidate.reading_queue = builtin_corpus()
                        .pass_b
                        .iter()
                        .map(|phrase_id| (*phrase_id).to_owned())
                        .collect();
                }
                ReadingPass::Second => {
                    candidate.inference_results = builtin_corpus()
                        .phrases
                        .iter()
                        .map(|phrase| {
                            let first = find_evidence(&candidate, phrase.id, ReadingPass::First)?;
                            let second = find_evidence(&candidate, phrase.id, ReadingPass::Second)?;
                            Ok(infer_candidate_correction_from_evidence(
                                phrase.id,
                                &first.evidence,
                                &second.evidence,
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    candidate.reading_evidence.clear();
                    candidate.stage = TuningStage::Review;
                }
            }
        }

        self.save(&candidate)?;
        Ok(candidate)
    }

    fn save(&self, checkpoint: &TuningCheckpoint) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(checkpoint)
            .map_err(|error| format!("Serialize Tuning checkpoint failed: {error}"))?;
        atomic_replace(&self.path, &bytes)
    }
}

fn find_evidence<'a>(
    checkpoint: &'a TuningCheckpoint,
    phrase_id: &str,
    pass: ReadingPass,
) -> Result<&'a StoredReadingEvidence, String> {
    checkpoint
        .reading_evidence
        .iter()
        .find(|evidence| evidence.phrase_id == phrase_id && evidence.pass == pass)
        .ok_or_else(|| format!("Missing {pass:?} evidence for Tuning Phrase {phrase_id}"))
}

pub fn default_tuning_checkpoint_path() -> PathBuf {
    dirs::data_local_dir()
        .map(|dir| dir.join("eaglescribe").join("tuning-checkpoint.json"))
        .unwrap_or_else(|| PathBuf::from("tuning-checkpoint.json"))
}

fn atomic_replace(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("Create Tuning checkpoint directory failed: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Secure Tuning checkpoint directory failed: {error}"))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tuning-checkpoint.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("Create Tuning checkpoint temp file failed: {error}"))?;
        file.write_all(data)
            .map_err(|error| format!("Write Tuning checkpoint failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Sync Tuning checkpoint failed: {error}"))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("Commit Tuning checkpoint failed: {error}"))?;
        if let Ok(directory) = fs::File::open(parent) {
            directory
                .sync_all()
                .map_err(|error| format!("Sync Tuning checkpoint directory failed: {error}"))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("eaglescribe-tuning-session-{label}-{nanos}"))
            .join("checkpoint.json")
    }

    fn envelope(id: &str) -> CompatibilityEnvelope {
        CompatibilityEnvelope::current(RecognitionFingerprint::from_stable_id(id))
    }

    #[test]
    fn starting_a_session_atomically_persists_practice_as_the_durable_stage() {
        let path = temp_path("start");
        let store = TuningCheckpointStore::new(path.clone());

        let started = store.start(envelope("recognition-a")).unwrap();

        assert_eq!(started.stage, TuningStage::Practice);
        assert!(!started.interrupted_attempt);
        assert!(path.is_file());
        assert_eq!(
            store.inspect(&envelope("recognition-a")),
            CheckpointState::Compatible(started)
        );
    }

    #[test]
    fn recognition_or_behavior_contract_changes_require_explicit_start_over() {
        let path = temp_path("incompatible");
        let store = TuningCheckpointStore::new(path);
        store.start(envelope("recognition-a")).unwrap();

        let state = store.inspect(&envelope("recognition-b"));

        assert!(matches!(state, CheckpointState::Incompatible { .. }));
        let CheckpointState::Incompatible { reason } = state else {
            unreachable!()
        };
        assert!(reason.contains("Recognition Fingerprint"));
        assert!(reason.contains("Start over"));

        let mut changed_contract = envelope("recognition-a");
        changed_contract.corpus_version = "tuning-corpus-v2".into();
        let state = store.inspect(&changed_contract);
        let CheckpointState::Incompatible { reason } = state else {
            panic!("changed behavior contract must be incompatible")
        };
        assert!(reason.contains("behavior contract"));
        assert!(reason.contains("Start over"));
    }

    #[test]
    fn an_interrupted_practice_resumes_at_practice_and_requires_a_repeat() {
        let path = temp_path("interrupted");
        let store = TuningCheckpointStore::new(path);
        let started = store.start(envelope("recognition-a")).unwrap();

        let interrupted = store.begin_attempt(&started).unwrap();
        let reloaded = store.inspect(&envelope("recognition-a"));

        assert!(interrupted.interrupted_attempt());
        assert_eq!(interrupted.stage(), TuningStage::Practice);
        assert_eq!(reloaded, CheckpointState::Compatible(interrupted));
    }

    #[test]
    fn practice_advances_only_after_the_checkpoint_save_succeeds() {
        let path = temp_path("practice-save");
        let store = TuningCheckpointStore::new(path);
        let started = store.start(envelope("recognition-a")).unwrap();
        let interrupted = store.begin_attempt(&started).unwrap();

        let completed = store.complete_practice(&interrupted).unwrap();

        assert_eq!(completed.stage(), TuningStage::FirstReading);
        assert!(!completed.interrupted_attempt());
        assert_eq!(
            store.inspect(&envelope("recognition-a")),
            CheckpointState::Compatible(completed)
        );
    }

    #[test]
    fn failed_practice_save_cannot_mutate_the_acknowledged_checkpoint() {
        let path = temp_path("practice-save-failure");
        let store = TuningCheckpointStore::new(path.clone());
        let started = store.start(envelope("recognition-a")).unwrap();
        let interrupted = store.begin_attempt(&started).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        assert!(store.complete_practice(&interrupted).is_err());
        assert_eq!(interrupted.stage(), TuningStage::Practice);
        assert!(interrupted.interrupted_attempt());
    }

    #[test]
    fn reading_passes_follow_the_exact_orders_and_defer_only_within_the_current_pass() {
        let path = temp_path("reading-orders");
        let store = TuningCheckpointStore::new(path);
        let started = store.start(envelope("recognition-a")).unwrap();
        let mut checkpoint = store.complete_practice(&started).unwrap();

        assert_eq!(checkpoint.reading_progress().unwrap().phrase_id, "T01");
        checkpoint = store.defer_current_phrase(&checkpoint).unwrap();
        assert_eq!(checkpoint.reading_progress().unwrap().phrase_id, "T02");

        for expected_id in ["T02", "T03", "T04", "T05", "T06", "T07", "T08", "T09", "T10", "T01"] {
            let progress = checkpoint.reading_progress().unwrap();
            assert_eq!(progress.phrase_id, expected_id);
            checkpoint = store
                .complete_reading(&checkpoint, builtin_corpus().phrase(expected_id).unwrap().text)
                .unwrap();
        }

        assert_eq!(checkpoint.stage(), TuningStage::SecondReading);
        assert_eq!(checkpoint.reading_progress().unwrap().phrase_id, "T06");
        for expected_id in ["T06", "T07", "T08", "T09", "T10", "T01", "T02", "T03", "T04", "T05"] {
            let progress = checkpoint.reading_progress().unwrap();
            assert_eq!(progress.phrase_id, expected_id);
            checkpoint = store
                .complete_reading(&checkpoint, builtin_corpus().phrase(expected_id).unwrap().text)
                .unwrap();
        }

        assert_eq!(checkpoint.stage(), TuningStage::Review);
        assert!(checkpoint.reading_progress().is_none());
        assert_eq!(checkpoint.inference_results().len(), 10);
    }

    #[test]
    fn a_valid_reading_saves_only_derived_evidence_and_progress_in_one_checkpoint() {
        let path = temp_path("derived-only");
        let store = TuningCheckpointStore::new(path.clone());
        let started = store.start(envelope("recognition-a")).unwrap();
        let checkpoint = store.complete_practice(&started).unwrap();
        let transcript = "That quick chip carries heavy blue boxes";

        let completed = store.complete_reading(&checkpoint, transcript).unwrap();

        assert_eq!(completed.reading_progress().unwrap().phrase_id, "T02");
        let persisted = fs::read_to_string(path).unwrap();
        assert!(!persisted.contains(transcript));
        assert!(persisted.contains("quick chip"));
        assert!(persisted.contains("quick ship"));
    }

    #[test]
    fn rejected_reading_persists_only_its_reason_code_not_mismatch_content_or_locations() {
        let path = temp_path("rejected-minimum");
        let store = TuningCheckpointStore::new(path.clone());
        let started = store.start(envelope("recognition-a")).unwrap();
        let checkpoint = store.complete_practice(&started).unwrap();
        let checkpoint = store.defer_current_phrase(&checkpoint).unwrap();

        store
            .complete_reading(
                &checkpoint,
                "Your boys made the joyful choice sound easy",
            )
            .unwrap();

        let persisted = fs::read_to_string(path).unwrap();
        assert!(persisted.contains("single_word_source"));
        assert!(!persisted.contains("boys"));
        assert!(!persisted.contains("expected_range"));
        assert!(!persisted.contains("observed_range"));
    }

    #[test]
    fn retry_discards_no_completed_reading_and_failed_save_cannot_advance_progress() {
        let path = temp_path("retry-and-failure");
        let store = TuningCheckpointStore::new(path.clone());
        let started = store.start(envelope("recognition-a")).unwrap();
        let checkpoint = store.complete_practice(&started).unwrap();
        let interrupted = store.begin_attempt(&checkpoint).unwrap();

        let retried = store.discard_current_attempt(&interrupted).unwrap();
        assert!(!retried.interrupted_attempt());
        assert_eq!(retried.reading_progress().unwrap().phrase_id, "T01");

        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store
            .complete_reading(&retried, "That quick ship carries heavy blue boxes")
            .is_err());
        assert_eq!(retried.reading_progress().unwrap().phrase_id, "T01");
    }

    #[test]
    fn completing_both_readings_produces_at_most_one_inactive_candidate_per_phrase() {
        let path = temp_path("inference");
        let store = TuningCheckpointStore::new(path);
        let started = store.start(envelope("recognition-a")).unwrap();
        let mut checkpoint = store.complete_practice(&started).unwrap();

        for phrase_id in builtin_corpus().pass_a {
            let transcript = if *phrase_id == "T01" {
                "That quick chip carries heavy blue boxes"
            } else {
                builtin_corpus().phrase(phrase_id).unwrap().text
            };
            checkpoint = store.complete_reading(&checkpoint, transcript).unwrap();
        }
        for phrase_id in builtin_corpus().pass_b {
            let transcript = if *phrase_id == "T01" {
                "That quick chip carries heavy blue boxes"
            } else {
                builtin_corpus().phrase(phrase_id).unwrap().text
            };
            checkpoint = store.complete_reading(&checkpoint, transcript).unwrap();
        }

        assert_eq!(checkpoint.stage(), TuningStage::Review);
        assert_eq!(checkpoint.inference_results().len(), 10);
        let candidates: Vec<_> = checkpoint
            .inference_results()
            .iter()
            .filter_map(|result| match &result.decision {
                crate::tuning::InferenceDecision::Candidate(candidate) => Some(candidate),
                crate::tuning::InferenceDecision::Rejected => None,
            })
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].from, "quick chip");
        assert_eq!(candidates[0].to, "quick ship");
        assert_eq!(candidates[0].state, crate::tuning::CandidateState::Inactive);
    }
}
