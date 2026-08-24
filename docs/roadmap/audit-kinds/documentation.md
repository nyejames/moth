# Documentation

Looking for: prose that misleads - wrong authority, stale status, broken routing, inaccurate examples, or two documents claiming the same contract.

**Evidence bar:** the exact conflicting, missing or ambiguous content, plus the canonical source of truth it contradicts.

A documentation audit may correct inaccurate documentation. It cannot change accepted semantics under the label of cleanup, and it cannot settle an open design question. Record those as blocked design proposals.

## Authority hierarchy

For the subject, identify which document owns accepted semantics, current status, future sequencing, contributor process and public explanation. A document may explain another authority without replacing it.

Check that each states what it owns, what it does not, its prerequisites, and whether it describes accepted end state or current implementation.

Watch for two documents claiming one canonical contract, a teaching page presented as architecture authority, a plan overriding accepted design, a progress page read as end-state design, and skills or agent instructions duplicating repository policy instead of routing to it.

## Canonical consistency

Across the canonical owners the subject touches: terminology and stage numbering agree, producer and consumer boundaries agree on data shape and ownership, each responsibility is assigned once, language and memory semantics agree, and accepted exclusions are consistent.

When canonical authorities conflict, record the exact incompatible statements and route to both owners. Do not silently pick one.

## Status against reality

Compare canonical design with the [progress matrix](../../src/docs/progress/@page.moth):

- supported rows describe behaviour the compiler actually claims
- partial rows name the implemented subset and the important gaps
- deferred rows do not read as implemented public behaviour
- accepted deferred work is distinguished from design-pending work
- backend coverage matches actual target support
- coverage labels are defensible against `tests/cases/manifest.toml`

A code defect found here becomes a linked Correctness finding; a missing test becomes a linked Tests finding. The documentation finding owns only the inaccurate claim.

## Paths, links and terminology

Verify every in-scope path and reference actually resolves. Search for old paths and terminology rather than only checking visible links: renamed modules, old project or language names, old file extensions, removed keywords, renamed stages or IRs, old diagnostic prefixes or command names.

`index.md` stays a locator, not a competing design authority. Generated output under `docs/release/**` is never cited as a source owner and is never edited directly.

## Examples

Check syntax against the canonical language reference, and status against the progress matrix - an example must not use deferred syntax as though it were supported. Verify imports, paths, module roots, mutability and return syntax. Invalid examples must be clearly labelled invalid.

Run executable examples through the documentation gate. Do not claim an example compiles when it was only read. Note that examples inside non-compiled code fences are not checked by the gate, so they drift silently and need reading.

## Teaching pages and compact references

Teach the canonical contract accurately, mark deferred or target-limited behaviour, do not invent design-pending syntax, and link to detailed owners rather than becoming incomplete authorities. A simplification is valid only when it stays true. Compact references must not compress away a restriction that affects ordinary use.

## Duplication

For each rule repeated in several places, decide: keep a compact summary plus link; move the detail to the canonical owner; retain separate teaching examples with no duplicated authority claim; remove obsolete copies; or leave the repetition because each audience needs the complete local rule.

## Classify the finding

Accuracy correction, non-semantic clarification, status correction, navigation or ownership correction, or design proposal. **Design proposals stay blocked** until approved - never implement one as cleanup.

## Stale when

Canonical design or terminology, progress status, roadmap sequencing, file or module structure, teaching pages, or contributor routing changes materially.

A production refactor leaving paths, status and documented contracts unchanged does not stale documentation.
