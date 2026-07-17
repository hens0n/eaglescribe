//! Platform-independent pull-request fixtures for the complete Tuning contract.
//!
//! These fixtures deliberately enter through the same corpus, normalizer,
//! inference, dictionary overlay, Verification Pass, checkpoint, and diagnostics
//! APIs as the product. Candidate Corrections are never constructed or refreshed
//! from expected snapshots.

use crate::dictionary::Dictionary;
use crate::recognition::RecognitionFingerprint;
use crate::tuning::{builtin_corpus, InferenceDecision};
use crate::tuning_diagnostics::{
    BackendCode, EventKind, ExportEnvironment, ExportSelection, OutcomeCode, PlatformCode,
    SessionId, TuningDiagnosticEvent, TuningDiagnosticsStore, TuningStage,
};
use crate::tuning_session::{
    CheckpointState, CompatibilityEnvelope, ReadingPass, ReviewDecision, TuningCheckpoint,
    TuningCheckpointStore, UnchangedResultReason, VerificationAdvance, VerificationRuleOutcome,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TRANSCRIPT_SENTINEL: &str = "private transcript sentinel";
const CORRECTION_SENTINEL: &str = "private correction sentinel";
const DICTIONARY_SENTINEL: &str = "private dictionary sentinel";
const AUDIO_SENTINEL: &[u8] = b"private audio sentinel";

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("eaglescribe-tuning-ci-{label}-{nanos}"))
}

fn start_readings(store: &TuningCheckpointStore) -> TuningCheckpoint {
    let checkpoint = store
        .start(CompatibilityEnvelope::current(
            RecognitionFingerprint::from_stable_id("ci-recognition-fingerprint"),
        ))
        .expect("fixture preflight checkpoint must save");
    store
        .complete_practice(&checkpoint)
        .expect("fixture practice must save")
}

fn complete_readings(
    store: &TuningCheckpointStore,
    mut checkpoint: TuningCheckpoint,
    transcript: impl Fn(&str, ReadingPass, &str) -> String,
) -> TuningCheckpoint {
    while let Some(progress) = checkpoint.reading_progress() {
        let raw = transcript(&progress.phrase_id, progress.pass, progress.phrase_text);
        checkpoint = store
            .complete_reading(&checkpoint, &raw)
            .expect("fixture reading must save through production inference");
    }
    assert_eq!(checkpoint.stage(), TuningStage::Review);
    checkpoint
}

fn candidate_transcript(phrase_id: &str, phrase_text: &str) -> String {
    match phrase_id {
        "T01" => phrase_text.replace("quick ship", "quick chip"),
        "T05" => phrase_text.replace("upstairs", "up stairs"),
        _ => phrase_text.to_owned(),
    }
}

fn approve_all(
    store: &TuningCheckpointStore,
    mut checkpoint: TuningCheckpoint,
) -> TuningCheckpoint {
    let row_ids = checkpoint
        .review()
        .rows
        .iter()
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    for row_id in row_ids {
        checkpoint = store
            .record_review_decision(&checkpoint, &row_id, ReviewDecision::Approve)
            .expect("fixture approval must save");
    }
    store
        .continue_review(&checkpoint)
        .expect("approved fixture must enter verification")
}

fn verification_transcript(phrase_id: &str, exercise: bool) -> String {
    let phrase = builtin_corpus()
        .phrase(phrase_id)
        .expect("fixture phrase must remain in the production corpus");
    if !exercise {
        return phrase.verification_text.to_owned();
    }
    match phrase_id {
        "T01" => phrase.verification_text.replace("quick ship", "quick chip"),
        "T05" => phrase.verification_text.replace("Upstairs", "Up stairs"),
        _ => panic!("fixture has no captured mismatch for {phrase_id}"),
    }
}

