//! Deterministic post-STT polish (offline, no LLM).
//!
//! Pipeline when enabled:
//! 1. Normalize whitespace
//! 2. Spoken punctuation commands → symbols
//! 3. Backtrack / self-corrections
//! 4. Filler-word removal
//! 5. Capitalize sentence starts + trailing period when appropriate
//! 6. Collapse whitespace again
//!
//! When disabled (verbatim), only light whitespace normalization is applied.

use regex::Regex;
use std::sync::OnceLock;

/// Result of polishing a raw STT transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolishResult {
    pub raw: String,
    pub polished: String,
    /// True when polished text differs from the light-normalized raw text.
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolishMode {
    /// Full cleanup (default).
    Smart,
    /// Keep STT output as-is aside from whitespace.
    Verbatim,
}

impl Default for PolishMode {
    fn default() -> Self {
        Self::Smart
    }
}

/// Apply polish according to `mode`.
pub fn polish(raw: &str, mode: PolishMode) -> PolishResult {
    let raw_normalized = normalize_whitespace(raw);
    let polished = match mode {
        PolishMode::Verbatim => raw_normalized.clone(),
        PolishMode::Smart => smart_polish(&raw_normalized),
    };

    PolishResult {
        changed: polished != raw_normalized,
        raw: raw_normalized,
        polished,
    }
}

fn smart_polish(input: &str) -> String {
    let mut text = input.to_string();
    // Order matters:
    // 1) Fillers/backtrack while correction cues are still words.
    // 2) Spoken punctuation after backtrack so a leftover "question mark"
    //    in the suffix (when "three question mark" was split) still converts.
    // 3) Capitalize / terminal punct last.
    text = remove_fillers(&text);
    text = apply_backtrack(&text);
    text = apply_spoken_punctuation(&text);
    // Second pass: STT sometimes leaves "question mark" after other rewrites.
    text = apply_spoken_punctuation(&text);
    text = capitalize_sentences(&text);
    text = ensure_terminal_punctuation(&text);
    text = cleanup_double_terminal_punct(&text);
    normalize_whitespace(&text)
}

fn normalize_whitespace(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim().to_string()
}

/// Convert spoken punctuation phrases into symbols (best-effort, English).
fn apply_spoken_punctuation(input: &str) -> String {
    // Order matters: longer phrases first. Include Whisper-ish variants.
    let replacements: &[(&str, &str)] = &[
        (r"(?i)\bnew\s+paragraph\b", "\n\n"),
        (r"(?i)\b(next\s+line|new\s+line|line\s+break)\b", "\n"),
        // question mark — allow hyphen / missing space / trailing comma
        (r"(?i)\bquestion[\s\-]*marks?\b,?", "?"),
        (r"(?i)\bexclamation[\s\-]*(point|mark)s?\b,?", "!"),
        (r"(?i)\b(full\s+stop|period)\b,?", "."),
        (r"(?i)\bcomma\b", ","),
        (r"(?i)\bcolon\b", ":"),
        (r"(?i)\bsemicolon\b", ";"),
        (r"(?i)\bem\s*dash\b", "—"),
        (r"(?i)\b(open\s+paren(thesis)?|left\s+paren(thesis)?)\b", "("),
        (r"(?i)\b(close\s+paren(thesis)?|right\s+paren(thesis)?)\b", ")"),
        (r"(?i)\b(quote|quotation[\s\-]*marks?)\b", "\""),
        (r"(?i)\bellipsis\b", "…"),
    ];

    let mut text = input.to_string();
    for (pat, repl) in replacements {
        let re = Regex::new(pat).expect("valid punctuation regex");
        text = re.replace_all(&text, *repl).into_owned();
    }

    // Tidy spaces around punctuation introduced above.
    let re_space_before = punct_space_before_re();
    text = re_space_before.replace_all(&text, "$1").into_owned();

    let re_space_after = punct_space_after_re();
    text = re_space_after.replace_all(&text, "$1 ").into_owned();

    // Remove space before newlines
    text = text.replace(" \n", "\n");
    normalize_whitespace_preserve_newlines(&text)
}

