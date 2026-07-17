//! Content-free, local-only diagnostics for guided Tuning.
//!
//! This ledger is deliberately non-authoritative. Callers must commit Tuning
//! checkpoints and dictionary changes independently, and must never make a
//! Tuning outcome depend on whether this module can read or write its file.

use crate::tuning::builtin_corpus;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;
pub const EXPORT_SCHEMA_VERSION: u32 = 1;
pub const TERMINAL_RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
pub const MAX_TERMINAL_SESSIONS: usize = 20;
const DURATION_SAMPLE_MINIMUM: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsNotice {
    CorruptStoreDiscarded,
    StorageUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsError {
    InvalidEvent,
    InvalidExportMetadata,
    CreateDirectoryFailed,
    SerializeFailed,
    CreateTemporaryFailed,
    WriteFailed,
    SyncFailed,
    ReplaceFailed,
    SyncDirectoryFailed,
}

impl std::fmt::Display for DiagnosticsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = serde_json::to_string(self).unwrap_or_else(|_| "\"storage_unavailable\"".into());
        f.write_str(code.trim_matches('"'))
    }
}

impl std::error::Error for DiagnosticsError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, DiagnosticsError> {
        validate_uuid(value)
            .then(|| Self(value.to_owned()))
            .ok_or(DiagnosticsError::InvalidEvent)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(String);

impl RuleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, DiagnosticsError> {
        validate_uuid(value)
            .then(|| Self(value.to_owned()))
            .ok_or(DiagnosticsError::InvalidEvent)
    }
}

impl Default for RuleId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentFingerprint(String);

impl ContentFingerprint {
    pub fn from_sha256_hex(value: &str) -> Result<Self, DiagnosticsError> {
        is_sha256_hex(value)
            .then(|| Self(value.to_ascii_lowercase()))
            .ok_or(DiagnosticsError::InvalidEvent)
    }