fn finish_verification(
    store: &TuningCheckpointStore,
    mut checkpoint: TuningCheckpoint,
    dictionary_path: &Path,
    exercise: impl Fn(&str) -> bool,
) -> (crate::tuning_session::CompletedTuningResult, Dictionary) {
    let dictionary = Dictionary::default();
    loop {
        let phrase_id = checkpoint
            .verification_phrase_id()
            .expect("verification fixture must have a current held-out row")
            .to_owned();
        let raw = verification_transcript(&phrase_id, exercise(&phrase_id));
        match store
            .complete_verification_and_commit(
                &checkpoint,
                &raw,
                &dictionary,
                dictionary_path,
                1_725_000_000_000,
            )
            .expect("verification fixture must complete")
        {
            VerificationAdvance::InProgress(next) => checkpoint = *next,
            VerificationAdvance::Complete(result, committed) => return (result, committed),
            VerificationAdvance::ConflictReview(_) => {
                panic!("an unchanged fixture dictionary cannot create a conflict")
            }
        }
    }
}

#[test]
fn ci_full_session_no_candidates_keeps_dictionary_unchanged() {
    let dir = temp_dir("no-candidates");
    let checkpoint_path = dir.join("checkpoint.json");
    let store = TuningCheckpointStore::new(checkpoint_path.clone());
    let checkpoint =
        complete_readings(&store, start_readings(&store), |_, _, text| text.to_owned());

    assert!(checkpoint.review().rows.is_empty());
    let result = store
        .continue_review(&checkpoint)
        .expect("zero-row Review must continue to Result");
    assert_eq!(result.stage(), TuningStage::Result);
    assert_eq!(
        result.unchanged_result_reason(),
        Some(UnchangedResultReason::NoSafeCorrectionsFound)
    );
    assert!(!checkpoint_path.exists());
    assert!(!dir.join("dictionary.json").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ci_full_session_approval_and_verification_commits_one_rule() {
    let dir = temp_dir("one-kept");
    let store = TuningCheckpointStore::new(dir.join("checkpoint.json"));
    let checkpoint = complete_readings(&store, start_readings(&store), |id, _, text| {
        if id == "T01" {
            candidate_transcript(id, text)
        } else {
            text.to_owned()
        }
    });
    assert_eq!(checkpoint.candidate_count(), 1);
    let checkpoint = approve_all(&store, checkpoint);

    let (result, dictionary) =
        finish_verification(&store, checkpoint, &dir.join("dictionary.json"), |_| true);

    assert_eq!(result.rules.len(), 1);
    assert_eq!(result.rules[0].outcome, VerificationRuleOutcome::Kept);
    assert_eq!(dictionary.entries.len(), 1);
    assert_eq!(dictionary.entries[0].from, "quick chip");
    assert_eq!(dictionary.entries[0].to, "quick ship");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ci_full_session_individual_rollback_keeps_unaffected_rule() {
    let dir = temp_dir("partial-success");
    let store = TuningCheckpointStore::new(dir.join("checkpoint.json"));
    let checkpoint = complete_readings(&store, start_readings(&store), |id, _, text| {
        candidate_transcript(id, text)
    });
    assert_eq!(checkpoint.candidate_count(), 2);
    let checkpoint = approve_all(&store, checkpoint);

    let (result, dictionary) = finish_verification(
        &store,
        checkpoint,
        &dir.join("dictionary.json"),
        |phrase_id| phrase_id == "T01",
    );

    assert!(result
        .rules
        .iter()
        .any(|rule| rule.from == "quick chip" && rule.outcome == VerificationRuleOutcome::Kept));
    assert!(result.rules.iter().any(|rule| {
        rule.from == "up stairs" && rule.outcome == VerificationRuleOutcome::CouldNotVerify
    }));
    assert_eq!(dictionary.entries.len(), 1);
    assert_eq!(dictionary.entries[0].from, "quick chip");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ci_full_session_all_rules_rolled_back_keeps_dictionary_unchanged() {
    let dir = temp_dir("all-rolled-back");
    let dictionary_path = dir.join("dictionary.json");
    let store = TuningCheckpointStore::new(dir.join("checkpoint.json"));
    let checkpoint = complete_readings(&store, start_readings(&store), |id, _, text| {
        candidate_transcript(id, text)
    });
    assert_eq!(checkpoint.candidate_count(), 2);
    let checkpoint = approve_all(&store, checkpoint);

    let (result, dictionary) = finish_verification(&store, checkpoint, &dictionary_path, |_| false);

    assert_eq!(result.rules.len(), 2);
    assert!(result
        .rules
        .iter()
        .all(|rule| rule.outcome == VerificationRuleOutcome::CouldNotVerify));
    assert!(dictionary.entries.is_empty());
    assert!(!dictionary_path.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ci_unsafe_candidate_cannot_be_blessed_by_refreshing_expectations() {
    let dir = temp_dir("unsafe-candidate");
    let store = TuningCheckpointStore::new(dir.join("checkpoint.json"));
    let checkpoint = complete_readings(&store, start_readings(&store), |id, pass, text| {
        if id != "T01" {
            return text.to_owned();
        }
        match pass {
            ReadingPass::First => text.replace("quick ship", "quick chip"),
            ReadingPass::Second => text.replace("quick ship", "quick slip"),
        }
    });

    let result = checkpoint
        .inference_results()
        .iter()
        .find(|result| result.phrase_id == "T01")
        .expect("production inference must return a result for every phrase");
    assert_eq!(result.decision, InferenceDecision::Rejected);
    assert!(checkpoint.review().rows.is_empty());
    assert_eq!(checkpoint.candidate_count(), 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ci_recovery_pause_resume_incompatibility_cancellation_and_write_failure() {
    let dir = temp_dir("recovery");
    let checkpoint_path = dir.join("checkpoint.json");
    let store = TuningCheckpointStore::new(checkpoint_path.clone());
    let envelope = CompatibilityEnvelope::current(RecognitionFingerprint::from_stable_id(
        "recovery-fingerprint",
    ));
    let checkpoint = store.start(envelope.clone()).unwrap();
    let interrupted = store.begin_attempt(&checkpoint).unwrap();
    assert!(interrupted.interrupted_attempt());
    assert_eq!(
        store.inspect(&envelope),
        CheckpointState::Compatible(Box::new(interrupted.clone()))
    );
    let resumed = store.discard_in_flight_attempt(&interrupted).unwrap();
    assert!(!resumed.interrupted_attempt());
    assert!(matches!(
        store.inspect(&CompatibilityEnvelope::current(
            RecognitionFingerprint::from_stable_id("changed-fingerprint")
        )),
        CheckpointState::Incompatible { .. }
    ));
    store.cancel(&resumed, false).unwrap();
    assert!(!checkpoint_path.exists());

    let blocked_path = dir.join("blocked-checkpoint");
    fs::create_dir_all(&blocked_path).unwrap();
    let blocked_store = TuningCheckpointStore::new(blocked_path);
    assert!(blocked_store.start(envelope).is_err());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ci_privacy_sentinels_never_enter_diagnostics_export_logs_or_network_paths() {
    const CHILD_ENV: &str = "EAGLESCRIBE_TUNING_PRIVACY_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        run_privacy_sentinel_fixture();
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tuning_ci::ci_privacy_sentinels_never_enter_diagnostics_export_logs_or_network_paths",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .expect("privacy fixture child process must run with captured output");
    assert!(
        output.status.success(),
        "privacy fixture child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captured_output = [output.stdout, output.stderr].concat();
    for sentinel in privacy_sentinels() {
        assert!(
            !captured_output
                .windows(sentinel.len())
                .any(|window| window == sentinel.as_bytes()),
            "free-form stdout/stderr leaked a Tuning privacy sentinel"
        );
    }
}

fn privacy_sentinels() -> [&'static str; 4] {
    [
        std::str::from_utf8(AUDIO_SENTINEL).unwrap(),
        TRANSCRIPT_SENTINEL,
        CORRECTION_SENTINEL,
        DICTIONARY_SENTINEL,
    ]
}

fn run_privacy_sentinel_fixture() {
    let dir = temp_dir("privacy");
    fs::create_dir_all(&dir).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        std::env::set_var(name, &proxy);
    }
    let audio_path = dir.join("discarded-at-capture-boundary.flac");
    fs::write(&audio_path, AUDIO_SENTINEL).unwrap();
    let mut dictionary = Dictionary::default();
    dictionary
        .upsert(DICTIONARY_SENTINEL, "approved replacement")
        .unwrap();
    dictionary.save(&dir.join("dictionary.json")).unwrap();

    let checkpoint_store = TuningCheckpointStore::new(dir.join("checkpoint.json"));
    let checkpoint = complete_readings(
        &checkpoint_store,
        start_readings(&checkpoint_store),
        |id, _, text| match id {
            "T01" => text.replace("quick ship", CORRECTION_SENTINEL),
            "T02" => format!("{text} {TRANSCRIPT_SENTINEL}"),
            _ => text.to_owned(),
        },
    );
    assert_eq!(checkpoint.candidate_count(), 1);
    let checkpoint_json = fs::read_to_string(dir.join("checkpoint.json")).unwrap();
    assert!(checkpoint_json.contains(CORRECTION_SENTINEL));
    assert!(!checkpoint_json.contains(TRANSCRIPT_SENTINEL));

    let diagnostics_path = dir.join("diagnostics.json");
    let export_path = dir.join("export.json");
    let (mut diagnostics, notice) = TuningDiagnosticsStore::open(diagnostics_path.clone(), 1);
    assert!(notice.is_none());
    let session = SessionId::new();
    diagnostics
        .append(
            TuningDiagnosticEvent::new(
                1,
                session,
                EventKind::SessionCompleted,
                TuningStage::Result,
                OutcomeCode::Completed,
            ),
            1,
        )
        .unwrap();
    diagnostics
        .export_to(
            &export_path,
            ExportSelection::all(),
            ExportEnvironment {
                app_version: "0.1.2-ci".into(),
                platform: PlatformCode::Other,
                backend: BackendCode::Cpu,
                model_fingerprint: None,
                configuration_fingerprint: None,
            },
        )
        .unwrap();

    let persisted = fs::read_to_string(diagnostics_path).unwrap();
    let exported = fs::read_to_string(export_path).unwrap();
    for sentinel in privacy_sentinels() {
        assert!(!persisted.contains(sentinel));
        assert!(!exported.contains(sentinel));
    }

    // The deterministic Tuning core has no free-form output or network
    // primitive. This source-level tripwire makes either capability an explicit
    // reviewed change instead of silently exposing fixture content in CI.
    let production_sources = [
        include_str!("tuning.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap(),
        include_str!("tuning_session.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap(),
        include_str!("tuning_diagnostics.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap(),
        include_str!("state.rs")
            .split_once("// TUNING_PRIVACY_BOUNDARY_BEGIN")
            .and_then(|(_, rest)| rest.split_once("// TUNING_PRIVACY_BOUNDARY_END"))
            .map(|(tuning_state, _)| tuning_state)
            .expect("state.rs must retain the explicit Tuning privacy boundary"),
    ]
    .join("\n");
    for forbidden_api in [
        "ureq::",
        "reqwest::",
        "TcpStream",
        "UdpSocket",
        "std::io::stderr",
        "log::",
        "tracing::",
        "println!(",
        "eprintln!(",
        "dbg!(",
    ] {
        assert!(
            !production_sources.contains(forbidden_api),
            "Tuning privacy gate rejected free-form output/network API {forbidden_api}"
        );
    }
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    let _ = fs::remove_dir_all(dir);
}
