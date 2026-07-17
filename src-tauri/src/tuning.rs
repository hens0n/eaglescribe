//! Deterministic built-in Tuning corpus and conservative Candidate Correction inference.

use std::collections::HashSet;
use std::ops::Range;

use serde::{Deserialize, Serialize};

pub const CORPUS_VERSION: &str = "tuning-corpus-v1";
pub const NORMALIZATION_VERSION: &str = "tuning-normalization-v1";
pub const INFERENCE_VERSION: &str = "tuning-inference-v1";
pub const MAX_PROMPT_WORDS: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeSpan {
    pub id: &'static str,
    pub text: &'static str,
    pub token_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuningPhrase {
    pub id: &'static str,
    pub text: &'static str,
    pub eligible_spans: &'static [ProbeSpan],
    pub verification_id: &'static str,
    pub verification_text: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct TuningCorpus {
    pub practice_prompt: &'static str,
    pub phrases: &'static [TuningPhrase],
    pub pass_a: &'static [&'static str],
    pub pass_b: &'static [&'static str],
}

impl TuningCorpus {
    pub fn phrase(&self, phrase_id: &str) -> Option<&'static TuningPhrase> {
        self.phrases.iter().find(|phrase| phrase.id == phrase_id)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_prompt_length("practice", self.practice_prompt)?;
        if self.phrases.is_empty() {
            return Err("Tuning corpus must contain discovery phrases".into());
        }

        let mut phrase_ids = HashSet::new();
        let mut verification_ids = HashSet::new();
        let mut span_ids = HashSet::new();
        let mut discovery_prompts = HashSet::new();
        let mut verification_prompts = HashSet::new();

        for phrase in self.phrases {
            if phrase.id.is_empty() || !phrase_ids.insert(phrase.id) {
                return Err(format!(
                    "Discovery phrase ID {:?} is empty or duplicated",
                    phrase.id
                ));
            }
            if phrase.verification_id.is_empty() || !verification_ids.insert(phrase.verification_id)
            {
                return Err(format!(
                    "Verification phrase ID {:?} is empty or duplicated",
                    phrase.verification_id
                ));
            }
            validate_prompt_length(phrase.id, phrase.text)?;
            validate_prompt_length(phrase.verification_id, phrase.verification_text)?;

            let expected_tokens = normalize_tokens(phrase.text);
            let verification_tokens = normalize_tokens(phrase.verification_text);
            let normalized_discovery = expected_tokens.join(" ");
            let normalized_verification = verification_tokens.join(" ");
            if !discovery_prompts.insert(normalized_discovery) {
                return Err("Normalized discovery prompts must be unique".into());
            }
            if !verification_prompts.insert(normalized_verification) {
                return Err("Normalized verification prompts must be unique".into());
            }
            if phrase.eligible_spans.is_empty() {
                return Err(format!("{} must contain an eligible probe span", phrase.id));
            }

            for span in phrase.eligible_spans {
                if span.id.is_empty() || !span_ids.insert(span.id) {
                    return Err(format!(
                        "Probe span ID {:?} is empty or duplicated",
                        span.id
                    ));
                }
                if span.token_range.is_empty() || span.token_range.end > expected_tokens.len() {
                    return Err(format!("Probe span {} has an invalid token range", span.id));
                }
                let span_tokens = normalize_tokens(span.text);
                if expected_tokens[span.token_range.clone()] != span_tokens {
                    return Err(format!(
                        "Probe span {} does not align to normalized phrase token boundaries",
                        span.id
                    ));
                }
                if !contains_tokens(&verification_tokens, &span_tokens) {
                    return Err(format!(
                        "Verification phrase {} does not contain probe span {}",
                        phrase.verification_id, span.id
                    ));
                }
            }
        }

        if discovery_prompts
            .iter()
            .any(|prompt| verification_prompts.contains(prompt))
        {
            return Err("Discovery and verification prompts must be disjoint".into());
        }
        validate_pass("Pass A", self.pass_a, &phrase_ids)?;
        validate_pass("Pass B", self.pass_b, &phrase_ids)?;
        Ok(())
    }
}

fn validate_prompt_length(id: &str, text: &str) -> Result<(), String> {
    let word_count = normalize_tokens(text).len();
    if word_count == 0 || word_count > MAX_PROMPT_WORDS {
        return Err(format!(
            "Prompt {id} must contain 1..={MAX_PROMPT_WORDS} normalized words"
        ));
    }
    Ok(())
}

fn validate_pass(name: &str, pass: &[&str], phrase_ids: &HashSet<&str>) -> Result<(), String> {
    let ids: HashSet<_> = pass.iter().copied().collect();
    if pass.len() != phrase_ids.len() || ids.len() != pass.len() || ids != *phrase_ids {
        return Err(format!(
            "{name} must contain every discovery phrase exactly once"
        ));
    }
    Ok(())
}