fn punct_space_before_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Include `?` so "three ?" → "three?"
    RE.get_or_init(|| Regex::new(r#"\s+([,.!?;:…\)"?])"#).expect("regex"))
}

fn punct_space_after_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"([,.!?;:…])([^\s\n,.!?;:…])").expect("regex"))
}

fn normalize_whitespace_preserve_newlines(s: &str) -> String {
    let paragraphs: Vec<String> = s
        .split("\n\n")
        .map(|para| {
            para.lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|p| !p.is_empty())
        .collect();
    paragraphs.join("\n\n")
}

/// Handle self-corrections: "X actually Y", "X scratch that Y", restatements.
fn apply_backtrack(input: &str) -> String {
    let mut text = input.to_string();

    // "… scratch that …" / "… no scratch that …" → keep only after the cue
    // when the cue is mid-utterance with content on both sides.
    let scratch = scratch_that_re();
    while let Some(caps) = scratch.captures(&text) {
        let before = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        let after = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        if after.is_empty() {
            break;
        }
        // Prefer the corrected tail; drop the abandoned prefix.
        let _ = before;
        text = after.to_string();
    }

    // "let's meet at two actually three" / "at 2 actually 3" / "two, actually three"
    // → keep only the corrected value.
    let actually = actually_correction_re();
    text = actually
        .replace_all(&text, |caps: &regex::Captures| {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("").trim_end();
            let old_val = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let new_val = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            let suffix = caps.get(4).map(|m| m.as_str()).unwrap_or("");

            if looks_like_correction_token(old_val) && looks_like_correction_token(new_val) {
                // Drop trailing punct from the abandoned token; keep any terminal
                // punct that was glued to the replacement (e.g. "three?").
                let new_core = strip_edge_punct(new_val);
                let trailing = trailing_sentence_punct(new_val);
                format!(
                    "{}{}{}{}{}",
                    prefix,
                    if prefix.is_empty() { "" } else { " " },
                    new_core,
                    trailing,
                    suffix
                )
                .trim()
                .to_string()
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .into_owned();

    // "as a gift… as a present" style restatement: repeated "as a X / as a Y"
    let restate = restatement_re();
    text = restate
        .replace_all(&text, |caps: &regex::Captures| {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("").trim_end();
            let second = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            format!(
                "{}{}{}",
                prefix,
                if prefix.is_empty() { "" } else { " " },
                second
            )
            .trim()
            .to_string()
        })
        .into_owned();

    text
}

fn scratch_that_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(.*?)\b(?:no\s+)?scratch\s+that\b[,\s]*(.*)$").expect("regex")
    })
}

fn actually_correction_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // prefix, old token, optional comma, actually, new token, suffix
        // Allows Whisper inserting a comma: "two, actually three"
        Regex::new(r"(?i)(.*\s)?(\S+)\s*,?\s*actually\s+(\S+)(\s.*)?$").expect("regex")
    })
}

fn restatement_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // "… as a gift as a present" or with ellipsis/comma between
        Regex::new(r"(?i)(.*?)\bas\s+a\s+(\w+)\s*(?:…|\.\.\.|[,;])?\s*as\s+a\s+(\w+)\b")
            .expect("regex")
    })
}

fn strip_edge_punct(tok: &str) -> String {
    tok.trim_matches(|c: char| {
        matches!(c, ',' | '.' | ';' | '!' | '?' | '"' | '\'' | ':' | '…')
    })
    .to_string()
}

fn trailing_sentence_punct(tok: &str) -> &'static str {
    if tok.ends_with('?') {
        "?"
    } else if tok.ends_with('!') {
        "!"
    } else if tok.ends_with('.') {
        "."
    } else if tok.ends_with('…') {
        "…"
    } else {
        ""
    }
}

