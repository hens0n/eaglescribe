//! Deterministic post-STT polish (offline, no LLM).
//!
//! Pipeline when enabled:
//! 1. Filler-word removal
//! 2. Backtrack / self-corrections
//! 3. Spoken punctuation commands → symbols
//! 4. List detection/formatting (`one… two…`, `first… second…`, digits)
//! 5. Capitalize sentence / list-item starts
//! 6. Trailing punctuation cleanup (preserve multi-line lists)
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
    // 3) List formatting (needs markers still as words or digits).
    // 4) Capitalize / terminal punct last — preserve newlines for lists.
    text = remove_fillers(&text);
    text = apply_backtrack(&text);
    text = apply_spoken_punctuation(&text);
    // Second pass: STT sometimes leaves "question mark" after other rewrites.
    text = apply_spoken_punctuation(&text);
    text = format_lists(&text);
    text = capitalize_sentences(&text);
    text = ensure_terminal_punctuation(&text);
    text = cleanup_double_terminal_punct(&text);
    normalize_whitespace_preserve_newlines(&text)
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
    // Numbered / bulleted multi-line lists: leave as-is (no trailing period on the block).
    if looks_like_formatted_list(trimmed) {
        return trimmed.to_string();
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

fn looks_like_formatted_list(s: &str) -> bool {
    let lines: Vec<&str> = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < 2 {
        return false;
    }
    let item_re = list_item_line_re();
    let item_lines = lines.iter().filter(|l| item_re.is_match(l)).count();
    item_lines >= 2
}

fn list_item_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?:\d+\.|[-*•])\s+\S").expect("regex"))
}

// ---------------------------------------------------------------------------
// List detection & formatting
// ---------------------------------------------------------------------------

/// Spoken / typed markers that open a numbered list item.
const CARDINAL_MARKERS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];
const ORDINAL_MARKERS: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth",
    "ninth", "tenth",
];
/// Bullet-style spoken cues → `-` list (not numbered).
const BULLET_MARKERS: &[&str] = &["bullet", "dash", "asterisk", "next item", "next bullet"];

/// Detect spoken lists and rewrite as numbered or bulleted multi-line text.
///
/// Examples:
/// - `goals are one finish the report two send the deck`
///   → intro + `1. Finish the report` / `2. Send the deck`
/// - `first open the app second log in`
/// - `buy milk 1 eggs 2 bread` (digit markers)
/// - `bullet milk bullet eggs` → `- Milk` / `- Eggs`
///
/// Requires **at least two** items. Single uses of "one" / "first" stay prose.
fn format_lists(input: &str) -> String {
    if input.trim().is_empty() {
        return input.to_string();
    }
    // Prefer longer multi-word bullet markers, then ordinals, then cardinals, then digits.
    if let Some(out) = try_format_with_word_markers(input, BULLET_MARKERS, ListStyle::Bullet) {
        return out;
    }
    if let Some(out) = try_format_with_word_markers(input, ORDINAL_MARKERS, ListStyle::Numbered) {
        return out;
    }
    if let Some(out) = try_format_with_word_markers(input, CARDINAL_MARKERS, ListStyle::Numbered) {
        return out;
    }
    if let Some(out) = try_format_digit_markers(input) {
        return out;
    }
    input.to_string()
}

#[derive(Clone, Copy)]
enum ListStyle {
    Numbered,
    Bullet,
}

fn try_format_with_word_markers(
    input: &str,
    markers: &[&str],
    style: ListStyle,
) -> Option<String> {
    // Longest first so "next item" wins over bare words if both were present.
    let mut sorted: Vec<&str> = markers.to_vec();
    sorted.sort_by_key(|m| std::cmp::Reverse(m.len()));
    let alt = sorted
        .iter()
        .map(|m| regex::escape(m))
        .collect::<Vec<_>>()
        .join("|");
    let re = Regex::new(&format!(r"(?i)\b(?:{alt})\b")).ok()?;
    let hits: Vec<_> = re.find_iter(input).collect();
    if hits.len() < 2 {
        return None;
    }
    build_list_from_hits(input, &hits, style)
}

fn try_format_digit_markers(input: &str) -> Option<String> {
    // Standalone 1–20 as list markers (avoid years like 2024 by limiting length).
    let re = digit_list_marker_re();
    let hits: Vec<_> = re.find_iter(input).collect();
    if hits.len() < 2 {
        return None;
    }
    // Prefer ascending or restart-from-1 sequences; still accept appearance order.
    build_list_from_hits(input, &hits, ListStyle::Numbered)
}

fn digit_list_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Word-boundary digits 1–20 only.
    RE.get_or_init(|| Regex::new(r"\b(?:[1-9]|1[0-9]|20)\b").expect("regex"))
}