fn contains_tokens(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

static T01_SPANS: [ProbeSpan; 2] = [
    ProbeSpan {
        id: "T01-P01",
        text: "quick ship",
        token_range: 1..3,
    },
    ProbeSpan {
        id: "T01-P02",
        text: "heavy blue boxes",
        token_range: 4..7,
    },
];
static T02_SPANS: [ProbeSpan; 3] = [
    ProbeSpan {
        id: "T02-P01",
        text: "voice",
        token_range: 1..2,
    },
    ProbeSpan {
        id: "T02-P02",
        text: "joyful choice",
        token_range: 4..6,
    },
    ProbeSpan {
        id: "T02-P03",
        text: "easy",
        token_range: 7..8,
    },
];
static T03_SPANS: [ProbeSpan; 3] = [
    ProbeSpan {
        id: "T03-P01",
        text: "Measure",
        token_range: 0..1,
    },
    ProbeSpan {
        id: "T03-P02",
        text: "yellow ring",
        token_range: 2..4,
    },
    ProbeSpan {
        id: "T03-P03",
        text: "order",
        token_range: 6..7,
    },
];
static T04_SPANS: [ProbeSpan; 2] = [
    ProbeSpan {
        id: "T04-P01",
        text: "Three green leaves",
        token_range: 0..3,
    },
    ProbeSpan {
        id: "T04-P02",
        text: "path",
        token_range: 6..7,
    },
];
static T05_SPANS: [ProbeSpan; 2] = [
    ProbeSpan {
        id: "T05-P01",
        text: "good blue book",
        token_range: 3..6,
    },
    ProbeSpan {
        id: "T05-P02",
        text: "upstairs",
        token_range: 6..7,
    },
];
static T06_SPANS: [ProbeSpan; 2] = [
    ProbeSpan {
        id: "T06-P01",
        text: "small talk",
        token_range: 2..4,
    },
    ProbeSpan {
        id: "T06-P02",
        text: "busy office",
        token_range: 6..8,
    },
];
static T07_SPANS: [ProbeSpan; 2] = [
    ProbeSpan {
        id: "T07-P01",
        text: "fresh weather report",
        token_range: 2..5,
    },
    ProbeSpan {
        id: "T07-P02",
        text: "lunch",
        token_range: 6..7,
    },
];
static T08_SPANS: [ProbeSpan; 2] = [
    ProbeSpan {
        id: "T08-P01",
        text: "brown fox",
        token_range: 1..3,
    },
    ProbeSpan {
        id: "T08-P02",
        text: "quiet yard",
        token_range: 5..7,
    },
];
static T09_SPANS: [ProbeSpan; 3] = [
    ProbeSpan {
        id: "T09-P01",
        text: "late train",
        token_range: 1..3,
    },
    ProbeSpan {
        id: "T09-P02",
        text: "reach town",
        token_range: 4..6,
    },
    ProbeSpan {
        id: "T09-P03",
        text: "nine",
        token_range: 7..8,
    },
];
static T10_SPANS: [ProbeSpan; 3] = [
    ProbeSpan {
        id: "T10-P01",
        text: "judge",
        token_range: 1..2,
    },
    ProbeSpan {
        id: "T10-P02",
        text: "chose",
        token_range: 2..3,
    },
    ProbeSpan {
        id: "T10-P03",
        text: "bright orange jacket",
        token_range: 4..7,
    },
];

static PHRASES: [TuningPhrase; 10] = [
    TuningPhrase {
        id: "T01",
        text: "That quick ship carries heavy blue boxes.",
        eligible_spans: &T01_SPANS,
        verification_id: "V01",
        verification_text: "The heavy blue boxes arrived on a quick ship.",
    },
    TuningPhrase {
        id: "T02",
        text: "Your voice made the joyful choice sound easy.",
        eligible_spans: &T02_SPANS,
        verification_id: "V02",
        verification_text: "The joyful choice was easy to explain in her voice.",
    },
    TuningPhrase {
        id: "T03",
        text: "Measure the yellow ring before you order it.",
        eligible_spans: &T03_SPANS,
        verification_id: "V03",
        verification_text: "Before the order ships, measure the yellow ring again.",
    },
    TuningPhrase {
        id: "T04",
        text: "Three green leaves fell beside the path.",
        eligible_spans: &T04_SPANS,
        verification_id: "V04",
        verification_text: "Three green leaves covered the garden path.",
    },
    TuningPhrase {
        id: "T05",
        text: "She found a good blue book upstairs.",
        eligible_spans: &T05_SPANS,
        verification_id: "V05",
        verification_text: "Upstairs, the good blue book remains on the desk.",
    },
    TuningPhrase {
        id: "T06",
        text: "We made small talk near the busy office.",
        eligible_spans: &T06_SPANS,
        verification_id: "V06",
        verification_text: "After small talk, the busy office fell quiet.",
    },
    TuningPhrase {
        id: "T07",
        text: "Check the fresh weather report before lunch.",
        eligible_spans: &T07_SPANS,
        verification_id: "V07",
        verification_text: "During lunch, we checked the fresh weather report.",
    },
    TuningPhrase {
        id: "T08",
        text: "A brown fox crossed the quiet yard.",
        eligible_spans: &T08_SPANS,
        verification_id: "V08",
        verification_text: "The brown fox left the quiet yard before dawn.",
    },
    TuningPhrase {
        id: "T09",
        text: "The late train should reach town by nine.",
        eligible_spans: &T09_SPANS,
        verification_id: "V09",
        verification_text: "By nine, the late train should reach town.",
    },
    TuningPhrase {
        id: "T10",
        text: "The judge chose a bright orange jacket.",
        eligible_spans: &T10_SPANS,
        verification_id: "V10",
        verification_text: "The bright orange jacket pleased the judge who chose it.",
    },
];

static PASS_A: [&str; 10] = [
    "T01", "T02", "T03", "T04", "T05", "T06", "T07", "T08", "T09", "T10",
];
static PASS_B: [&str; 10] = [
    "T06", "T07", "T08", "T09", "T10", "T01", "T02", "T03", "T04", "T05",
];

static BUILTIN_CORPUS: TuningCorpus = TuningCorpus {
    practice_prompt: "Today is a good day to try voice typing.",
    phrases: &PHRASES,
    pass_a: &PASS_A,
    pass_b: &PASS_B,
};

pub fn builtin_corpus() -> &'static TuningCorpus {
    &BUILTIN_CORPUS
}