/// Tokens that are safe to treat as "X actually Y" substitutions.
/// Includes digits and English number-words so "two actually three" works,
/// while "I actually enjoyed" does not.
fn looks_like_correction_token(tok: &str) -> bool {
    let t = strip_edge_punct(tok).to_ascii_lowercase();
    if t.is_empty() || t.chars().count() > 16 {
        return false;
    }
    if t.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    matches!(
        t.as_str(),
        "am"
            | "pm"
            | "yes"
            | "no"
            | "true"
            | "false"
            | "ok"
            | "okay"
            // Cardinal number-words (common in spoken time corrections)
            | "zero"
            | "oh"
            | "one"
            | "two"
            | "three"
            | "four"
            | "five"
            | "six"
            | "seven"
            | "eight"
            | "nine"
            | "ten"
            | "eleven"
            | "twelve"
            | "thirteen"
            | "fourteen"
            | "fifteen"
            | "sixteen"
            | "seventeen"
            | "eighteen"
            | "nineteen"
            | "twenty"
            | "thirty"
            | "forty"
            | "fifty"
            | "sixty"
            | "half"
            | "quarter"
            | "noon"
            | "midnight"
    )
}

/// Remove common English fillers while preserving intentional uses where possible.
fn remove_fillers(input: &str) -> String {
    // Leading / trailing / mid fillers. Use word boundaries.
    // "like" is risky — only strip as standalone discourse filler between commas
    // or at start: ", like," or "like," at sentence start after a pause pattern.
    // Note: the default `regex` crate has no look-around — keep patterns simple.
    let patterns: &[&str] = &[
        r"(?i)\buh+\b",
        r"(?i)\bum+\b",
        r"(?i)\ber+\b",
        r"(?i)\bah+\b",
        r"(?i)\byou\s+know\b",
        r"(?i)\bi\s+mean\b",
        r"(?i)\bkind\s+of\b",
        r"(?i)\bsort\s+of\b",
        r"(?i)\bbasically\b",
        r"(?i)\bliterally\b",
        r"(?i)\bright\s*,", // trailing "right," as filler
        r"(?i),\s*like\s*,",
        r"(?i)^\s*like\s*,",
        r"(?i)\bwell\s*,",
    ];

    let mut text = input.to_string();
    for pat in patterns {
        let re = Regex::new(pat).expect("filler regex");
        text = re.replace_all(&text, " ").into_owned();
    }

    // Clean double commas / spaces left behind
    let re_commas = Regex::new(r",\s*,+").expect("regex");
    text = re_commas.replace_all(&text, ",").into_owned();
    normalize_whitespace_preserve_newlines(&text)
}

fn capitalize_sentences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut capitalize_next = true;

    for ch in input.chars() {
        if capitalize_next && ch.is_alphabetic() {
            for c in ch.to_uppercase() {
                out.push(c);
            }
            capitalize_next = false;
        } else {
            out.push(ch);
            if matches!(ch, '.' | '!' | '?' | '\n') {
                capitalize_next = true;
            } else if !ch.is_whitespace() {
                // keep capitalize_next as-is for whitespace after punct
            }
        }
    }
    out
}

/// If the text is a non-empty sentence without terminal punctuation, add a period.
fn ensure_terminal_punctuation(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let last_char = trimmed.chars().last().unwrap_or(' ');
    if matches!(last_char, '.' | '!' | '?' | '…' | ':' | '"' | '\'' | ')') {
        return trimmed.to_string();
    }
    if last_char == '\n' {
        return trimmed.to_string();
    }
    format!("{trimmed}.")
}

/// Collapse accidental doubles like `?.` `!.` `..` left by mixed spoken+auto punct.
fn cleanup_double_terminal_punct(input: &str) -> String {
    let mut text = input.to_string();
    // Prefer ? or ! over a trailing period when both appear.
    let re = double_terminal_re();
    text = re
        .replace_all(&text, |caps: &regex::Captures| {
            let chunk = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            if chunk.contains('?') {
                "?".to_string()
            } else if chunk.contains('!') {
                "!".to_string()
            } else if chunk.contains('…') {
                "…".to_string()
            } else {
                ".".to_string()
            }
        })
        .into_owned();
    text
}