fn build_list_from_hits(
    input: &str,
    hits: &[regex::Match<'_>],
    style: ListStyle,
) -> Option<String> {
    let mut items: Vec<String> = Vec::new();
    for (i, m) in hits.iter().enumerate() {
        let start = m.end();
        let end = if i + 1 < hits.len() {
            hits[i + 1].start()
        } else {
            input.len()
        };
        let raw_item = input.get(start..end).unwrap_or("").trim();
        let item = clean_list_item(raw_item);
        if !item.is_empty() {
            items.push(capitalize_item(&item));
        }
    }
    if items.len() < 2 {
        return None;
    }

    let mut prefix = input.get(..hits[0].start()).unwrap_or("").trim().to_string();
    // Drop trailing glue words/punct before the list.
    prefix = trim_list_prefix(&prefix);

    let mut out = String::new();
    if !prefix.is_empty() {
        out.push_str(prefix.trim_end_matches(':'));
        out.push_str(":\n");
    }
    for (i, item) in items.iter().enumerate() {
        match style {
            ListStyle::Numbered => {
                out.push_str(&format!("{}. {}\n", i + 1, item));
            }
            ListStyle::Bullet => {
                out.push_str(&format!("- {}\n", item));
            }
        }
    }
    Some(out.trim_end().to_string())
}

fn clean_list_item(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '.' | '—' | '-'))
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_item(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() => {
            let mut out = c.to_uppercase().collect::<String>();
            out.push_str(chars.as_str());
            out
        }
        _ => s.to_string(),
    }
}

fn trim_list_prefix(prefix: &str) -> String {
    let mut p = prefix.trim().to_string();
    // Strip trailing connectors often spoken before lists.
    let trailers = [
        r"(?i)\bare\s*$",
        r"(?i)\bis\s*$",
        r"(?i)\binclude\s*$",
        r"(?i)\bincludes\s*$",
        r"(?i)\bincluding\s*$",
        r"(?i)\blike\s*$",
        r"(?i)\bsuch as\s*$",
        r"(?i)\bas follows\s*$",
        r"(?i)\bthe following\s*$",
        r"[:,\-—]\s*$",
    ];
    for pat in trailers {
        let re = Regex::new(pat).expect("prefix regex");
        p = re.replace(&p, "").into_owned();
        p = p.trim().to_string();
    }
    p
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

    #[test]
    fn formats_cardinal_list() {
        let r = polish(
            "my top goals this week are one finish the report two send the presentation",
            PolishMode::Smart,
        );
        assert!(
            r.polished.contains("1. Finish the report"),
            "got: {}",
            r.polished
        );
        assert!(
            r.polished.contains("2. Send the presentation"),
            "got: {}",
            r.polished
        );
        assert!(
            r.polished.contains('\n'),
            "list should be multi-line, got: {}",
            r.polished
        );
        assert!(
            r.polished.to_lowercase().contains("goals"),
            "keep intro, got: {}",
            r.polished
        );
    }

    #[test]
    fn formats_ordinal_list() {
        let r = polish(
            "steps first open the app second log in third create a project",
            PolishMode::Smart,
        );
        assert!(r.polished.contains("1. Open the app"), "got: {}", r.polished);
        assert!(r.polished.contains("2. Log in"), "got: {}", r.polished);
        assert!(
            r.polished.contains("3. Create a project"),
            "got: {}",
            r.polished
        );
    }

    #[test]
    fn formats_digit_list() {
        let r = polish(
            "shopping list 1 milk 2 eggs 3 bread",
            PolishMode::Smart,
        );
        assert!(r.polished.contains("1. Milk"), "got: {}", r.polished);
        assert!(r.polished.contains("2. Eggs"), "got: {}", r.polished);
        assert!(r.polished.contains("3. Bread"), "got: {}", r.polished);
    }

    #[test]
    fn formats_bullet_list() {
        let r = polish("bullet milk bullet eggs", PolishMode::Smart);
        assert!(r.polished.contains("- Milk"), "got: {}", r.polished);
        assert!(r.polished.contains("- Eggs"), "got: {}", r.polished);
    }

    #[test]
    fn single_number_word_is_not_a_list() {
        let r = polish("I only need one thing from the store", PolishMode::Smart);
        assert!(
            !r.polished.contains("1."),
            "should not listify single marker: {}",
            r.polished
        );
    }

    #[test]
    fn time_correction_is_not_a_list() {
        // Regression: "two actually three" must not become a two-item list.
        let r = polish("meet at two actually three", PolishMode::Smart);
        assert!(
            !r.polished.contains("1."),
            "got: {}",
            r.polished
        );
        assert!(
            r.polished.to_lowercase().contains("three"),
            "got: {}",
            r.polished
        );
    }
}
