# AUD-0016: Empty-container documentation correctness

- State: `complete`
- Kind: `Documentation`
- Area: `docs.empty_container_contract` — accepted empty-map and empty-collection semantics across the public language references, status matrix and related examples.
- Coverage: `partial`
- Reviewed: `2026-10`
- Baseline: Branch `feature/mon-crate-extraction`, reviewed tip `8b1229c119693e070fbf565824402c75453a7162`. The external reviewer source-traced that tip. Focused validation passed: `cargo test -q -p moth --lib` (5,460 passed); `cargo test -q -p moth-mon` (passed); clippy for `moth` and `moth-mon` clean; `moth check docs --terse` clean. The final `just validate-full` result for the exact closeout tree is recorded in the closeout commit message.

## What was inspected

The external review compared the empty-container claim in the hash-map reference with the accepted roadmap decision and checked the progress matrix's implementation status. It also traced the empty-map examples and related statements in the collection-literal reference, language cheatsheet, reactive-source reference, MON format and developer memory example, plus the existing AUD-0010-F01 report. The review did not audit every brace-literal example in the documentation set.

## Authorities read

`docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/documentation.md`; `docs/roadmap/audits/README.md`; `docs/roadmap/audit-log.md`; `docs/roadmap/open-audit-findings.md`; `docs/roadmap/roadmap.md:105-109` (accepted empty-container design); `docs/src/docs/progress/@page.moth` (current hash-map implementation status); `docs/src/docs/collections/hash-maps.mtf`; `docs/src/docs/collections/collection-literals.mtf`; `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`; `docs/src/docs/reactivity/reactive-sources.mtf`; `docs/src/docs/mon/mon-format.mtf`; `docs/src/developer-docs/memory-management/retained-edge-counting/retained-edge-counting.mtf`; and `docs/roadmap/audits/AUD-0010-mon-syntax-parity-tests.md`.

## Existing findings and active plans checked

The audit log and open-findings index were checked. AUD-0010-F01 records an observed `MapHolder(scores = {})` diagnostic and a fixture correction. The observed `MOTH-TYPE-0002` remains current compiler behavior, but it is not a defect against the accepted design: `{}` is always a collection and source `{=}` remains unimplemented. The earlier report's wording is clarified below so its test-fixture rationale does not treat contextual typing as the accepted empty-map contract. The roadmap decision was treated as authority for the accepted design; the progress matrix remains the authority for implementation status.

## Findings

### AUD-0016-F01: Empty-container documentation conflates current behavior with accepted design

- State: `closed`
- Kind: `Documentation`

#### Evidence

Before the correction, `docs/src/docs/collections/hash-maps.mtf` presented empty `{}` as a map when an explicit or contextual map type supplied a receiver. The roadmap's accepted design at `docs/roadmap/roadmap.md:105-109` is unambiguous: `{=}` is an empty map and `{}` remains a collection even at a map receiving context. The progress matrix separately records that the current compiler still types `{}` as a map from an explicit or immediate contextual map type, and marks the accepted `{=}` design as not implemented. Public explanation must not turn that current behavior into the accepted container-kind rule.

The observed `MapHolder(scores = {})` `MOTH-TYPE-0002` result in AUD-0010-F01 describes current compiler behavior; it does not show a violation of the accepted design. Under that design `{}` is a collection regardless of receiving context, while source `{=}` remains deferred.

#### Counter-explanation tested

The old wording could have been retained as a description of currently implemented contextual typing. The progress matrix does need to preserve that implementation fact, and it does. But a public language contract cannot present that status as the accepted end state when the roadmap says the container's kind does not change with context. The correction separates the two statements instead of deleting current-status information or implementing deferred syntax.

#### Violated contract or cost

Readers could mistake current implementation behavior for accepted semantics and rely on `{}` as a map literal even though the roadmap accepts `{=}` as the only empty-map spelling. Conflicting examples and summaries also made the deferred source feature look implemented.

#### Root owner

The public empty-container explanations and examples in the hash-map, collection-literal, cheatsheet, reactive-source and developer memory references, with implementation status owned by the progress matrix and accepted design owned by the roadmap.

#### Suggested correction

State the accepted container-kind rule consistently in public and example documentation: `{=}` is an empty map, `{}` is an empty collection, and receiving context does not change the kind. Keep the implementation's current behavior in the progress matrix as status and mark the accepted source syntax as not yet implemented.

#### Fix scope and preserved invariants

Resolved in the closeout commit following `8b1229c11`. The hash-map, collection-literal, cheatsheet and reactive-source references now state the accepted distinction and identify source `{=}` as accepted but not implemented. Developer memory examples use `{=}` for maps. The progress matrix distinguishes current compiler behavior from the accepted design. No source semantics changed. AUD-0010-F01's `MapHolder(scores = {})` observation is retained as current behavior and no longer frames it as a bug against the former contextual-map rule.

#### Required validation

Run the documentation release check on the final documentation tree and confirm references and anchors resolve. `cargo run --quiet -- check docs --terse` passed on the completed audit-record tree; the focused package tests and clippy also passed. The final `just validate-full` result for the exact closeout tree is recorded in the closeout commit message.

#### Linked findings

AUD-0010-F01 records the related test-fixture context and current `MapHolder(scores = {})` observation.

#### Triage record

2026-10-03 — **Accepted.** Align the public design statement with the roadmap and distinguish accepted syntax from current implementation status. Preserve `{=}` as the accepted empty-map spelling and `{}` as an empty collection regardless of context; do not implement source `{=}` as part of documentation correction.

2026-10-03 — **Fixed and closed.** The closeout commit following `8b1229c11` aligns the relevant public references and examples, makes the progress matrix distinguish current behavior from accepted design, and clarifies the AUD-0010-F01 observation. The documentation gate passed. The final `just validate-full` result is recorded in the closeout commit message.

## Checked and clean

The roadmap continues to own accepted semantics and the progress matrix continues to report actual support. MON's accepted `{=}` map data and source's deferred `{=}` implementation remain distinct. The `MapHolder(scores = {})` diagnostic remains a factual observation, not evidence that the accepted empty-container design requires contextual map inference.

## Limitations

Coverage is partial and limited to the empty-container references and examples named above. The external review source-traced this contract and did not audit every map, collection or brace-literal teaching example. This report does not claim a full documentation consistency audit.
