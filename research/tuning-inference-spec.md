# Safe Candidate Correction inference

**Status:** Behavior specification  
**Date:** 2026-07-16  
**Related:** [Define safe Candidate Correction inference](https://github.com/hens0n/eaglescribe/issues/26) · [Design guided user-specific Tuning](https://github.com/hens0n/eaglescribe/issues/25)

## Decision

EagleScribe may propose at most one Candidate Correction from a Tuning Phrase. It does so only when both raw Whisper readings independently produce the same conservative, context-bearing mapping inside the same candidate-eligible probe span.

Inference uses exact normalized word boundaries and a single-hunk prefix/suffix alignment. It does not use edit distance, phonetic similarity, stemming, fuzzy spelling, language-model judgment, or tie-breaking heuristics. Ambiguous and structurally noisy evidence remains available as a local diagnostic result but cannot create a Candidate Correction.

The output is only an inactive Candidate Correction. Approval, Personal Dictionary conflicts, Verification Pass scoring, activation, and rollback are separate decisions.

## Inputs

Inference receives:

- The stable Tuning Phrase ID.
- The exact expected phrase text.
- Stable IDs and exact token ranges for that phrase's candidate-eligible probe spans.
- Two raw Whisper transcripts from the phrase's separated readings.

Probe spans are corpus data, not rediscovered from display markup at runtime. Corpus validation must guarantee that every span is nonempty, lies on normalized token boundaries, and has a unique stable identity within its phrase.

## Normalization

Normalize the expected phrase, probe spans, and both transcripts identically before alignment:

1. Apply Unicode lowercase without accent or phonetic folding.
2. Tokenize alphanumeric word content.
3. Canonicalize straight and curly apostrophes occurring inside a word to ASCII `'`.
4. Treat whitespace, hyphens, and all other punctuation as token separators, then join tokens with one ASCII space.

Consequences:

- Case, spacing, terminal punctuation, and typographic apostrophe differences cannot manufacture a mismatch.
- A hyphenated and space-separated form have the same token sequence.
- Stemming, synonyms, fuzzy spelling, and phonetic similarity never make two tokens equal.
- Candidate `from` and `to` phrases are stored in normalized lowercase. Existing Personal Dictionary application behavior remains responsible for adapting output casing at a match site.

If normalization makes both readings equal to the expected phrase, inference returns `no_mismatch` and no Candidate Correction.

## Per-reading alignment

Align each reading independently using this deterministic procedure:

1. Consume the longest exact token prefix shared by expected and observed text.
2. Consume the longest exact token suffix that does not overlap that prefix.
3. Treat the remaining expected and observed ranges as the only possible mismatch hunk.

A reading qualifies only when all of the following are true:

- The expected and observed mismatch ranges are both nonempty. A pure insertion or deletion is not a substitution.
- At least one exact context token remains. A sentence boundary may anchor one side, but the other side must contain exact matching context.
- The two mismatch ranges share no token. A shared internal token proves that the residual contains multiple separated edits rather than one isolated hunk.
- The complete expected mismatch range lies inside exactly one candidate-eligible probe span.
- Expanding the mismatch to that full probe span maps to one contiguous observed range without crossing phrase boundaries.
- The expanded observed source contains at least two normalized words.

The expanded observed range becomes `from`; the full expected probe span becomes `to`. Expansion is mandatory even when the minimal mismatch is one word. For example, the stable minimal mismatch `chip → ship` inside the `quick ship` probe produces `quick chip → quick ship`, not a global `chip → ship` rule.

The source and target may contain different word counts as long as both mismatch ranges are nonempty. This permits stable segmentation mappings such as `up stairs → upstairs`; it does not permit a pure missing or extra word to become a rule.

## Agreement across readings

After independent alignment, propose a Candidate Correction only when:

- Both readings qualify.
- Their normalized expanded `from` values are exactly equal.
- Their normalized expanded `to` values are exactly equal.
- Both mappings point to the same probe-span ID.

There is no fuzzy agreement, majority rule, or merging of evidence. If one reading is exact, unsafe, or produces a different source mapping, the phrase produces no candidate. Because each qualifying reading contains only one mismatch hunk, a Tuning Phrase can produce at most one Candidate Correction.

## Result and reason codes

Inference returns a structured local result rather than a bare optional candidate:

```text
InferenceResult
  phrase_id
  decision: Candidate | Rejected
  reading_results[2]
  aggregate_reason_codes[]

Candidate
  probe_span_id
  from
  to
  state: inactive
```

Every rejected reading and aggregate rejection uses stable machine-readable reason codes. V1 requires at least:

| Code | Meaning |
| --- | --- |
| `no_mismatch` | Normalized observed and expected text are identical. |
| `missing_context` | No exact context token anchors either side of the mismatch. |
| `insertion_or_deletion` | One mismatch side is empty, so there is no nonempty substitution mapping. |
| `multiple_hunks` | An exact token occurs inside both residual ranges, proving separated edits. |
| `outside_eligible_span` | The expected mismatch is not wholly inside exactly one eligible probe span. |
| `span_mapping_failed` | The full eligible span cannot map to one contiguous observed range. |
| `single_word_source` | The expanded observed trigger has fewer than two words. |
| `readings_disagree` | Both readings produced mappings, but their span ID, `from`, or `to` values differ. |

Reason codes are local diagnostic data. This specification does not authorize sending transcripts, expected text, audio, or reason-bearing content to a network service. A later observability decision may define local retention and user-facing wording without changing these inference semantics.

## Required fixtures

Regression fixtures should cover at least these cases using the exact corpus data and the same normalization implementation:

| Evidence | Result |
| --- | --- |
| Both readings say `That quick chip carries heavy blue boxes` | Propose `quick chip → quick ship`. |
| Both readings say `Your boys made the joyful choice sound easy` | Reject with `single_word_source`. |
| One reading says `quick chip`, the other `quick sheep` | Reject with `readings_disagree`. |
| Both readings change untagged `carries` to `carried` | Reject with `outside_eligible_span`. |
| Both readings omit a word without replacing it | Reject with `insertion_or_deletion`. |
| Both readings contain two edits separated by an exact matching word | Reject with `multiple_hunks`. |
| Both readings say `up stairs` for expected `upstairs` | Propose `up stairs → upstairs`. |
| Readings differ only in case, punctuation, hyphenation, spacing, or apostrophe typography | Return `no_mismatch`. |

Fixtures must assert the decision, reason codes, normalized source and target, probe-span identity, and the two independent alignment ranges. They should run across every supported Whisper model using captured raw transcript strings; model and microphone coverage belong to the regression strategy, not to the pure inference function.

## Boundaries for later decisions

- [Resolve Personal Dictionary conflicts and rule lifecycle](https://github.com/hens0n/eaglescribe/issues/31) decides duplicate/conflicting `from` keys, approval, provenance, editing, removal, and activation timing.
- [Specify Verification Pass and rollback outcomes](https://github.com/hens0n/eaglescribe/issues/28) decides positive and harmful-replacement scoring, retries, and rollback.
- User-facing explanations, local retention, and aggregate observability are not specified here; they may build on the stable reason codes above.
- This inference path never modifies Whisper model weights or decoder parameters and never labels the user's pronunciation wrong.