pub fn normalize_text(text: &str) -> String {
    normalize_tokens(text).join(" ")
}

fn normalize_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();

    for (index, character) in chars.iter().copied().enumerate() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if matches!(character, '\'' | '\u{2018}' | '\u{2019}')
            && index > 0
            && index + 1 < chars.len()
            && chars[index - 1].is_alphanumeric()
            && chars[index + 1].is_alphanumeric()
        {
            current.push('\'');
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    NoMismatch,
    MissingContext,
    InsertionOrDeletion,
    MultipleHunks,
    OutsideEligibleSpan,
    SpanMappingFailed,
    SingleWordSource,
    ReadingsDisagree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateCorrection {
    pub probe_span_id: String,
    pub from: String,
    pub to: String,
    pub state: CandidateState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceDecision {
    Candidate(CandidateCorrection),
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingResult {
    /// Minimal normalized expected range left after exact prefix/suffix alignment.
    pub expected_range: Range<usize>,
    /// Minimal normalized observed range left after exact prefix/suffix alignment.
    pub observed_range: Range<usize>,
    /// Full eligible probe range, when alignment reached a unique probe.
    pub expanded_expected_range: Option<Range<usize>>,
    /// Contiguous observed range mapped to the full eligible probe.
    pub expanded_observed_range: Option<Range<usize>>,
    pub probe_span_id: Option<String>,
    pub normalized_source: Option<String>,
    pub normalized_target: Option<String>,
    pub reason_codes: Vec<ReasonCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceResult {
    pub phrase_id: String,
    pub decision: InferenceDecision,
    pub reading_results: [ReadingResult; 2],
    pub aggregate_reason_codes: Vec<ReasonCode>,
}

/// Infer one inactive Candidate Correction from two separated raw readings.
///
/// This is a pure, deterministic operation: it performs no I/O and retains no
/// audio or transcript after the caller drops the returned derived result.
pub fn infer_candidate_correction(
    phrase: &TuningPhrase,
    first_raw_reading: &str,
    second_raw_reading: &str,
) -> InferenceResult {
    let expected = normalize_tokens(phrase.text);
    let first = align_reading(phrase, &expected, first_raw_reading);
    let second = align_reading(phrase, &expected, second_raw_reading);
    let reading_results = [first, second];

    let first_mapping = qualifying_mapping(&reading_results[0]);
    let second_mapping = qualifying_mapping(&reading_results[1]);
    let (decision, aggregate_reason_codes) = match (first_mapping, second_mapping) {
        (Some(first), Some(second)) if first == second => (
            InferenceDecision::Candidate(CandidateCorrection {
                probe_span_id: first.0.to_owned(),
                from: first.1.to_owned(),
                to: first.2.to_owned(),
                state: CandidateState::Inactive,
            }),
            Vec::new(),
        ),
        (Some(_), Some(_)) => (
            InferenceDecision::Rejected,
            vec![ReasonCode::ReadingsDisagree],
        ),
        _ => {
            let mut reasons = Vec::new();
            for reading in &reading_results {
                for reason in &reading.reason_codes {
                    if !reasons.contains(reason) {
                        reasons.push(*reason);
                    }
                }
            }
            (InferenceDecision::Rejected, reasons)
        }
    };

    InferenceResult {
        phrase_id: phrase.id.to_owned(),
        decision,
        reading_results,
        aggregate_reason_codes,
    }
}

fn qualifying_mapping(reading: &ReadingResult) -> Option<(&str, &str, &str)> {
    if !reading.reason_codes.is_empty() {
        return None;
    }
    Some((
        reading.probe_span_id.as_deref()?,
        reading.normalized_source.as_deref()?,
        reading.normalized_target.as_deref()?,
    ))
}

fn align_reading(phrase: &TuningPhrase, expected: &[String], raw_reading: &str) -> ReadingResult {
    let observed = normalize_tokens(raw_reading);
    let prefix_len = expected
        .iter()
        .zip(&observed)
        .take_while(|(left, right)| left == right)
        .count();

    let mut suffix_len = 0;
    while prefix_len + suffix_len < expected.len()
        && prefix_len + suffix_len < observed.len()
        && expected[expected.len() - 1 - suffix_len] == observed[observed.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    let expected_range = prefix_len..expected.len() - suffix_len;
    let observed_range = prefix_len..observed.len() - suffix_len;
    let base = || ReadingResult {
        expected_range: expected_range.clone(),
        observed_range: observed_range.clone(),
        expanded_expected_range: None,
        expanded_observed_range: None,
        probe_span_id: None,
        normalized_source: None,
        normalized_target: None,
        reason_codes: Vec::new(),
    };

    if expected_range.is_empty() && observed_range.is_empty() {
        return rejected_reading(base(), ReasonCode::NoMismatch);
    }
    if expected_range.is_empty() || observed_range.is_empty() {
        return rejected_reading(base(), ReasonCode::InsertionOrDeletion);
    }
    if prefix_len == 0 && suffix_len == 0 {
        return rejected_reading(base(), ReasonCode::MissingContext);
    }

    let expected_residual: HashSet<_> = expected[expected_range.clone()].iter().collect();
    if observed[observed_range.clone()]
        .iter()
        .any(|token| expected_residual.contains(token))
    {
        return rejected_reading(base(), ReasonCode::MultipleHunks);
    }

    let eligible: Vec<_> = phrase
        .eligible_spans
        .iter()
        .filter(|span| {
            span.token_range.start <= expected_range.start
                && expected_range.end <= span.token_range.end
        })
        .collect();
    if eligible.len() != 1 {
        return rejected_reading(base(), ReasonCode::OutsideEligibleSpan);
    }
    let span = eligible[0];

    let left_expansion = expected_range.start - span.token_range.start;
    let right_expansion = span.token_range.end - expected_range.end;
    let Some(observed_start) = observed_range.start.checked_sub(left_expansion) else {
        return rejected_reading(base(), ReasonCode::SpanMappingFailed);
    };
    let Some(observed_end) = observed_range.end.checked_add(right_expansion) else {
        return rejected_reading(base(), ReasonCode::SpanMappingFailed);
    };
    if observed_start > observed_end || observed_end > observed.len() {
        return rejected_reading(base(), ReasonCode::SpanMappingFailed);
    }

    let source_tokens = &observed[observed_start..observed_end];
    let target_tokens = &expected[span.token_range.clone()];
    let mut result = base();
    result.expanded_expected_range = Some(span.token_range.clone());
    result.expanded_observed_range = Some(observed_start..observed_end);
    result.probe_span_id = Some(span.id.to_owned());
    result.normalized_source = Some(source_tokens.join(" "));
    result.normalized_target = Some(target_tokens.join(" "));
    if source_tokens.len() < 2 {
        result.reason_codes.push(ReasonCode::SingleWordSource);
    }
    result
}

fn rejected_reading(mut result: ReadingResult, reason: ReasonCode) -> ReadingResult {
    result.reason_codes.push(reason);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const CMUDICT_REVISION: &str = "74790861f652b15e4ac49015a90074ad62a27690";
    const CMUDICT_TUNING_SUBSET: &str = include_str!("../testdata/cmudict-tuning-74790861.dict");

    fn infer(phrase_id: &str, first: &str, second: &str) -> InferenceResult {
        let phrase = builtin_corpus().phrase(phrase_id).expect("fixture phrase");
        infer_candidate_correction(phrase, first, second)
    }

    fn assert_rejected_reading(
        reading: &ReadingResult,
        reason: ReasonCode,
        expected_range: Range<usize>,
        observed_range: Range<usize>,
    ) {
        assert_eq!(reading.expected_range, expected_range);
        assert_eq!(reading.observed_range, observed_range);
        assert_eq!(reading.reason_codes, [reason]);
    }

    fn assert_no_derived_mapping(reading: &ReadingResult) {
        assert_eq!(reading.expanded_expected_range, None);
        assert_eq!(reading.expanded_observed_range, None);
        assert_eq!(reading.probe_span_id, None);
        assert_eq!(reading.normalized_source, None);
        assert_eq!(reading.normalized_target, None);
    }

    fn assert_qualifying_quick_chip(reading: &ReadingResult) {
        assert_eq!(reading.expected_range, 2..3);
        assert_eq!(reading.observed_range, 2..3);
        assert_eq!(reading.expanded_expected_range, Some(1..3));
        assert_eq!(reading.expanded_observed_range, Some(1..3));
        assert_eq!(reading.probe_span_id.as_deref(), Some("T01-P01"));
        assert_eq!(reading.normalized_source.as_deref(), Some("quick chip"));
        assert_eq!(reading.normalized_target.as_deref(), Some("quick ship"));
        assert!(reading.reason_codes.is_empty());
    }

    #[test]
    fn builtin_corpus_is_valid_and_complete() {
        let corpus = builtin_corpus();
        corpus.validate().expect("built-in corpus should be valid");

        assert_eq!(
            corpus.practice_prompt,
            "Today is a good day to try voice typing."
        );
        assert_eq!(corpus.phrases.len(), 10);
        let exact_rows = [
            (
                "T01",
                "That quick ship carries heavy blue boxes.",
                "V01",
                "The heavy blue boxes arrived on a quick ship.",
            ),
            (
                "T02",
                "Your voice made the joyful choice sound easy.",
                "V02",
                "The joyful choice was easy to explain in her voice.",
            ),
            (
                "T03",
                "Measure the yellow ring before you order it.",
                "V03",
                "Before the order ships, measure the yellow ring again.",
            ),
            (
                "T04",
                "Three green leaves fell beside the path.",
                "V04",
                "Three green leaves covered the garden path.",
            ),
            (
                "T05",
                "She found a good blue book upstairs.",
                "V05",
                "Upstairs, the good blue book remains on the desk.",
            ),
            (
                "T06",
                "We made small talk near the busy office.",
                "V06",
                "After small talk, the busy office fell quiet.",
            ),
            (
                "T07",
                "Check the fresh weather report before lunch.",
                "V07",
                "During lunch, we checked the fresh weather report.",
            ),
            (
                "T08",
                "A brown fox crossed the quiet yard.",
                "V08",
                "The brown fox left the quiet yard before dawn.",
            ),
            (
                "T09",
                "The late train should reach town by nine.",
                "V09",
                "By nine, the late train should reach town.",
            ),
            (
                "T10",
                "The judge chose a bright orange jacket.",
                "V10",
                "The bright orange jacket pleased the judge who chose it.",
            ),
        ];
        for (phrase, expected) in corpus.phrases.iter().zip(exact_rows) {
            assert_eq!(
                (
                    phrase.id,
                    phrase.text,
                    phrase.verification_id,
                    phrase.verification_text
                ),
                expected
            );
            assert!(!phrase.eligible_spans.is_empty());
        }
        assert_eq!(
            corpus.pass_a,
            ["T01", "T02", "T03", "T04", "T05", "T06", "T07", "T08", "T09", "T10"]
        );
        assert_eq!(
            corpus.pass_b,
            ["T06", "T07", "T08", "T09", "T10", "T01", "T02", "T03", "T04", "T05"]
        );
    }

    #[test]
    fn corpus_validation_rejects_unstable_or_unsafe_data() {
        static BAD_BOUNDARY_SPANS: [ProbeSpan; 1] = [ProbeSpan {
            id: "P01",
            text: "alpha beta",
            token_range: 1..2,
        }];
        static BAD_BOUNDARY_PHRASES: [TuningPhrase; 1] = [TuningPhrase {
            id: "T01",
            text: "alpha beta gamma",
            eligible_spans: &BAD_BOUNDARY_SPANS,
            verification_id: "V01",
            verification_text: "alpha beta elsewhere",
        }];
        static ONE_PASS: [&str; 1] = ["T01"];
        let bad_boundary = TuningCorpus {
            practice_prompt: "practice prompt",
            phrases: &BAD_BOUNDARY_PHRASES,
            pass_a: &ONE_PASS,
            pass_b: &ONE_PASS,
        };
        assert!(bad_boundary
            .validate()
            .expect_err("misaligned probe text must fail")
            .contains("token boundaries"));

        static DISJOINT_SPANS: [ProbeSpan; 1] = [ProbeSpan {
            id: "P02",
            text: "alpha beta",
            token_range: 0..2,
        }];
        static DISJOINT_PHRASES: [TuningPhrase; 1] = [TuningPhrase {
            id: "T01",
            text: "alpha beta gamma",
            eligible_spans: &DISJOINT_SPANS,
            verification_id: "V01",
            verification_text: "alpha beta gamma",
        }];
        let overlapping_prompts = TuningCorpus {
            practice_prompt: "practice prompt",
            phrases: &DISJOINT_PHRASES,
            pass_a: &ONE_PASS,
            pass_b: &ONE_PASS,
        };
        assert!(overlapping_prompts
            .validate()
            .expect_err("discovery and verification strings must differ")
            .contains("disjoint"));

        static MISSING_SPAN_PHRASES: [TuningPhrase; 1] = [TuningPhrase {
            id: "T01",
            text: "alpha beta gamma",
            eligible_spans: &DISJOINT_SPANS,
            verification_id: "V01",
            verification_text: "unrelated held out words",
        }];
        let missing_span = TuningCorpus {
            practice_prompt: "practice prompt",
            phrases: &MISSING_SPAN_PHRASES,
            pass_a: &ONE_PASS,
            pass_b: &ONE_PASS,
        };
        assert!(missing_span
            .validate()
            .expect_err("held-out phrase must contain the probe")
            .contains("does not contain probe span"));

        let too_long = TuningCorpus {
            practice_prompt: "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen",
            phrases: &DISJOINT_PHRASES,
            pass_a: &ONE_PASS,
            pass_b: &ONE_PASS,
        };
        assert!(too_long
            .validate()
            .expect_err("15-word prompt must exceed the limit")
            .contains("1..=14"));

        static DUPLICATE_SPANS: [ProbeSpan; 2] = [
            ProbeSpan {
                id: "P03",
                text: "alpha beta",
                token_range: 0..2,
            },
            ProbeSpan {
                id: "P03",
                text: "gamma",
                token_range: 2..3,
            },
        ];
        static DUPLICATE_PHRASES: [TuningPhrase; 1] = [TuningPhrase {
            id: "T01",
            text: "alpha beta gamma",
            eligible_spans: &DUPLICATE_SPANS,
            verification_id: "V01",
            verification_text: "alpha beta gamma elsewhere",
        }];
        let duplicate_ids = TuningCorpus {
            practice_prompt: "practice prompt",
            phrases: &DUPLICATE_PHRASES,
            pass_a: &ONE_PASS,
            pass_b: &ONE_PASS,
        };
        assert!(duplicate_ids
            .validate()
            .expect_err("probe IDs must be stable and unique")
            .contains("duplicated"));
    }

    #[test]
    fn pinned_cmudict_subset_knows_every_corpus_word_and_reports_phone_coverage() {
        assert!(CMUDICT_TUNING_SUBSET.contains(CMUDICT_REVISION));
        let pronunciations: HashMap<_, Vec<_>> = CMUDICT_TUNING_SUBSET
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| {
                let mut fields = line.split_ascii_whitespace();
                let word = fields.next().expect("CMUdict word");
                (word, fields.collect())
            })
            .collect();

        let corpus = builtin_corpus();
        let mut corpus_words = HashSet::new();
        corpus_words.extend(normalize_tokens(corpus.practice_prompt));
        for phrase in corpus.phrases {
            corpus_words.extend(normalize_tokens(phrase.text));
            corpus_words.extend(normalize_tokens(phrase.verification_text));
        }

        let mut unknown_words: Vec<_> = corpus_words
            .iter()
            .filter(|word| !pronunciations.contains_key(word.as_str()))
            .cloned()
            .collect();
        unknown_words.sort();
        assert!(
            unknown_words.is_empty(),
            "corpus words absent from pinned CMUdict revision {CMUDICT_REVISION}: {unknown_words:?}"
        );

        let mut base_phones = HashSet::new();
        for word in corpus_words {
            for phone in &pronunciations[word.as_str()] {
                base_phones.insert(phone.trim_end_matches(|c: char| c.is_ascii_digit()));
            }
        }
        let mut base_phones: Vec<_> = base_phones.into_iter().collect();
        base_phones.sort();
        eprintln!(
            "CMUdict {CMUDICT_REVISION} corpus diagnostic: {} base phones: {}",
            base_phones.len(),
            base_phones.join(" ")
        );
    }

    #[test]
    fn normalization_uses_exact_words_and_canonical_apostrophes() {
        assert_eq!(
            normalize_text("  DON’T re-enter rock'n'roll—today! "),
            "don't re enter rock'n'roll today"
        );
        assert_ne!(normalize_text("résumé"), normalize_text("resume"));
        assert_ne!(normalize_text("ships"), normalize_text("ship"));
    }

    #[test]
    fn repeated_safe_mismatch_proposes_inactive_context_bearing_candidate() {
        let result = infer(
            "T01",
            "That quick chip carries heavy blue boxes",
            "That quick chip carries heavy blue boxes",
        );

        let InferenceDecision::Candidate(candidate) = result.decision else {
            panic!("expected candidate: {result:?}");
        };
        assert_eq!(candidate.probe_span_id, "T01-P01");
        assert_eq!(candidate.from, "quick chip");
        assert_eq!(candidate.to, "quick ship");
        assert_eq!(candidate.state, CandidateState::Inactive);
        assert!(result.aggregate_reason_codes.is_empty());
        for reading in result.reading_results {
            assert_eq!(reading.expected_range, 2..3);
            assert_eq!(reading.observed_range, 2..3);
            assert_eq!(reading.expanded_expected_range, Some(1..3));
            assert_eq!(reading.expanded_observed_range, Some(1..3));
            assert_eq!(reading.probe_span_id.as_deref(), Some("T01-P01"));
            assert_eq!(reading.normalized_source.as_deref(), Some("quick chip"));
            assert_eq!(reading.normalized_target.as_deref(), Some("quick ship"));
            assert!(reading.reason_codes.is_empty());
        }
    }

    #[test]
    fn single_word_expanded_source_is_rejected() {
        let result = infer(
            "T02",
            "Your boys made the joyful choice sound easy",
            "Your boys made the joyful choice sound easy",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(
            result.aggregate_reason_codes,
            [ReasonCode::SingleWordSource]
        );
        for reading in result.reading_results {
            assert_rejected_reading(&reading, ReasonCode::SingleWordSource, 1..2, 1..2);
            assert_eq!(reading.expanded_expected_range, Some(1..2));
            assert_eq!(reading.expanded_observed_range, Some(1..2));
            assert_eq!(reading.probe_span_id.as_deref(), Some("T02-P01"));
            assert_eq!(reading.normalized_source.as_deref(), Some("boys"));
            assert_eq!(reading.normalized_target.as_deref(), Some("voice"));
        }
    }

    #[test]
    fn otherwise_safe_readings_must_agree_exactly() {
        let result = infer(
            "T01",
            "That quick chip carries heavy blue boxes",
            "That quick sheep carries heavy blue boxes",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(
            result.aggregate_reason_codes,
            [ReasonCode::ReadingsDisagree]
        );
        assert_eq!(
            result.reading_results[0].normalized_source.as_deref(),
            Some("quick chip")
        );
        assert_eq!(
            result.reading_results[1].normalized_source.as_deref(),
            Some("quick sheep")
        );
        for reading in &result.reading_results {
            assert_eq!(reading.expected_range, 2..3);
            assert_eq!(reading.observed_range, 2..3);
            assert_eq!(reading.expanded_expected_range, Some(1..3));
            assert_eq!(reading.expanded_observed_range, Some(1..3));
            assert_eq!(reading.probe_span_id.as_deref(), Some("T01-P01"));
            assert_eq!(reading.normalized_target.as_deref(), Some("quick ship"));
            assert!(reading.reason_codes.is_empty());
        }
    }

    #[test]
    fn mismatch_outside_a_probe_is_rejected() {
        let result = infer(
            "T01",
            "That quick ship carried heavy blue boxes",
            "That quick ship carried heavy blue boxes",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(
            result.aggregate_reason_codes,
            [ReasonCode::OutsideEligibleSpan]
        );
        for reading in result.reading_results {
            assert_rejected_reading(&reading, ReasonCode::OutsideEligibleSpan, 3..4, 3..4);
            assert_no_derived_mapping(&reading);
        }
    }

    #[test]
    fn pure_omission_is_not_a_substitution() {
        let result = infer(
            "T01",
            "That quick carries heavy blue boxes",
            "That quick carries heavy blue boxes",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(
            result.aggregate_reason_codes,
            [ReasonCode::InsertionOrDeletion]
        );
        for reading in result.reading_results {
            assert_rejected_reading(&reading, ReasonCode::InsertionOrDeletion, 2..3, 2..2);
            assert_no_derived_mapping(&reading);
        }
    }

    #[test]
    fn separated_edits_with_a_shared_internal_token_are_rejected() {
        let result = infer(
            "T01",
            "That slow ship hauls heavy blue boxes",
            "That slow ship hauls heavy blue boxes",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(result.aggregate_reason_codes, [ReasonCode::MultipleHunks]);
        for reading in result.reading_results {
            assert_rejected_reading(&reading, ReasonCode::MultipleHunks, 1..4, 1..4);
            assert_no_derived_mapping(&reading);
        }
    }

    #[test]
    fn stable_segmentation_mismatch_can_produce_a_candidate() {
        let result = infer(
            "T05",
            "She found a good blue book up stairs",
            "She found a good blue book up stairs",
        );

        let InferenceDecision::Candidate(candidate) = result.decision else {
            panic!("expected candidate: {result:?}");
        };
        assert_eq!(candidate.probe_span_id, "T05-P02");
        assert_eq!(candidate.from, "up stairs");
        assert_eq!(candidate.to, "upstairs");
        for reading in result.reading_results {
            assert_eq!(reading.expected_range, 6..7);
            assert_eq!(reading.observed_range, 6..8);
            assert_eq!(reading.expanded_expected_range, Some(6..7));
            assert_eq!(reading.expanded_observed_range, Some(6..8));
            assert_eq!(reading.probe_span_id.as_deref(), Some("T05-P02"));
            assert_eq!(reading.normalized_source.as_deref(), Some("up stairs"));
            assert_eq!(reading.normalized_target.as_deref(), Some("upstairs"));
            assert!(reading.reason_codes.is_empty());
        }
    }

    #[test]
    fn formatting_only_differences_are_not_mismatches() {
        let result = infer(
            "T01",
            " THAT quick-ship carries heavy blue boxes!!! ",
            "that   QUICK SHIP carries, heavy blue boxes.",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(result.aggregate_reason_codes, [ReasonCode::NoMismatch]);
        for reading in result.reading_results {
            assert_rejected_reading(&reading, ReasonCode::NoMismatch, 7..7, 7..7);
            assert_no_derived_mapping(&reading);
        }
    }

    #[test]
    fn unanchored_whole_phrase_change_is_rejected() {
        let result = infer(
            "T01",
            "Completely different observed words here",
            "Completely different observed words here",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(result.aggregate_reason_codes, [ReasonCode::MissingContext]);
        for reading in result.reading_results {
            assert_rejected_reading(&reading, ReasonCode::MissingContext, 0..7, 0..5);
            assert_no_derived_mapping(&reading);
        }
    }

    #[test]
    fn qualifying_reading_paired_with_exact_reading_is_rejected() {
        let result = infer(
            "T01",
            "That quick chip carries heavy blue boxes",
            "That quick ship carries heavy blue boxes",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(result.aggregate_reason_codes, [ReasonCode::NoMismatch]);
        assert_qualifying_quick_chip(&result.reading_results[0]);
        assert_rejected_reading(
            &result.reading_results[1],
            ReasonCode::NoMismatch,
            7..7,
            7..7,
        );
        assert_no_derived_mapping(&result.reading_results[1]);
    }

    #[test]
    fn qualifying_reading_paired_with_unsafe_reading_is_rejected() {
        let result = infer(
            "T01",
            "That quick chip carries heavy blue boxes",
            "That quick ship carried heavy blue boxes",
        );

        assert_eq!(result.decision, InferenceDecision::Rejected);
        assert_eq!(
            result.aggregate_reason_codes,
            [ReasonCode::OutsideEligibleSpan]
        );
        assert_qualifying_quick_chip(&result.reading_results[0]);
        assert_rejected_reading(
            &result.reading_results[1],
            ReasonCode::OutsideEligibleSpan,
            3..4,
            3..4,
        );
        assert_no_derived_mapping(&result.reading_results[1]);
    }

    #[test]
    fn reason_codes_have_stable_machine_readable_names() {
        let encoded = serde_json::to_value([
            ReasonCode::NoMismatch,
            ReasonCode::MissingContext,
            ReasonCode::InsertionOrDeletion,
            ReasonCode::MultipleHunks,
            ReasonCode::OutsideEligibleSpan,
            ReasonCode::SpanMappingFailed,
            ReasonCode::SingleWordSource,
            ReasonCode::ReadingsDisagree,
        ])
        .expect("serialize reason codes");
        assert_eq!(
            encoded,
            serde_json::json!([
                "no_mismatch",
                "missing_context",
                "insertion_or_deletion",
                "multiple_hunks",
                "outside_eligible_span",
                "span_mapping_failed",
                "single_word_source",
                "readings_disagree"
            ])
        );
    }
}
