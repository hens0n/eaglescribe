# V1 Tuning Phrase corpus

**Status:** Research recommendation
**Date:** 2026-07-16
**Related:** [Curate the v1 Tuning Phrase corpus](https://github.com/hens0n/eaglescribe/issues/30) · [Design guided user-specific Tuning](https://github.com/hens0n/eaglescribe/issues/25)

## Recommendation

Ship one unscored practice prompt, ten scored English Tuning Phrases, two separated readings of every scored phrase, and a prompt-disjoint Verification Pass selected only for approved Correction Rules. Target **4–6 minutes** for a normal session and treat **7 minutes** as a usability failure threshold to investigate, not as a promise proven by the literature.

This is deliberately a small read-speech probe, not a comprehensive English corpus. TIMIT used ten phonetically rich utterances per speaker and mixed phone-pair coverage with phonetic-context diversity; its design supports ten as a defensible v1 scale, not as a magic optimum for EagleScribe ([NIST/LDC TIMIT documentation](https://catalog.ldc.upenn.edu/docs/LDC93S1/timit.readme.html)). CMU ARCTIC selected easily read, in-dictionary prompts of 5–20 words and found that chasing rare diphones eventually produced awkward prompts and questionable coverage; that supports short natural phrases and a stopping rule rather than phonetic maximalism ([CMU ARCTIC paper](https://www.isca-archive.org/ssw_2004/kominek04b_ssw.pdf)). Microsoft likewise recommends speech material representative of users' real utterances and warns against irrelevant material; therefore these prompts use contemporary dictation-like language rather than tongue twisters ([Microsoft Custom Speech guidance](https://learn.microsoft.com/en-ie/azure/ai-services/speech-service/how-to-custom-speech-test-and-train)).

## Exact built-in corpus

The text below is original project-authored text. Bold spans are the **only candidate-eligible probe spans**. A mismatch elsewhere may be logged for diagnostics but must not create a Candidate Correction in v1. Restricting eligibility is necessary because every eligible span needs a held-out phrase that can exercise it.

### Unscored practice

> Today is a good day to try voice typing.

Discard its transcript. It exists to let the user experience the capture rhythm before scored readings.

### Tuning Phrases: Pass A order

| ID | Exact displayed text | Words |
| --- | --- | ---: |
| T01 | That **quick ship** carries **heavy blue boxes**. | 7 |
| T02 | Your **voice** made the **joyful choice** sound **easy**. | 8 |
| T03 | **Measure** the **yellow ring** before you **order** it. | 8 |
| T04 | **Three green leaves** fell beside the **path**. | 7 |
| T05 | She found a **good blue book** **upstairs**. | 7 |
| T06 | We made **small talk** near the **busy office**. | 8 |
| T07 | Check the **fresh weather report** before **lunch**. | 7 |
| T08 | A **brown fox** crossed the **quiet yard**. | 7 |
| T09 | The **late train** should **reach town** by **nine**. | 8 |
| T10 | The **judge** **chose** a **bright orange jacket**. | 7 |

The ten prompts total 74 words, and every prompt is 7–8 words. A local check against CMUdict's unstressed ARPAbet symbols found all 39 base phones across the full displayed text and also across the bold probe spans: `AA AE AH AO AW AY B CH D DH EH ER EY F G HH IH IY JH K L M N NG OW OY P R S SH T TH UH UW V W Y Z ZH`. CMUdict is maintained by Carnegie Mellon's Speech Group for speech-technology use and permits unrestricted research and commercial use, while warning that errors and omissions remain ([CMUdict repository and license statement](https://github.com/cmusphinx/cmudict)). Phone coverage is a construction check, not evidence that these ten prompts cover every accent, phone context, or likely Whisper error.

Pass A is ordered by descending marginal base-phone coverage in that CMUdict check: T01 adds 20 phones, T02 11, T03 4, T04 3, and T05 the final 1; T06–T10 add common words and new contexts. This makes an abandoned session collect the broadest probe first, but the ordering itself is a product inference and should be usability-tested.

### Tuning Phrases: Pass B order

Read every phrase again in this deterministic order:

> T06, T07, T08, T09, T10, T01, T02, T03, T04, T05

This five-position rotation separates the two readings of every phrase by at least five intervening prompts. Do not reveal the Pass A transcript, highlight a mismatch, or describe Pass B as an error retry. People resolving a computer recognition error have been observed to repeat the same lexical content with longer segments, more and longer pauses, and clearer speech features; hiding interim results and separating the readings reduces that known error-repair cue, though it cannot guarantee identical delivery ([Oviatt et al., *Modeling hyperarticulate speech during human-computer error resolution*](https://research.monash.edu/en/publications/modeling-hyperarticulate-speech-during-human-computer-error-resol/)).

Use a **full second pass**, not selective rereads. Selective rereads save at most 74 spoken words but reveal, through prompt selection, which first readings the system disliked and make the second sample an error-repair interaction. The full pass also gives the same two-opportunity protocol to every probe and directly implements the map's “same mismatch in two readings” boundary. This is a product decision derived from the error-repair evidence above, not a published comparison of EagleScribe protocols.

## Separate Verification Pass set

Each verification phrase is different from its Tuning Phrase but repeats every candidate-eligible span from that row. TIMIT's complete test split likewise keeps the same sentence text from appearing in both training and test material, supporting prompt disjointness as the cleaner evaluation design ([NIST/LDC TIMIT documentation](https://catalog.ldc.upenn.edu/docs/LDC93S1/timit.readme.html)).

| Pair | Exact held-out text |
| --- | --- |
| V01 | The **heavy blue boxes** arrived on a **quick ship**. |
| V02 | The **joyful choice** was **easy** to explain in her **voice**. |
| V03 | Before the **order** ships, **measure** the **yellow ring** again. |
| V04 | **Three green leaves** covered the garden **path**. |
| V05 | **Upstairs**, the **good blue book** remains on the desk. |
| V06 | After **small talk**, the **busy office** fell quiet. |
| V07 | During **lunch**, we checked the **fresh weather report**. |
| V08 | The **brown fox** left the **quiet yard** before dawn. |
| V09 | By **nine**, the **late train** should **reach town**. |
| V10 | The **bright orange jacket** pleased the **judge** who **chose** it. |

After approval, enqueue each distinct verification row needed by at least one approved rule; if several approved rules came from one Tuning Phrase, read its paired row once. Apply and score rules individually so one failure rolls back only its own Correction Rule, as required by the map. The corpus supplies a positive held-out occurrence; the separate Verification Pass decision must still define how to detect harmful replacements outside that occurrence and the exact pass/fail comparison.

Do not substitute an unrelated fixed “test paragraph.” A Verification Pass can exercise a Correction Rule only if its raw/preferred mapping occurs in the held-out material. A small static verification list therefore requires either the tagged-span restriction above or a much larger built-in index with held-out contexts for every candidate-eligible span; arbitrary mismatches cannot honestly be verified by these ten sentences.

## Session budget

- Practice plus both full Tuning passes: **157 displayed words** (9 + 74 + 74).
- Verification: **7–10 words per selected row**, 87 words if all ten rows are needed.
- Worst-case spoken total: **244 words**. A study of adults reading short texts reported about 160 words per minute aloud, implying roughly 92 seconds of voiced speech at that measured rate; UI transitions, local Whisper latency, approvals, pauses, and accessibility needs dominate the remaining session time ([Brysbaert, *No Correlation Between Articulation Speed and Silent Reading Rate when Adults Read Short Texts*](https://pmc.ncbi.nlm.nih.gov/articles/PMC10360968/)).

The **4–6 minute target** is therefore an engineering estimate, not a sourced human-completion benchmark. Instrument median and 90th-percentile time by stage locally (without retaining audio or dictation content), and revisit corpus size if ordinary sessions cross seven minutes.

## Boundaries and risks

1. **Static lexical ceiling.** This corpus can discover corrections only for its tagged built-in words and phrases. It cannot produce Dictionary mappings for unseen names, jargon, organizations, product terms, or user-specific vocabulary. That limitation follows directly from v1's ban on user-authored and history-derived Tuning Phrases; it must be explicit in the UI and product spec.
2. **Phone coverage is not error coverage.** Covering CMUdict's 39 unstressed base phones does not establish coverage of diphones, stress, dialects, prosody, microphones, or Whisper model variants. TIMIT deliberately combined compact phone-pair prompts and phonetically diverse contexts, while CMU ARCTIC shows that exhaustive rare-context coverage grows burdensome ([TIMIT](https://catalog.ldc.upenn.edu/docs/LDC93S1/timit.readme.html); [CMU ARCTIC](https://www.isca-archive.org/ssw_2004/kominek04b_ssw.pdf)).
3. **Common-word danger.** A globally applied mapping between common words (especially function words or homophones) can be harmful outside the prompt context. The Candidate Correction inference decision should reject unsafe/general mappings, use exact whole-word or multi-word boundaries, and require the paired Verification Pass; corpus membership alone is not evidence of safety.
4. **Accent neutrality.** Different pronunciations are valid. Mozilla Common Voice explicitly accepts accent variation and asks readers to use their normal voice; Tuning must compare intended text with raw Whisper output without labeling the user's pronunciation “wrong” ([Mozilla Common Voice contribution guidelines](https://commonvoice.mozilla.org/en/guidelines)).
5. **Licensing.** Do not copy TIMIT, Harvard/IEEE, or other benchmark prompt text merely because its design is useful. The proposed prompt text is original to EagleScribe. CMUdict may be used for automated coverage validation under its unrestricted terms ([CMUdict](https://github.com/cmusphinx/cmudict)).

## Acceptance checks for the corpus artifact

- Store stable IDs, exact text, candidate-eligible spans, Pass A order, Pass B order, and verification-pair ID as data rather than UI literals.
- CI tokenizes corpus text using the same normalization selected by the Candidate Correction inference spec and confirms that every eligible span occurs verbatim in its paired verification phrase.
- CI checks that discovery and verification full strings are disjoint and all prompts remain below 15 words, consistent with Mozilla Common Voice's guidance that useful submitted sentences be natural, conversational, easy to read, and fewer than 15 words ([Mozilla Common Voice contribution guidelines](https://commonvoice.mozilla.org/en/guidelines)).
- Pin the CMUdict revision used by the test; fail on unknown corpus words and report base-phone coverage as a diagnostic, not a release guarantee.
- Before locking v1, run the complete flow with at least the supported default Whisper model and measure duration, abandon rate, prompt rereads, and Candidate Correction yield. This is necessary because the exact EagleScribe corpus and protocol have not been validated by the cited external studies.