    fn is_valid(&self) -> bool {
        is_sha256_hex(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningStage {
    Ready,
    Practice,
    FirstReading,
    SecondReading,
    Review,
    Verify,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionCreated,
    StageCompleted,
    SessionPaused,
    SessionResumed,
    SessionCancelled,
    SessionRestarted,
    CheckpointRecovered,
    SessionCompleted,
    PhraseAttempt,
    CandidateDecision,
    VerificationAttempt,
    VerificationRetryExhausted,
    RuleRollback,
    DictionaryCommit,
    StorageFailure,
}

impl EventKind {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::SessionCancelled | Self::SessionRestarted | Self::SessionCompleted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeCode {
    Started,
    Completed,
    Paused,
    Resumed,
    Cancelled,
    Restarted,
    Recovered,
    Valid,
    Discarded,
    Interrupted,
    Skipped,
    Deferred,
    OperationalFailure,
    Proposed,
    Rejected,
    Approved,
    Declined,
    KeepExisting,
    VerifyReplacement,
    StaleRuleReturned,
    Successful,
    Failed,
    NotExercised,
    Retry,
    RolledBack,
    Committed,
    PartialSuccess,
    NoSafeCorrections,
    AlreadyCovered,
    NoneApproved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    NoMismatch,
    ReadingsDisagree,
    MissingContext,
    MultipleHunks,
    InsertionOrDeletion,
    SpanMappingFailed,
    OutsideEligibleSpan,
    SingleWordSource,
    CouldNotVerify,
    TargetNotCorrected,
    HarmfulChange,
    RuleInteraction,
    MicrophoneUnavailable,
    ModelLoadFailed,
    TranscriptionFailed,
    CheckpointWriteFailed,
    CheckpointIncompatible,
    DiagnosticWriteFailed,
}

impl ReasonCode {
    /// Canonical neutral copy for Review, Result, and local diagnostics views.
    pub fn user_facing_meaning(self) -> &'static str {
        match self {
            Self::NoMismatch => "No correction needed",
            Self::ReadingsDisagree => "Did not repeat consistently",
            Self::MissingContext | Self::MultipleHunks => {
                "Recognition differed in more than one clear way"
            }
            Self::InsertionOrDeletion | Self::SpanMappingFailed => {
                "Could not form a complete phrase replacement"
            }
            Self::OutsideEligibleSpan | Self::SingleWordSource => "Too broad to apply safely",
            Self::CouldNotVerify => "Could not reproduce the recognized phrase",
            Self::TargetNotCorrected => "Did not correct the intended phrase",
            Self::HarmfulChange => "Changed other text",
            Self::RuleInteraction => "Interacted with another rule",
            Self::MicrophoneUnavailable => "Microphone unavailable",
            Self::ModelLoadFailed => "Model could not be loaded",
            Self::TranscriptionFailed => "Transcription could not be completed",
            Self::CheckpointWriteFailed => "Tuning progress could not be saved",
            Self::CheckpointIncompatible => "Saved Tuning progress is incompatible",
            Self::DiagnosticWriteFailed => "Diagnostics unavailable",
        }
    }
}

impl From<crate::tuning::ReasonCode> for ReasonCode {
    fn from(reason: crate::tuning::ReasonCode) -> Self {
        match reason {
            crate::tuning::ReasonCode::NoMismatch => Self::NoMismatch,
            crate::tuning::ReasonCode::ReadingsDisagree => Self::ReadingsDisagree,
            crate::tuning::ReasonCode::MissingContext => Self::MissingContext,
            crate::tuning::ReasonCode::MultipleHunks => Self::MultipleHunks,
            crate::tuning::ReasonCode::InsertionOrDeletion => Self::InsertionOrDeletion,
            crate::tuning::ReasonCode::SpanMappingFailed => Self::SpanMappingFailed,
            crate::tuning::ReasonCode::OutsideEligibleSpan => Self::OutsideEligibleSpan,
            crate::tuning::ReasonCode::SingleWordSource => Self::SingleWordSource,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountKind {
    RestoredPhrases,
    RestoredApprovals,
    SupportingPhrases,
    ParticipatingRules,
    Attempts,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticFingerprints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<ContentFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ContentFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<ContentFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<ContentFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuningDiagnosticEvent {
    schema_version: u32,
    event_id: String,
    timestamp_ms: u64,
    session_id: SessionId,
    kind: EventKind,
    stage: TuningStage,
    outcome: OutcomeCode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reason_codes: Vec<ReasonCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phrase_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probe_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    valid_attempt_ordinal: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rule_ids: Vec<RuleId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    participating_rule_ids: Vec<RuleId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    counts: BTreeMap<CountKind, u64>,
    #[serde(default, skip_serializing_if = "fingerprints_are_empty")]
    fingerprints: DiagnosticFingerprints,
    #[serde(default, skip_serializing_if = "is_false")]
    session_record_partial: bool,
}

impl TuningDiagnosticEvent {
    pub fn new(
        timestamp_ms: u64,
        session_id: SessionId,
        kind: EventKind,
        stage: TuningStage,
        outcome: OutcomeCode,
    ) -> Self {
        Self {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            event_id: Uuid::new_v4().to_string(),
            timestamp_ms,
            session_id,
            kind,
            stage,
            outcome,
            reason_codes: Vec::new(),
            phrase_id: None,
            probe_id: None,
            valid_attempt_ordinal: None,
            rule_ids: Vec::new(),
            participating_rule_ids: Vec::new(),
            duration_ms: None,
            counts: BTreeMap::new(),
            fingerprints: DiagnosticFingerprints::default(),
            session_record_partial: false,
        }
    }

    pub fn with_reason(mut self, reason: ReasonCode) -> Self {
        if !self.reason_codes.contains(&reason) {
            self.reason_codes.push(reason);
        }
        self
    }

    pub fn with_phrase(mut self, phrase_id: &str) -> Result<Self, DiagnosticsError> {
        if builtin_corpus().phrase(phrase_id).is_none() {
            return Err(DiagnosticsError::InvalidEvent);
        }
        self.phrase_id = Some(phrase_id.to_owned());
        Ok(self)
    }

    pub fn with_probe(mut self, probe_id: &str) -> Result<Self, DiagnosticsError> {
        if !is_builtin_probe(probe_id) {
            return Err(DiagnosticsError::InvalidEvent);
        }
        self.probe_id = Some(probe_id.to_owned());
        Ok(self)
    }

    pub fn with_attempt_ordinal(mut self, ordinal: u16) -> Result<Self, DiagnosticsError> {
        if ordinal == 0 {
            return Err(DiagnosticsError::InvalidEvent);
        }
        self.valid_attempt_ordinal = Some(ordinal);
        Ok(self)
    }

    pub fn with_rule(mut self, rule_id: RuleId) -> Self {
        if !self.rule_ids.contains(&rule_id) {
            self.rule_ids.push(rule_id);
        }
        self
    }

    pub fn with_participating_rule(mut self, rule_id: RuleId) -> Self {
        if !self.participating_rule_ids.contains(&rule_id) {
            self.participating_rule_ids.push(rule_id);
        }
        self
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    pub fn with_count(mut self, kind: CountKind, count: u64) -> Self {
        self.counts.insert(kind, count);
        self
    }

    pub fn with_fingerprints(mut self, fingerprints: DiagnosticFingerprints) -> Self {
        self.fingerprints = fingerprints;
        self
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    fn validate(&self) -> bool {
        self.schema_version == DIAGNOSTIC_SCHEMA_VERSION
            && validate_uuid(&self.event_id)
            && validate_uuid(self.session_id.as_str())
            && self.valid_attempt_ordinal != Some(0)
            && self
                .phrase_id
                .as_deref()
                .is_none_or(|id| builtin_corpus().phrase(id).is_some())
            && self.probe_id.as_deref().is_none_or(is_builtin_probe)
            && self.rule_ids.iter().all(|id| validate_uuid(&id.0))
            && self
                .participating_rule_ids
                .iter()
                .all(|id| validate_uuid(&id.0))
            && fingerprints_are_valid(&self.fingerprints)
            && self.reason_codes.iter().collect::<HashSet<_>>().len() == self.reason_codes.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticLedger {
    schema_version: u32,
    events: Vec<TuningDiagnosticEvent>,
}

impl Default for DiagnosticLedger {
    fn default() -> Self {
        Self {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            events: Vec::new(),
        }
    }
}

impl DiagnosticLedger {
    fn validate(&self) -> bool {
        if self.schema_version != DIAGNOSTIC_SCHEMA_VERSION
            || self.events.iter().any(|e| !e.validate())
        {
            return false;
        }
        let mut ids = HashSet::new();
        self.events
            .iter()
            .all(|event| ids.insert(event.event_id.as_str()))
    }
}

pub struct TuningDiagnosticsStore {
    path: PathBuf,
    ledger: DiagnosticLedger,
    partial_sessions: HashSet<SessionId>,
    period_partial: bool,
}

impl TuningDiagnosticsStore {
    /// Load and prune without ever making diagnostics a startup dependency.
    pub fn open(path: PathBuf, now_ms: u64) -> (Self, Option<DiagnosticsNotice>) {
        if !path.is_file() {
            return (Self::empty(path), None);
        }
        let parsed = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<DiagnosticLedger>(&bytes).ok())
            .filter(DiagnosticLedger::validate);
        let Some(mut ledger) = parsed else {
            let mut store = Self::empty(path);
            store.period_partial = true;
            return (store, Some(DiagnosticsNotice::CorruptStoreDiscarded));
        };
        let original_len = ledger.events.len();
        // The store writes no file on first use. A valid empty file therefore
        // means retained diagnostics were explicitly cleared; keep summaries
        // honest across a restart until a subsequent event carries the marker.
        let starts_after_clear = ledger.events.is_empty();
        prune_ledger(&mut ledger, now_ms);
        let partial_sessions = ledger
            .events
            .iter()
            .filter(|e| e.session_record_partial)
            .map(|e| e.session_id.clone())
            .collect();
        let mut store = Self {
            path,
            ledger,
            partial_sessions,
            period_partial: starts_after_clear,
        };
        let notice = if store.ledger.events.len() != original_len && store.persist().is_err() {
            Some(DiagnosticsNotice::StorageUnavailable)
        } else {
            None
        };
        (store, notice)
    }

    fn empty(path: PathBuf) -> Self {
        Self {
            path,
            ledger: DiagnosticLedger::default(),
            partial_sessions: HashSet::new(),
            period_partial: false,
        }
    }

    pub fn append(
        &mut self,
        mut event: TuningDiagnosticEvent,
        now_ms: u64,
    ) -> Result<(), DiagnosticsError> {
        if !event.validate() {
            return Err(DiagnosticsError::InvalidEvent);
        }
        if self.period_partial || self.partial_sessions.contains(&event.session_id) {
            event.session_record_partial = true;
            self.partial_sessions.insert(event.session_id.clone());
        }
        let mut candidate = self.ledger.clone();
        candidate.events.push(event);
        prune_ledger(&mut candidate, now_ms);
        persist_ledger(&self.path, &candidate, true)?;
        self.ledger = candidate;
        self.period_partial = false;
        Ok(())
    }

    /// Clear only this ledger. The active session is remembered in memory so
    /// subsequent events make the retained period's incompleteness explicit.
    pub fn clear(&mut self, active_session: Option<&SessionId>) -> Result<(), DiagnosticsError> {
        let candidate = DiagnosticLedger::default();
        persist_ledger(&self.path, &candidate, true)?;
        self.ledger = candidate;
        self.partial_sessions.clear();
        if let Some(session) = active_session {
            self.partial_sessions.insert(session.clone());
        }
        // Even without an unfinished session, the retained 30-day view starts
        // after an explicit deletion and is therefore incomplete.
        self.period_partial = true;
        Ok(())
    }

    pub fn retained_events(&self) -> &[TuningDiagnosticEvent] {
        &self.ledger.events
    }

    pub fn health_summary(&self) -> TuningHealthSummary {
        summarize(
            &self.ledger.events,
            self.period_partial || !self.partial_sessions.is_empty(),
        )
    }

    pub fn export_preview(&self, selection: ExportSelection) -> ExportPreview {
        ExportPreview {
            categories: selection.categories(),
            retained_event_count: selection.events.then_some(self.ledger.events.len() as u64),
            summary_partial: selection.summary.then_some(self.health_summary().partial),
        }
    }

    pub fn export_to(
        &self,
        destination: &Path,
        selection: ExportSelection,
        environment: ExportEnvironment,
    ) -> Result<(), DiagnosticsError> {
        environment.validate()?;
        let bundle = DiagnosticExportBundle {
            export_schema_version: EXPORT_SCHEMA_VERSION,
            diagnostic_schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            events: selection.events.then(|| self.ledger.events.clone()),
            summary: selection.summary.then(|| self.health_summary()),
            environment: selection.environment.then_some(environment),
        };
        let bytes =
            serde_json::to_vec_pretty(&bundle).map_err(|_| DiagnosticsError::SerializeFailed)?;
        atomic_replace(destination, &bytes, false)
    }

    fn persist(&mut self) -> Result<(), DiagnosticsError> {
        persist_ledger(&self.path, &self.ledger, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DurationStatistics {
    pub samples: u64,
    pub median_ms: u64,
    pub p90_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TuningHealthSummary {
    pub diagnostic_schema_version: u32,
    pub retained_event_count: u64,
    pub retained_session_count: u64,
    pub unfinished_session_count: u64,
    pub terminal_session_count: u64,
    pub partial: bool,
    pub event_counts: BTreeMap<EventKind, u64>,
    pub outcome_counts: BTreeMap<OutcomeCode, u64>,
    pub reason_counts: BTreeMap<ReasonCode, u64>,
    pub stage_durations: BTreeMap<TuningStage, DurationStatistics>,
}

fn summarize(events: &[TuningDiagnosticEvent], externally_partial: bool) -> TuningHealthSummary {
    let mut sessions: HashMap<&SessionId, bool> = HashMap::new();
    let mut event_counts = BTreeMap::new();
    let mut outcome_counts = BTreeMap::new();
    let mut reason_counts = BTreeMap::new();
    let mut durations: BTreeMap<TuningStage, Vec<u64>> = BTreeMap::new();
    for event in events {
        sessions
            .entry(&event.session_id)
            .and_modify(|terminal| *terminal |= event.kind.is_terminal())
            .or_insert(event.kind.is_terminal());
        *event_counts.entry(event.kind).or_insert(0) += 1;
        *outcome_counts.entry(event.outcome).or_insert(0) += 1;
        for reason in &event.reason_codes {
            *reason_counts.entry(*reason).or_insert(0) += 1;
        }
        if let Some(duration) = event.duration_ms {
            durations.entry(event.stage).or_default().push(duration);
        }
    }
    let stage_durations = durations
        .into_iter()
        .filter_map(|(stage, mut values)| {
            if values.len() < DURATION_SAMPLE_MINIMUM {
                return None;
            }
            values.sort_unstable();
            let midpoint = values.len() / 2;
            let median_ms = if values.len() % 2 == 0 {
                values[midpoint - 1] / 2
                    + values[midpoint] / 2
                    + (values[midpoint - 1] % 2 + values[midpoint] % 2) / 2
            } else {
                values[midpoint]
            };
            let p90_index = (values.len() * 9).div_ceil(10).saturating_sub(1);
            Some((
                stage,
                DurationStatistics {
                    samples: values.len() as u64,
                    median_ms,
                    p90_ms: values[p90_index],
                },
            ))
        })
        .collect();
    let terminal_session_count = sessions.values().filter(|terminal| **terminal).count() as u64;
    TuningHealthSummary {
        diagnostic_schema_version: DIAGNOSTIC_SCHEMA_VERSION,
        retained_event_count: events.len() as u64,
        retained_session_count: sessions.len() as u64,
        unfinished_session_count: sessions.len() as u64 - terminal_session_count,
        terminal_session_count,
        partial: externally_partial || events.iter().any(|e| e.session_record_partial),
        event_counts,
        outcome_counts,
        reason_counts,
        stage_durations,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSelection {
    pub events: bool,
    pub summary: bool,
    pub environment: bool,
}

impl ExportSelection {
    pub fn all() -> Self {
        Self {
            events: true,
            summary: true,
            environment: true,
        }
    }

    fn categories(self) -> Vec<ExportCategory> {
        let mut categories = Vec::new();
        if self.events {
            categories.push(ExportCategory::RetainedEvents);
        }
        if self.summary {
            categories.push(ExportCategory::HealthSummary);
        }
        if self.environment {
            categories.push(ExportCategory::Environment);
        }
        categories
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportCategory {
    RetainedEvents,
    HealthSummary,
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPreview {
    pub categories: Vec<ExportCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained_event_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_partial: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformCode {
    Macos,
    Linux,
    Windows,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendCode {
    Cpu,
    Metal,
    Cuda,
    Vulkan,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportEnvironment {
    pub app_version: String,
    pub platform: PlatformCode,
    pub backend: BackendCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_fingerprint: Option<ContentFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_fingerprint: Option<ContentFingerprint>,
}

impl ExportEnvironment {
    fn validate(&self) -> Result<(), DiagnosticsError> {
        let valid_version = !self.app_version.is_empty()
            && self.app_version.len() <= 64
            && self
                .app_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'));
        let valid_fingerprints = self
            .model_fingerprint
            .as_ref()
            .is_none_or(ContentFingerprint::is_valid)
            && self
                .configuration_fingerprint
                .as_ref()
                .is_none_or(ContentFingerprint::is_valid);
        (valid_version && valid_fingerprints)
            .then_some(())
            .ok_or(DiagnosticsError::InvalidExportMetadata)
    }
}

#[derive(Serialize)]
struct DiagnosticExportBundle {
    export_schema_version: u32,
    diagnostic_schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    events: Option<Vec<TuningDiagnosticEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<TuningHealthSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<ExportEnvironment>,
}

fn prune_ledger(ledger: &mut DiagnosticLedger, now_ms: u64) {
    let mut terminal_times: HashMap<SessionId, u64> = HashMap::new();
    for event in &ledger.events {
        if event.kind.is_terminal() {
            terminal_times
                .entry(event.session_id.clone())
                .and_modify(|at| *at = (*at).max(event.timestamp_ms))
                .or_insert(event.timestamp_ms);
        }
    }
    let terminal_session_ids: HashSet<_> = terminal_times.keys().cloned().collect();
    let mut terminal: Vec<_> = terminal_times.into_iter().collect();
    terminal.sort_by(|(left_id, left_at), (right_id, right_at)| {
        right_at
            .cmp(left_at)
            .then_with(|| right_id.as_str().cmp(left_id.as_str()))
    });
    let retained: HashSet<_> = terminal
        .into_iter()
        .take(MAX_TERMINAL_SESSIONS)
        .filter(|(_, at)| now_ms.saturating_sub(*at) <= TERMINAL_RETENTION_MS)
        .map(|(id, _)| id)
        .collect();
    ledger.events.retain(|event| {
        !terminal_session_ids.contains(&event.session_id) || retained.contains(&event.session_id)
    });
}

fn persist_ledger(
    path: &Path,
    ledger: &DiagnosticLedger,
    private_parent: bool,
) -> Result<(), DiagnosticsError> {
    let bytes = serde_json::to_vec_pretty(ledger).map_err(|_| DiagnosticsError::SerializeFailed)?;
    atomic_replace(path, &bytes, private_parent)
}

fn atomic_replace(path: &Path, data: &[u8], private_parent: bool) -> Result<(), DiagnosticsError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| DiagnosticsError::CreateDirectoryFailed)?;
    #[cfg(unix)]
    if private_parent {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| DiagnosticsError::CreateDirectoryFailed)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tuning-diagnostics.json");
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
            .map_err(|_| DiagnosticsError::CreateTemporaryFailed)?;
        file.write_all(data)
            .map_err(|_| DiagnosticsError::WriteFailed)?;
        file.sync_all().map_err(|_| DiagnosticsError::SyncFailed)?;
        fs::rename(&temporary, path).map_err(|_| DiagnosticsError::ReplaceFailed)?;
        if let Ok(directory) = fs::File::open(parent) {
            directory
                .sync_all()
                .map_err(|_| DiagnosticsError::SyncDirectoryFailed)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_builtin_probe(probe_id: &str) -> bool {
    builtin_corpus()
        .phrases
        .iter()
        .flat_map(|phrase| phrase.eligible_spans)
        .any(|span| span.id == probe_id)
}

fn fingerprints_are_empty(value: &DiagnosticFingerprints) -> bool {
    value == &DiagnosticFingerprints::default()
}

fn fingerprints_are_valid(value: &DiagnosticFingerprints) -> bool {
    [
        &value.app,
        &value.model,
        &value.backend,
        &value.configuration,
    ]
    .into_iter()
    .all(|fingerprint| {
        fingerprint
            .as_ref()
            .is_none_or(ContentFingerprint::is_valid)
    })
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn default_tuning_diagnostics_path() -> PathBuf {
    dirs::data_local_dir()
        .map(|dir| dir.join("eaglescribe").join("tuning-diagnostics.json"))
        .unwrap_or_else(|| PathBuf::from("tuning-diagnostics.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("eaglescribe-tuning-diagnostics-{label}-{nanos}"))
    }

    fn event(at: u64, session: &SessionId, kind: EventKind) -> TuningDiagnosticEvent {
        TuningDiagnosticEvent::new(
            at,
            session.clone(),
            kind,
            TuningStage::Result,
            OutcomeCode::Completed,
        )
    }

    #[test]
    fn envelope_rejects_content_bearing_or_unknown_identifiers() {
        assert!(SessionId::parse("my spoken transcript").is_err());
        assert!(RuleId::parse("secret mapping: quick chip -> quick ship").is_err());
        assert!(ContentFingerprint::from_sha256_hex("/Users/person/model.bin").is_err());
        let session = SessionId::new();
        assert!(event(1, &session, EventKind::PhraseAttempt)
            .with_phrase("spoken words")
            .is_err());
        assert!(event(1, &session, EventKind::PhraseAttempt)
            .with_probe("quick ship")
            .is_err());
        assert_eq!(
            ReasonCode::MissingContext.user_facing_meaning(),
            "Recognition differed in more than one clear way"
        );
    }

    #[test]
    fn deserialized_content_cannot_bypass_content_boundary() {
        let dir = temp_dir("untrusted-json");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diagnostics.json");
        let session = SessionId::new();
        let mut value = serde_json::to_value(DiagnosticLedger {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            events: vec![event(1, &session, EventKind::SessionCreated)],
        })
        .unwrap();
        value["events"][0]["transcript"] =
            serde_json::json!("transcript sentinel in an unknown field");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (store, notice) = TuningDiagnosticsStore::open(path.clone(), 1);
        assert_eq!(notice, Some(DiagnosticsNotice::CorruptStoreDiscarded));
        assert!(store.retained_events().is_empty());
        value["events"][0]
            .as_object_mut()
            .unwrap()
            .remove("transcript");
        value["events"][0]["fingerprints"] = serde_json::json!({
            "model": "transcript sentinel that is not a fingerprint"
        });
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (store, notice) = TuningDiagnosticsStore::open(path, 1);
        assert_eq!(notice, Some(DiagnosticsNotice::CorruptStoreDiscarded));
        assert!(store.retained_events().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn store_is_structured_separate_atomic_and_private() {
        let dir = temp_dir("private");
        let path = dir.join("tuning-diagnostics.json");
        let (mut store, notice) = TuningDiagnosticsStore::open(path.clone(), 1);
        assert_eq!(notice, None);
        let session = SessionId::new();
        store
            .append(event(1, &session, EventKind::SessionCreated), 1)
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["schema_version"], DIAGNOSTIC_SCHEMA_VERSION);
        assert!(json["events"][0].get("text").is_none());
        assert!(!dir.join("history.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn retention_keeps_unfinished_but_bounds_terminal_sessions_by_age_and_count() {
        let dir = temp_dir("retention");
        let path = dir.join("diagnostics.json");
        let now = TERMINAL_RETENTION_MS + 10_000;
        let (mut store, _) = TuningDiagnosticsStore::open(path, now);
        let unfinished = SessionId::new();
        store
            .append(event(1, &unfinished, EventKind::SessionCreated), now)
            .unwrap();
        let old = SessionId::new();
        store
            .append(event(1, &old, EventKind::SessionCompleted), now)
            .unwrap();
        for index in 0..=MAX_TERMINAL_SESSIONS {
            let session = SessionId::new();
            store
                .append(
                    event(now + index as u64, &session, EventKind::SessionCompleted),
                    now + MAX_TERMINAL_SESSIONS as u64,
                )
                .unwrap();
        }
        let sessions: HashSet<_> = store
            .retained_events()
            .iter()
            .map(|event| event.session_id().as_str())
            .collect();
        assert!(sessions.contains(unfinished.as_str()));
        assert!(!sessions.contains(old.as_str()));
        assert_eq!(
            store.health_summary().terminal_session_count,
            MAX_TERMINAL_SESSIONS as u64
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_does_not_touch_authoritative_files_and_marks_new_period_partial() {
        let dir = temp_dir("clear");
        fs::create_dir_all(&dir).unwrap();
        let checkpoint = dir.join("checkpoint.json");
        let dictionary = dir.join("dictionary.json");
        fs::write(&checkpoint, b"checkpoint sentinel").unwrap();
        fs::write(&dictionary, b"dictionary sentinel").unwrap();
        let (mut store, _) = TuningDiagnosticsStore::open(dir.join("diagnostics.json"), 1);
        let session = SessionId::new();
        store
            .append(event(1, &session, EventKind::SessionCreated), 1)
            .unwrap();
        store.clear(Some(&session)).unwrap();
        assert!(store.retained_events().is_empty());
        assert!(store.health_summary().partial);
        drop(store);
        let (mut store, notice) = TuningDiagnosticsStore::open(dir.join("diagnostics.json"), 2);
        assert_eq!(notice, None);
        assert!(store.health_summary().partial);
        store
            .append(event(2, &session, EventKind::SessionResumed), 2)
            .unwrap();
        assert!(store.health_summary().partial);
        assert_eq!(fs::read(checkpoint).unwrap(), b"checkpoint sentinel");
        assert_eq!(fs::read(dictionary).unwrap(), b"dictionary sentinel");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn summary_is_recomputed_only_from_retained_events() {
        let dir = temp_dir("summary");
        let (mut store, _) = TuningDiagnosticsStore::open(dir.join("diagnostics.json"), 100);
        let session = SessionId::new();
        for duration in [10, 20, 30, 40, 50, 100] {
            let sample = TuningDiagnosticEvent::new(
                duration,
                session.clone(),
                EventKind::PhraseAttempt,
                TuningStage::Practice,
                OutcomeCode::Valid,
            )
            .with_duration(duration);
            store.append(sample, 100).unwrap();
        }
        let summary = store.health_summary();
        assert_eq!(summary.retained_event_count, 6);
        assert_eq!(summary.event_counts[&EventKind::PhraseAttempt], 6);
        assert_eq!(
            summary.stage_durations[&TuningStage::Practice],
            DurationStatistics {
                samples: 6,
                median_ms: 35,
                p90_ms: 100
            }
        );
        store.clear(None).unwrap();
        assert_eq!(store.health_summary().retained_event_count, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_preview_and_bundle_contain_only_selected_approved_categories() {
        let dir = temp_dir("export");
        let (mut store, _) = TuningDiagnosticsStore::open(dir.join("diagnostics.json"), 1);
        let session = SessionId::new();
        store
            .append(
                event(1, &session, EventKind::SessionCreated).with_reason(ReasonCode::NoMismatch),
                1,
            )
            .unwrap();
        let selection = ExportSelection {
            events: false,
            summary: true,
            environment: true,
        };
        let preview = store.export_preview(selection);
        assert_eq!(
            preview.categories,
            vec![ExportCategory::HealthSummary, ExportCategory::Environment]
        );
        let destination = dir.join("chosen-export.json");
        store
            .export_to(
                &destination,
                selection,
                ExportEnvironment {
                    app_version: "0.1.2".into(),
                    platform: PlatformCode::Macos,
                    backend: BackendCode::Metal,
                    model_fingerprint: Some(
                        ContentFingerprint::from_sha256_hex(&"a".repeat(64)).unwrap(),
                    ),
                    configuration_fingerprint: None,
                },
            )
            .unwrap();
        let text = fs::read_to_string(destination).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json.get("events").is_none());
        assert!(json.get("summary").is_some());
        assert!(json.get("environment").is_some());
        for forbidden in [
            "audio sentinel",
            "transcript sentinel",
            "quick chip",
            "quick ship",
            "dictionary sentinel",
            "/Users/person",
        ] {
            assert!(!text.contains(forbidden));
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_or_unwritable_diagnostics_do_not_block_caller_state() {
        let dir = temp_dir("isolation");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diagnostics.json");
        fs::write(&path, b"{corrupt transcript sentinel").unwrap();
        let (mut store, notice) = TuningDiagnosticsStore::open(path, 1);
        assert_eq!(notice, Some(DiagnosticsNotice::CorruptStoreDiscarded));
        assert!(store.retained_events().is_empty());
        let authoritative_outcome = "candidate accepted";
        store.path = dir.clone(); // replacing a directory is deterministically unwritable
        let result = store.append(event(2, &SessionId::new(), EventKind::SessionCompleted), 2);
        assert!(result.is_err());
        assert_eq!(authoritative_outcome, "candidate accepted");
        assert!(store.retained_events().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
