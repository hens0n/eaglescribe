//! Durable state for the guided Tuning Session through its required Practice stage.

use crate::recognition::RecognitionFingerprint;
use crate::tuning_diagnostics::{SessionId, TuningStage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

pub const CHECKPOINT_SCHEMA_VERSION: u32 = 1;

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
    fn with_interrupted_attempt(&self) -> Self {
        let mut candidate = self.clone();
        candidate.interrupted_attempt = true;
        candidate
    }

    fn with_practice_completed(&self) -> Self {
        let mut candidate = self.clone();
        candidate.stage = TuningStage::FirstReading;
        candidate.interrupted_attempt = false;
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

    fn save(&self, checkpoint: &TuningCheckpoint) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(checkpoint)
            .map_err(|error| format!("Serialize Tuning checkpoint failed: {error}"))?;
        atomic_replace(&self.path, &bytes)
    }
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
}