fn double_terminal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Match runs of terminal punct at end of string or before closing quotes.
    RE.get_or_init(|| Regex::new(r"[.!?…]{2,}").expect("regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_fillers() {
        let r = polish("um so I think uh we should go", PolishMode::Smart);
        assert!(!r.polished.to_lowercase().contains("um"));
        assert!(!r.polished.to_lowercase().contains("uh"));
        assert!(r.polished.to_lowercase().contains("we should go"));
    }

    #[test]
    fn spoken_punctuation() {
        let r = polish(
            "hello world exclamation point how are you question mark",
            PolishMode::Smart,
        );
        assert!(r.polished.contains('!'), "got: {}", r.polished);
        assert!(r.polished.contains('?'), "got: {}", r.polished);
    }

    #[test]
    fn backtrack_actually_number() {
        let r = polish("let's do coffee at 2 actually 3", PolishMode::Smart);
        assert!(
            r.polished.to_lowercase().contains("3"),
            "got: {}",
            r.polished
        );
        assert!(
            !r.polished.contains("2"),
            "should drop abandoned time, got: {}",
            r.polished
        );
    }

    #[test]
    fn preserves_actually_when_not_correction() {
        let r = polish("I actually enjoyed the movie", PolishMode::Smart);
        assert!(
            r.polished.to_lowercase().contains("actually"),
            "got: {}",
            r.polished
        );
        assert!(r.polished.to_lowercase().contains("enjoyed"));
    }

    #[test]
    fn scratch_that() {
        let r = polish(
            "send it to John scratch that send it to Sarah",
            PolishMode::Smart,
        );
        assert!(
            r.polished.to_lowercase().contains("sarah"),
            "got: {}",
            r.polished
        );
        assert!(
            !r.polished.to_lowercase().contains("john"),
            "got: {}",
            r.polished
        );
    }

    #[test]
    fn verbatim_keeps_fillers() {
        let r = polish("um hello uh world", PolishMode::Verbatim);
        assert!(r.polished.to_lowercase().contains("um"));
        assert!(!r.changed);
    }

    #[test]
    fn capitalizes_and_periods() {
        let r = polish("hello there friend", PolishMode::Smart);
        assert!(r.polished.starts_with('H'), "got: {}", r.polished);
        assert!(r.polished.ends_with('.'), "got: {}", r.polished);
    }

    #[test]
    fn empty_input() {
        let r = polish("   ", PolishMode::Smart);
        assert_eq!(r.polished, "");
    }

    #[test]
    fn user_phrase_number_words_and_question() {
        // Reported failure: "two actually three" kept both; trailing "?."
        let r = polish(
            "um let's meet at two actually three question mark",
            PolishMode::Smart,
        );
        assert_eq!(
            r.polished, "Let's meet at three?",
            "unexpected polish result"
        );
    }

    #[test]
    fn actually_with_whisper_comma() {
        let r = polish(
            "let's meet at two, actually three question mark",
            PolishMode::Smart,
        );
        assert_eq!(r.polished, "Let's meet at three?", "got: {}", r.polished);
    }

    #[test]
    fn question_mark_variants() {
        for raw in [
            "are you there question mark",
            "are you there question-mark",
            "are you there questionmark",
            "are you there question marks",
        ] {
            let r = polish(raw, PolishMode::Smart);
            assert!(
                r.polished.ends_with('?'),
                "input {raw:?} → {}",
                r.polished
            );
            assert!(
                !r.polished.to_lowercase().contains("question"),
                "input {raw:?} → {}",
                r.polished
            );
        }
    }

    /// If STT never emitted "question mark", we cannot invent `?` — only a period.
    #[test]
    fn without_spoken_question_gets_period() {
        let r = polish(
            "um let's meet at two actually three",
            PolishMode::Smart,
        );
        assert_eq!(r.polished, "Let's meet at three.");
    }
}
