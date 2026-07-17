//! Durable state for guided Tuning through Candidate Correction Review.

use crate::dictionary::{
    canonical_text, DictEntry, Dictionary, EntryEditState, EntryOrigin,
    VerifiedRecognitionFingerprint,
};
use crate::recognition::RecognitionFingerprint;
use crate::tuning::{
    builtin_corpus, derive_reading_evidence, infer_candidate_correction_from_evidence,
    score_verification_attempt, ReadingEvidence, SessionInferenceResult,
    VerificationAttemptOutcome,
};
use crate::tuning_diagnostics::{ReasonCode as DiagnosticReasonCode, SessionId, TuningStage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

pub const CHECKPOINT_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRowKind {
    Candidate,
    VerifyExisting,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Decline,
    KeepExisting,
    VerifyReplacement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingDictionaryEntry {
    pub id: String,
    pub version: u64,
    pub from: String,
    pub to: String,
}

impl From<&DictEntry> for ExistingDictionaryEntry {
    fn from(entry: &DictEntry) -> Self {
        Self {
            id: entry.id.clone(),
            version: entry.version,
            from: entry.from.clone(),
            to: entry.to.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRow {
    pub id: String,
    pub from: String,
    pub to: String,
    pub supporting_phrase_ids: Vec<String>,
    pub kind: ReviewRowKind,
    pub existing_entry: Option<ExistingDictionaryEntry>,
    pub decision: Option<ReviewDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlreadyCoveredRow {
    pub from: String,
    pub to: String,
    pub supporting_phrase_ids: Vec<String>,
    pub existing_entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewExplanation {
    pub meaning: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewState {
    pub rows: Vec<ReviewRow>,
    pub already_covered: Vec<AlreadyCoveredRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagedRuleKind {
    New,
    Existing,
    Replacement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedRule {
    pub id: String,
    pub from: String,
    pub to: String,
    pub supporting_phrase_ids: Vec<String>,
    pub kind: StagedRuleKind,
    pub existing_entry: Option<ExistingDictionaryEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnchangedResultReason {
    NoSafeCorrectionsFound,
    AlreadyCoveredByPersonalDictionary,
    CandidateCorrectionsFoundButNoneApproved,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationProgress {
    pub verification_id: &'static str,
    pub phrase_text: &'static str,
    pub probe_span_id: String,
    pub position: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRuleOutcome {
    Kept,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationRuleResult {
    pub from: String,
    pub to: String,
    pub outcome: VerificationRuleOutcome,
    pub dictionary_entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletedTuningResult {
    pub rules: Vec<VerificationRuleResult>,
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
    #[serde(default)]
    review: ReviewState,
    #[serde(default)]
    staged_rules: Vec<StagedRule>,
    #[serde(default)]
    unchanged_result_reason: Option<UnchangedResultReason>,
    /// Durable transaction marker written after successful scoring and before
    /// the Personal Dictionary update. Startup can idempotently finish this
    /// commit without asking for another reading or exposing a resumable
    /// pre-commit Verification Pass.
    #[serde(default)]
    pending_commit_verified_at_ms: Option<u64>,
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

    pub fn verification_progress(&self) -> Option<VerificationProgress> {
        if self.pending_commit_verified_at_ms.is_some() {
            return None;
        }
        self.verification_definition()
    }

    fn verification_definition(&self) -> Option<VerificationProgress> {
        if self.stage != TuningStage::Verify || self.staged_rules.len() != 1 {
            return None;
        }
        let rule = &self.staged_rules[0];
        let phrase_id = rule.supporting_phrase_ids.first()?;
        let inference = self.inference_results.iter().find(|result| {
            result.phrase_id == *phrase_id
                && matches!(
                    &result.decision,
                    crate::tuning::InferenceDecision::Candidate(candidate)
                        if canonical_text(&candidate.from) == canonical_text(&rule.from)
                            && canonical_text(&candidate.to) == canonical_text(&rule.to)
                )
        })?;
        let crate::tuning::InferenceDecision::Candidate(candidate) = &inference.decision else {
            return None;
        };
        let phrase = builtin_corpus().phrase(phrase_id)?;
        Some(VerificationProgress {
            verification_id: phrase.verification_id,
            phrase_text: phrase.verification_text,
            probe_span_id: candidate.probe_span_id.clone(),
            position: 1,
            total: 1,
        })
    }

    pub fn candidate_count(&self) -> usize {
        self.inference_results
            .iter()
            .filter(|result| {
                matches!(
                    result.decision,
                    crate::tuning::InferenceDecision::Candidate(_)
                )
            })
            .count()
    }

    pub fn review(&self) -> &ReviewState {
        &self.review
    }

    pub fn staged_rules(&self) -> &[StagedRule] {
        &self.staged_rules
    }

    pub fn unchanged_result_reason(&self) -> Option<UnchangedResultReason> {
        self.unchanged_result_reason
    }

    pub fn review_complete(&self) -> bool {
        self.stage == TuningStage::Review
            && self.review.rows.iter().all(|row| row.decision.is_some())
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

struct CandidateGroup {
    from: String,
    to: String,
    canonical_from: String,
    canonical_to: String,
    supporting_phrase_ids: Vec<String>,
}

fn candidate_groups(inference_results: &[SessionInferenceResult]) -> Vec<CandidateGroup> {
    let mut groups: Vec<CandidateGroup> = Vec::new();
    for result in inference_results {
        let crate::tuning::InferenceDecision::Candidate(candidate) = &result.decision else {
            continue;
        };
        let canonical_from = canonical_text(&candidate.from);
        let canonical_to = canonical_text(&candidate.to);
        if let Some(group) = groups.iter_mut().find(|group| {
            group.canonical_from == canonical_from && group.canonical_to == canonical_to
        }) {
            if !group.supporting_phrase_ids.contains(&result.phrase_id) {
                group.supporting_phrase_ids.push(result.phrase_id.clone());
            }
        } else {
            groups.push(CandidateGroup {
                from: candidate.from.clone(),
                to: candidate.to.clone(),
                canonical_from,
                canonical_to,
                supporting_phrase_ids: vec![result.phrase_id.clone()],
            });
        }
    }
    groups
}

fn ambiguous_sources(groups: &[CandidateGroup]) -> HashSet<String> {
    groups
        .iter()
        .filter_map(|group| {
            let distinct_targets = groups
                .iter()
                .filter(|other| other.canonical_from == group.canonical_from)
                .map(|other| other.canonical_to.as_str())
                .collect::<HashSet<_>>();
            (distinct_targets.len() > 1).then(|| group.canonical_from.clone())
        })
        .collect()
}

pub fn ambiguous_phrase_ids(inference_results: &[SessionInferenceResult]) -> Vec<String> {
    let groups = candidate_groups(inference_results);
    let ambiguous = ambiguous_sources(&groups);
    groups
        .iter()
        .filter(|group| ambiguous.contains(&group.canonical_from))
        .flat_map(|group| group.supporting_phrase_ids.iter().cloned())
        .collect()
}

pub fn review_explanations(inference_results: &[SessionInferenceResult]) -> Vec<ReviewExplanation> {
    let mut explanation_counts = BTreeMap::<String, usize>::new();
    for result in inference_results {
        if !matches!(result.decision, crate::tuning::InferenceDecision::Rejected) {
            continue;
        }
        let mut meanings = HashSet::new();
        for reason in &result.aggregate_reason_codes {
            meanings.insert(DiagnosticReasonCode::from(*reason).user_facing_meaning());
        }
        for meaning in meanings {
            *explanation_counts.entry(meaning.to_owned()).or_default() += 1;
        }
    }
    let ambiguous_count = ambiguous_phrase_ids(inference_results).len();
    if ambiguous_count > 0 {
        *explanation_counts
            .entry(
                DiagnosticReasonCode::OutsideEligibleSpan
                    .user_facing_meaning()
                    .to_owned(),
            )
            .or_default() += ambiguous_count;
    }
    explanation_counts
        .into_iter()
        .map(|(meaning, count)| ReviewExplanation { meaning, count })
        .collect()
}

pub fn build_review(
    inference_results: &[SessionInferenceResult],
    dictionary: &Dictionary,
    fingerprint: &RecognitionFingerprint,
) -> ReviewState {
    let groups = candidate_groups(inference_results);
    let ambiguous_sources = ambiguous_sources(&groups);
    let mut review = ReviewState::default();

    for group in groups
        .into_iter()
        .filter(|group| !ambiguous_sources.contains(&group.canonical_from))
    {
        let existing = dictionary.entry_for_source(&group.from);
        if let Some(existing) = existing {
            if existing.has_equivalent_mapping(&group.from, &group.to) {
                if existing.is_active_for(Some(fingerprint)) {
                    review.already_covered.push(AlreadyCoveredRow {
                        from: existing.from.clone(),
                        to: existing.to.clone(),
                        supporting_phrase_ids: group.supporting_phrase_ids,
                        existing_entry_id: existing.id.clone(),
                    });
                    continue;
                }
                debug_assert_eq!(existing.origin, EntryOrigin::Tuning);
                debug_assert_eq!(existing.edit_state, EntryEditState::Unmodified);
                review.rows.push(ReviewRow {
                    id: Uuid::new_v4().to_string(),
                    from: group.from,
                    to: group.to,
                    supporting_phrase_ids: group.supporting_phrase_ids,
                    kind: ReviewRowKind::VerifyExisting,
                    existing_entry: Some(existing.into()),
                    decision: None,
                });
                continue;
            }
            review.rows.push(ReviewRow {
                id: Uuid::new_v4().to_string(),
                from: group.from,
                to: group.to,
                supporting_phrase_ids: group.supporting_phrase_ids,
                kind: ReviewRowKind::Conflict,
                existing_entry: Some(existing.into()),
                decision: None,
            });
            continue;
        }
        review.rows.push(ReviewRow {
            id: Uuid::new_v4().to_string(),
            from: group.from,
            to: group.to,
            supporting_phrase_ids: group.supporting_phrase_ids,
            kind: ReviewRowKind::Candidate,
            existing_entry: None,
            decision: None,
        });
    }
    review
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointState {
    None,
    Compatible(Box<TuningCheckpoint>),
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
            review: ReviewState::default(),
            staged_rules: Vec::new(),
            unchanged_result_reason: None,
            pending_commit_verified_at_ms: None,
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
        CheckpointState::Compatible(Box::new(checkpoint))
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

    pub fn record_review_decision(
        &self,
        checkpoint: &TuningCheckpoint,
        row_id: &str,
        decision: ReviewDecision,
    ) -> Result<TuningCheckpoint, String> {
        if checkpoint.stage != TuningStage::Review {
            return Err("Review is not the current durable Tuning stage".into());
        }
        let mut candidate = checkpoint.clone();
        let row = candidate
            .review
            .rows
            .iter_mut()
            .find(|row| row.id == row_id)
            .ok_or_else(|| "The Candidate Correction is no longer in Review".to_owned())?;
        let valid = matches!(
            (row.kind, decision),
            (
                ReviewRowKind::Candidate | ReviewRowKind::VerifyExisting,
                ReviewDecision::Approve | ReviewDecision::Decline
            ) | (
                ReviewRowKind::Conflict,
                ReviewDecision::KeepExisting | ReviewDecision::VerifyReplacement
            )
        );
        if !valid {
            return Err("That decision is not valid for this Review row".into());
        }
        row.decision = Some(decision);
        self.save(&candidate)?;
        Ok(candidate)
    }

    pub fn continue_review(
        &self,
        checkpoint: &TuningCheckpoint,
    ) -> Result<TuningCheckpoint, String> {
        if checkpoint.stage != TuningStage::Review {
            return Err("Review is not the current durable Tuning stage".into());
        }
        if !checkpoint.review_complete() {
            return Err("Every Candidate Correction needs an explicit Review decision".into());
        }

        let mut candidate = checkpoint.clone();
        candidate.staged_rules = candidate
            .review
            .rows
            .iter()
            .filter_map(|row| {
                let kind = match (row.kind, row.decision) {
                    (ReviewRowKind::Candidate, Some(ReviewDecision::Approve)) => {
                        StagedRuleKind::New
                    }
                    (ReviewRowKind::VerifyExisting, Some(ReviewDecision::Approve)) => {
                        StagedRuleKind::Existing
                    }
                    (ReviewRowKind::Conflict, Some(ReviewDecision::VerifyReplacement)) => {
                        StagedRuleKind::Replacement
                    }
                    _ => return None,
                };
                Some(StagedRule {
                    id: Uuid::new_v4().to_string(),
                    from: row.from.clone(),
                    to: row.to.clone(),
                    supporting_phrase_ids: row.supporting_phrase_ids.clone(),
                    kind,
                    existing_entry: row.existing_entry.clone(),
                })
            })
            .collect();

        if candidate.staged_rules.is_empty() {
            candidate.stage = TuningStage::Result;
            candidate.unchanged_result_reason = Some(if !candidate.review.rows.is_empty() {
                UnchangedResultReason::CandidateCorrectionsFoundButNoneApproved
            } else if !candidate.review.already_covered.is_empty() {
                UnchangedResultReason::AlreadyCoveredByPersonalDictionary
            } else {
                UnchangedResultReason::NoSafeCorrectionsFound
            });
            candidate.inference_results.clear();
            candidate.review = ReviewState::default();
            self.delete_checkpoint(checkpoint)?;
        } else {
            candidate.stage = TuningStage::Verify;
            candidate.unchanged_result_reason = None;
            self.save(&candidate)?;
        }
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
        self.complete_reading_with_dictionary(checkpoint, raw_transcript, &Dictionary::default())
    }

    pub fn complete_reading_with_dictionary(
        &self,
        checkpoint: &TuningCheckpoint,
        raw_transcript: &str,
        dictionary: &Dictionary,
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
        candidate.reading_evidence.push(StoredReadingEvidence {
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
                    candidate.review = build_review(
                        &candidate.inference_results,
                        dictionary,
                        &candidate.envelope.recognition_fingerprint,
                    );
                    candidate.stage = TuningStage::Review;
                }
            }
        }

        self.save(&candidate)?;
        Ok(candidate)
    }

    /// Exercise the single staged Correction Rule through the production
    /// dictionary matcher plus a Tuning-only overlay. A successful attempt is
    /// committed in one atomic Personal Dictionary write; Result is returned
    /// only after the unfinished checkpoint and all derived evidence are gone.
    pub fn complete_verification_and_commit(
        &self,
        checkpoint: &TuningCheckpoint,
        raw_transcript: &str,
        dictionary: &Dictionary,
        dictionary_path: &Path,
        verified_at_ms: u64,
    ) -> Result<(CompletedTuningResult, Dictionary), String> {
        let progress = checkpoint.verification_definition().ok_or_else(|| {
            "Exactly one Correction Rule must be ready for verification".to_owned()
        })?;
        let rule = checkpoint
            .staged_rules
            .first()
            .ok_or_else(|| "No Correction Rule is staged for verification".to_owned())?;
        let phrase_id = rule.supporting_phrase_ids.first().ok_or_else(|| {
            "The staged Correction Rule has no supporting Tuning Phrase".to_owned()
        })?;
        let phrase = builtin_corpus().phrase(phrase_id).ok_or_else(|| {
            "The staged Correction Rule references an unknown Tuning Phrase".to_owned()
        })?;
        let overlay = dictionary.apply_tuning_overlay(
            raw_transcript,
            Some(&checkpoint.envelope.recognition_fingerprint),
            &[(rule.from.as_str(), rule.to.as_str())],
        );
        let outcome = score_verification_attempt(
            phrase,
            &progress.probe_span_id,
            &rule.from,
            &rule.to,
            &overlay.pre_overlay,
            &overlay.post_overlay,
        );
        if outcome != VerificationAttemptOutcome::Success {
            return Err(format!("The Correction Rule was not verified: {outcome:?}"));
        }

        let mut prepared = checkpoint.clone();
        prepared.pending_commit_verified_at_ms = Some(verified_at_ms);
        self.save(&prepared)?;
        self.finalize_pending_commit_checkpoint(&prepared, dictionary, dictionary_path)
    }

    /// Reconcile the durable transaction marker left by a crash or interrupted
    /// checkpoint cleanup. The dictionary mutation is idempotent, so startup
    /// converges to one committed rule and no unfinished evidence.
    pub fn recover_pending_commit(
        &self,
        dictionary: &Dictionary,
        dictionary_path: &Path,
    ) -> Result<Option<(CompletedTuningResult, Dictionary)>, String> {
        let Some(checkpoint) = self.load_saved()? else {
            return Ok(None);
        };
        if checkpoint.pending_commit_verified_at_ms.is_none() {
            return Ok(None);
        }
        self.finalize_pending_commit_checkpoint(&checkpoint, dictionary, dictionary_path)
            .map(Some)
    }

    fn finalize_pending_commit_checkpoint(
        &self,
        checkpoint: &TuningCheckpoint,
        dictionary: &Dictionary,
        dictionary_path: &Path,
    ) -> Result<(CompletedTuningResult, Dictionary), String> {
        let verified_at_ms = checkpoint
            .pending_commit_verified_at_ms
            .ok_or_else(|| "The Tuning dictionary transaction is not prepared".to_owned())?;
        let rule = checkpoint
            .staged_rules
            .first()
            .filter(|_| checkpoint.staged_rules.len() == 1)
            .ok_or_else(|| "Exactly one Correction Rule must be ready to commit".to_owned())?;
        let mut committed = dictionary.clone();
        let verified = VerifiedRecognitionFingerprint {
            fingerprint: checkpoint.envelope.recognition_fingerprint.clone(),
            verified_at_ms,
        };
        let (dictionary_entry_id, dictionary_changed) = match rule.kind {
            StagedRuleKind::New => {
                if let Some(existing) = committed.entry_for_source(&rule.from) {
                    let already_committed = existing.id == rule.id
                        && existing.has_equivalent_mapping(&rule.from, &rule.to)
                        && existing.origin == EntryOrigin::Tuning
                        && existing.edit_state == EntryEditState::Unmodified
                        && existing
                            .verified_fingerprints
                            .iter()
                            .any(|stored| stored == &verified);
                    if already_committed {
                        (existing.id.clone(), false)
                    } else {
                        return Err("The staged dictionary key changed before commit".into());
                    }
                } else {
                    committed.entries.push(DictEntry {
                        id: rule.id.clone(),
                        from: rule.from.clone(),
                        to: rule.to.clone(),
                        origin: EntryOrigin::Tuning,
                        edit_state: EntryEditState::Unmodified,
                        verified_fingerprints: vec![verified],
                        version: 1,
                    });
                    committed.revision = committed.revision.saturating_add(1);
                    (rule.id.clone(), true)
                }
            }
            StagedRuleKind::Existing => {
                let expected = rule.existing_entry.as_ref().ok_or_else(|| {
                    "The existing staged rule lost its dictionary identity".to_owned()
                })?;
                if let Some(entry) = committed.entries.iter().find(|entry| {
                    entry.id == expected.id
                        && entry.version == expected.version.saturating_add(1)
                        && entry.has_equivalent_mapping(&rule.from, &rule.to)
                        && entry
                            .verified_fingerprints
                            .iter()
                            .any(|stored| stored == &verified)
                }) {
                    (entry.id.clone(), false)
                } else {
                    let entry = committed
                        .entries
                        .iter_mut()
                        .find(|entry| entry.id == expected.id && entry.version == expected.version)
                        .ok_or_else(|| {
                            "The staged dictionary key changed before commit".to_owned()
                        })?;
                    if !entry.has_equivalent_mapping(&rule.from, &rule.to) {
                        return Err("The staged dictionary mapping changed before commit".into());
                    }
                    entry.verified_fingerprints.push(verified);
                    entry.version = entry.version.saturating_add(1);
                    let id = entry.id.clone();
                    committed.revision = committed.revision.saturating_add(1);
                    (id, true)
                }
            }
            StagedRuleKind::Replacement => {
                let expected = rule
                    .existing_entry
                    .as_ref()
                    .ok_or_else(|| "The replacement lost its dictionary identity".to_owned())?;
                if let Some(existing) = committed.entry_for_source(&rule.from) {
                    let already_committed = existing.id == rule.id
                        && existing.has_equivalent_mapping(&rule.from, &rule.to)
                        && existing.origin == EntryOrigin::Tuning
                        && existing.edit_state == EntryEditState::Unmodified
                        && existing
                            .verified_fingerprints
                            .iter()
                            .any(|stored| stored == &verified);
                    if already_committed {
                        (existing.id.clone(), false)
                    } else {
                        let index = committed
                            .entries
                            .iter()
                            .position(|entry| {
                                entry.id == expected.id && entry.version == expected.version
                            })
                            .ok_or_else(|| {
                                "The staged dictionary key changed before commit".to_owned()
                            })?;
                        committed.entries.remove(index);
                        committed.entries.push(DictEntry {
                            id: rule.id.clone(),
                            from: rule.from.clone(),
                            to: rule.to.clone(),
                            origin: EntryOrigin::Tuning,
                            edit_state: EntryEditState::Unmodified,
                            verified_fingerprints: vec![verified],
                            version: 1,
                        });
                        committed.revision = committed.revision.saturating_add(1);
                        (rule.id.clone(), true)
                    }
                } else {
                    return Err("The staged dictionary key changed before commit".into());
                }
            }
        };
        if dictionary_changed {
            committed
                .save(dictionary_path)
                .map_err(|error| error.to_string())?;
        }
        self.delete_checkpoint(checkpoint)?;
        Ok((
            CompletedTuningResult {
                rules: vec![VerificationRuleResult {
                    from: rule.from.clone(),
                    to: rule.to.clone(),
                    outcome: VerificationRuleOutcome::Kept,
                    dictionary_entry_id,
                }],
            },
            committed,
        ))
    }

    fn save(&self, checkpoint: &TuningCheckpoint) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(checkpoint)
            .map_err(|error| format!("Serialize Tuning checkpoint failed: {error}"))?;
        atomic_replace(&self.path, &bytes)
    }

    fn delete_checkpoint(&self, resumable: &TuningCheckpoint) -> Result<(), String> {
        fs::remove_file(&self.path)
            .map_err(|error| format!("Delete completed Tuning checkpoint failed: {error}"))?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        if let Ok(directory) = fs::File::open(parent) {
            if let Err(error) = directory.sync_all() {
                let _ = self.save(resumable);
                return Err(format!(
                    "Sync completed Tuning checkpoint deletion failed: {error}"
                ));
            }
        }
        Ok(())
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
    use crate::dictionary::{
        DictEntry, Dictionary, EntryEditState, EntryOrigin, VerifiedRecognitionFingerprint,
    };
    use crate::tuning::{CandidateCorrection, CandidateState, InferenceDecision};
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

    fn candidate(phrase_id: &str, from: &str, to: &str) -> SessionInferenceResult {
        SessionInferenceResult {
            phrase_id: phrase_id.into(),
            decision: InferenceDecision::Candidate(CandidateCorrection {
                probe_span_id: format!("{phrase_id}-P01"),
                from: from.into(),
                to: to.into(),
                state: CandidateState::Inactive,
            }),
            reading_reason_codes: [Vec::new(), Vec::new()],
            aggregate_reason_codes: Vec::new(),
        }
    }

    fn dictionary_entry(
        id: &str,
        from: &str,
        to: &str,
        origin: EntryOrigin,
        verified_fingerprints: Vec<VerifiedRecognitionFingerprint>,
    ) -> DictEntry {
        DictEntry {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            origin,
            edit_state: EntryEditState::Unmodified,
            verified_fingerprints,
            version: 1,
        }
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
            CheckpointState::Compatible(Box::new(started))
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
        assert_eq!(reloaded, CheckpointState::Compatible(Box::new(interrupted)));
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
            CheckpointState::Compatible(Box::new(completed))
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

        for expected_id in [
            "T02", "T03", "T04", "T05", "T06", "T07", "T08", "T09", "T10", "T01",
        ] {
            let progress = checkpoint.reading_progress().unwrap();
            assert_eq!(progress.phrase_id, expected_id);
            checkpoint = store
                .complete_reading(
                    &checkpoint,
                    builtin_corpus().phrase(expected_id).unwrap().text,
                )
                .unwrap();
        }

        assert_eq!(checkpoint.stage(), TuningStage::SecondReading);
        assert_eq!(checkpoint.reading_progress().unwrap().phrase_id, "T06");
        for expected_id in [
            "T06", "T07", "T08", "T09", "T10", "T01", "T02", "T03", "T04", "T05",
        ] {
            let progress = checkpoint.reading_progress().unwrap();
            assert_eq!(progress.phrase_id, expected_id);
            checkpoint = store
                .complete_reading(
                    &checkpoint,
                    builtin_corpus().phrase(expected_id).unwrap().text,
                )
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
            .complete_reading(&checkpoint, "Your boys made the joyful choice sound easy")
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

    #[test]
    fn review_coalesces_equivalent_candidates_and_rejects_ambiguous_sources() {
        let fingerprint = RecognitionFingerprint::from_stable_id("recognition-a");
        let inference = vec![
            candidate("T01", "Quick   Chip", "Quick Ship"),
            candidate("T02", "quick chip", "quick ship"),
            candidate("T03", "up stairs", "upstairs"),
            candidate("T04", "UP STAIRS", "up the stairs"),
        ];

        let review = build_review(&inference, &Dictionary::default(), &fingerprint);

        assert_eq!(review.rows.len(), 1);
        assert_eq!(review.rows[0].from, "Quick   Chip");
        assert_eq!(review.rows[0].to, "Quick Ship");
        assert_eq!(review.rows[0].supporting_phrase_ids, ["T01", "T02"]);
        assert_eq!(review.rows[0].kind, ReviewRowKind::Candidate);
        assert_eq!(review.rows[0].decision, None);
        assert_eq!(ambiguous_phrase_ids(&inference), ["T03", "T04"]);
        assert!(review_explanations(&inference)
            .iter()
            .any(|explanation| explanation.meaning == "Too broad to apply safely"));
    }

    #[test]
    fn review_classifies_covered_inactive_and_conflicting_dictionary_entries() {
        let current = RecognitionFingerprint::from_stable_id("recognition-current");
        let previous = RecognitionFingerprint::from_stable_id("recognition-previous");
        let mut modified_tuning = dictionary_entry(
            "modified",
            "judge shoes",
            "judge chose",
            EntryOrigin::Tuning,
            vec![],
        );
        modified_tuning.edit_state = EntryEditState::ModifiedAfterVerification;
        let dictionary = Dictionary {
            entries: vec![
                dictionary_entry(
                    "manual",
                    "quick chip",
                    "quick ship",
                    EntryOrigin::Manual,
                    vec![],
                ),
                dictionary_entry(
                    "active",
                    "late crane",
                    "late train",
                    EntryOrigin::Tuning,
                    vec![VerifiedRecognitionFingerprint {
                        fingerprint: current.clone(),
                        verified_at_ms: 20,
                    }],
                ),
                modified_tuning,
                dictionary_entry(
                    "inactive",
                    "up stairs",
                    "upstairs",
                    EntryOrigin::Tuning,
                    vec![VerifiedRecognitionFingerprint {
                        fingerprint: previous,
                        verified_at_ms: 10,
                    }],
                ),
                dictionary_entry(
                    "conflict",
                    "brown socks",
                    "brown box",
                    EntryOrigin::Manual,
                    vec![],
                ),
            ],
            ..Dictionary::default()
        };
        let inference = vec![
            candidate("T01", "QUICK CHIP", "QUICK SHIP"),
            candidate("T09", "late crane", "late train"),
            candidate("T10", "judge shoes", "judge chose"),
            candidate("T05", "up stairs", "upstairs"),
            candidate("T08", "brown socks", "brown fox"),
        ];

        let review = build_review(&inference, &dictionary, &current);

        assert_eq!(review.already_covered.len(), 3);
        assert_eq!(review.already_covered[0].existing_entry_id, "manual");
        assert_eq!(review.already_covered[1].existing_entry_id, "active");
        assert_eq!(review.already_covered[2].existing_entry_id, "modified");
        assert_eq!(review.rows.len(), 2);
        assert_eq!(review.rows[0].kind, ReviewRowKind::VerifyExisting);
        assert_eq!(
            review.rows[0].existing_entry.as_ref().unwrap().id,
            "inactive"
        );
        assert_eq!(review.rows[1].kind, ReviewRowKind::Conflict);
        assert_eq!(
            review.rows[1].existing_entry.as_ref().unwrap().to,
            "brown box"
        );
    }

    #[test]
    fn every_actionable_review_row_requires_a_valid_explicit_decision() {
        let path = temp_path("review-decisions");
        let store = TuningCheckpointStore::new(path);
        let current = RecognitionFingerprint::from_stable_id("recognition-current");
        let previous = RecognitionFingerprint::from_stable_id("recognition-previous");
        let dictionary = Dictionary {
            entries: vec![
                dictionary_entry(
                    "inactive",
                    "up stairs",
                    "upstairs",
                    EntryOrigin::Tuning,
                    vec![VerifiedRecognitionFingerprint {
                        fingerprint: previous,
                        verified_at_ms: 10,
                    }],
                ),
                dictionary_entry(
                    "conflict",
                    "brown socks",
                    "brown box",
                    EntryOrigin::Manual,
                    vec![],
                ),
            ],
            ..Dictionary::default()
        };
        let inference = vec![
            candidate("T01", "quick chip", "quick ship"),
            candidate("T05", "up stairs", "upstairs"),
            candidate("T08", "brown socks", "brown fox"),
        ];
        let mut checkpoint = store
            .start(CompatibilityEnvelope::current(current.clone()))
            .unwrap();
        checkpoint.stage = TuningStage::Review;
        checkpoint.inference_results = inference.clone();
        checkpoint.review = build_review(&inference, &dictionary, &current);
        store.save(&checkpoint).unwrap();

        assert!(!checkpoint.review_complete());
        assert!(store.continue_review(&checkpoint).is_err());
        let candidate_id = checkpoint.review.rows[0].id.clone();
        assert!(store
            .record_review_decision(&checkpoint, &candidate_id, ReviewDecision::KeepExisting)
            .is_err());

        for (index, decision) in [
            ReviewDecision::Approve,
            ReviewDecision::Approve,
            ReviewDecision::VerifyReplacement,
        ]
        .into_iter()
        .enumerate()
        {
            let row_id = checkpoint.review.rows[index].id.clone();
            checkpoint = store
                .record_review_decision(&checkpoint, &row_id, decision)
                .unwrap();
        }

        assert!(checkpoint.review_complete());
        let continued = store.continue_review(&checkpoint).unwrap();
        assert_eq!(continued.stage(), TuningStage::Verify);
        assert_eq!(continued.staged_rules().len(), 3);
        assert_eq!(continued.staged_rules()[0].kind, StagedRuleKind::New);
        assert_eq!(continued.staged_rules()[0].from, "quick chip");
        assert_eq!(continued.staged_rules()[1].kind, StagedRuleKind::Existing);
        assert_eq!(
            continued.staged_rules()[1]
                .existing_entry
                .as_ref()
                .unwrap()
                .id,
            "inactive"
        );
        assert_eq!(
            continued.staged_rules()[2].kind,
            StagedRuleKind::Replacement
        );
        assert_eq!(
            continued.staged_rules()[2]
                .existing_entry
                .as_ref()
                .unwrap()
                .id,
            "conflict"
        );
    }

    #[test]
    fn one_successful_verification_atomically_commits_and_removes_session_evidence() {
        let checkpoint_path = temp_path("verify-kept");
        let dictionary_path = checkpoint_path.with_file_name("dictionary.json");
        let store = TuningCheckpointStore::new(checkpoint_path.clone());
        let fingerprint = RecognitionFingerprint::from_stable_id("recognition-current");
        let mut checkpoint = store
            .start(CompatibilityEnvelope::current(fingerprint.clone()))
            .unwrap();
        checkpoint.stage = TuningStage::Review;
        checkpoint.inference_results = vec![candidate("T01", "quick chip", "quick ship")];
        checkpoint.review = build_review(
            &checkpoint.inference_results,
            &Dictionary::default(),
            &fingerprint,
        );
        store.save(&checkpoint).unwrap();
        let row_id = checkpoint.review.rows[0].id.clone();
        checkpoint = store
            .record_review_decision(&checkpoint, &row_id, ReviewDecision::Approve)
            .unwrap();
        checkpoint = store.continue_review(&checkpoint).unwrap();

        let progress = checkpoint.verification_progress().unwrap();
        assert_eq!(progress.verification_id, "V01");
        assert_eq!(
            progress.phrase_text,
            "The heavy blue boxes arrived on a quick ship."
        );
        assert_eq!(
            Dictionary::default().apply_for_fingerprint("a quick chip", Some(&fingerprint)),
            "a quick chip",
            "the staged rule remains inactive in ordinary dictation"
        );

        let (result, dictionary) = store
            .complete_verification_and_commit(
                &checkpoint,
                "The heavy blue boxes arrived on a quick chip.",
                &Dictionary::default(),
                &dictionary_path,
                42,
            )
            .unwrap();

        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].outcome, VerificationRuleOutcome::Kept);
        assert!(!checkpoint_path.exists());
        let entry = &dictionary.entries[0];
        assert_eq!(entry.id, result.rules[0].dictionary_entry_id);
        assert_eq!(entry.origin, EntryOrigin::Tuning);
        assert_eq!(entry.edit_state, EntryEditState::Unmodified);
        assert_eq!(entry.version, 1);
        assert_eq!(entry.verified_fingerprints.len(), 1);
        assert_eq!(entry.verified_fingerprints[0].fingerprint, fingerprint);
        assert_eq!(entry.verified_fingerprints[0].verified_at_ms, 42);

        let persisted = fs::read_to_string(dictionary_path).unwrap();
        assert!(!persisted.contains("T01"));
        assert!(!persisted.contains("V01"));
        assert!(!persisted.contains("heavy blue boxes"));
        assert!(!persisted.contains("The heavy blue boxes arrived"));
    }

    #[test]
    fn result_cannot_appear_when_the_atomic_dictionary_update_fails() {
        let checkpoint_path = temp_path("verify-storage-failure");
        let dictionary_path = checkpoint_path.with_file_name("dictionary.json");
        let store = TuningCheckpointStore::new(checkpoint_path.clone());
        let fingerprint = RecognitionFingerprint::from_stable_id("recognition-current");
        let mut checkpoint = store
            .start(CompatibilityEnvelope::current(fingerprint.clone()))
            .unwrap();
        checkpoint.stage = TuningStage::Review;
        checkpoint.inference_results = vec![candidate("T01", "quick chip", "quick ship")];
        checkpoint.review = build_review(
            &checkpoint.inference_results,
            &Dictionary::default(),
            &fingerprint,
        );
        store.save(&checkpoint).unwrap();
        let row_id = checkpoint.review.rows[0].id.clone();
        checkpoint = store
            .record_review_decision(&checkpoint, &row_id, ReviewDecision::Approve)
            .unwrap();
        checkpoint = store.continue_review(&checkpoint).unwrap();
        fs::create_dir(&dictionary_path).unwrap();

        let result = store.complete_verification_and_commit(
            &checkpoint,
            "The heavy blue boxes arrived on a quick chip.",
            &Dictionary::default(),
            &dictionary_path,
            42,
        );

        assert!(result.is_err());
        assert!(checkpoint_path.exists());
        assert_eq!(checkpoint.stage(), TuningStage::Verify);
        assert!(Dictionary::default().entries.is_empty());
    }

    #[test]
    fn startup_finishes_a_prepared_dictionary_commit_without_another_reading() {
        let checkpoint_path = temp_path("verify-commit-recovery");
        let dictionary_path = checkpoint_path.with_file_name("dictionary.json");
        let store = TuningCheckpointStore::new(checkpoint_path.clone());
        let fingerprint = RecognitionFingerprint::from_stable_id("recognition-current");
        let mut checkpoint = store
            .start(CompatibilityEnvelope::current(fingerprint.clone()))
            .unwrap();
        checkpoint.stage = TuningStage::Review;
        checkpoint.inference_results = vec![candidate("T01", "quick chip", "quick ship")];
        checkpoint.review = build_review(
            &checkpoint.inference_results,
            &Dictionary::default(),
            &fingerprint,
        );
        store.save(&checkpoint).unwrap();
        let row_id = checkpoint.review.rows[0].id.clone();
        checkpoint = store
            .record_review_decision(&checkpoint, &row_id, ReviewDecision::Approve)
            .unwrap();
        checkpoint = store.continue_review(&checkpoint).unwrap();
        checkpoint.pending_commit_verified_at_ms = Some(99);
        store.save(&checkpoint).unwrap();

        let recovered = store
            .recover_pending_commit(&Dictionary::default(), &dictionary_path)
            .unwrap()
            .expect("prepared commit should be finalized");

        assert_eq!(recovered.0.rules[0].outcome, VerificationRuleOutcome::Kept);
        assert_eq!(recovered.1.entries.len(), 1);
        assert_eq!(
            recovered.1.entries[0].verified_fingerprints[0].verified_at_ms,
            99
        );
        assert!(!checkpoint_path.exists());
    }

    #[test]
    fn approving_no_rules_completes_with_the_precise_unchanged_reason() {
        let fingerprint = RecognitionFingerprint::from_stable_id("recognition-current");

        let no_safe_path = temp_path("review-no-safe");
        let no_safe_store = TuningCheckpointStore::new(no_safe_path.clone());
        let mut no_safe = no_safe_store
            .start(CompatibilityEnvelope::current(fingerprint.clone()))
            .unwrap();
        no_safe.stage = TuningStage::Review;
        no_safe.review = ReviewState::default();
        no_safe_store.save(&no_safe).unwrap();
        let no_safe = no_safe_store.continue_review(&no_safe).unwrap();
        assert_eq!(no_safe.stage(), TuningStage::Result);
        assert_eq!(
            no_safe.unchanged_result_reason(),
            Some(UnchangedResultReason::NoSafeCorrectionsFound)
        );
        assert!(!no_safe_path.exists());
        assert!(no_safe.inference_results().is_empty());
        assert!(no_safe.review().rows.is_empty());

        let covered_store = TuningCheckpointStore::new(temp_path("review-covered"));
        let mut covered = covered_store
            .start(CompatibilityEnvelope::current(fingerprint.clone()))
            .unwrap();
        covered.stage = TuningStage::Review;
        covered.inference_results = vec![candidate("T01", "quick chip", "quick ship")];
        covered.review = ReviewState {
            already_covered: vec![AlreadyCoveredRow {
                from: "quick chip".into(),
                to: "quick ship".into(),
                supporting_phrase_ids: vec!["T01".into()],
                existing_entry_id: "manual".into(),
            }],
            ..ReviewState::default()
        };
        covered_store.save(&covered).unwrap();
        let covered = covered_store.continue_review(&covered).unwrap();
        assert_eq!(
            covered.unchanged_result_reason(),
            Some(UnchangedResultReason::AlreadyCoveredByPersonalDictionary)
        );

        let declined_store = TuningCheckpointStore::new(temp_path("review-declined"));
        let mut declined = declined_store
            .start(CompatibilityEnvelope::current(fingerprint))
            .unwrap();
        declined.stage = TuningStage::Review;
        declined.inference_results = vec![candidate("T01", "quick chip", "quick ship")];
        declined.review = ReviewState {
            rows: vec![ReviewRow {
                id: "row-1".into(),
                from: "quick chip".into(),
                to: "quick ship".into(),
                supporting_phrase_ids: vec!["T01".into()],
                kind: ReviewRowKind::Candidate,
                existing_entry: None,
                decision: Some(ReviewDecision::Decline),
            }],
            ..ReviewState::default()
        };
        declined_store.save(&declined).unwrap();
        let declined = declined_store.continue_review(&declined).unwrap();
        assert_eq!(
            declined.unchanged_result_reason(),
            Some(UnchangedResultReason::CandidateCorrectionsFoundButNoneApproved)
        );
    }
}
