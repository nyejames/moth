# Comments Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. The repository [style guide](../../src/docs/codebase/style-guide/style-guide.mtf) remains the authority for code comments and file documentation. This document defines the focused audit procedure.

A comments audit is read-only and code-neutral. It records missing, stale, misleading, noisy or misplaced implementation comments. It does not authorise executable changes or edits to public design documentation.

## Purpose and boundary

Use this audit to make implementation intent locally understandable without narrating syntax or duplicating canonical documentation.

The audit covers:

- file-level ownership documentation
- module entry-point guidance
- intent comments for important types, functions and blocks
- stage, ordering and data-flow landmarks
- invariants, failure conditions and fallbacks
- subtle non-local behaviour and consumer expectations
- stale, contradictory or redundant comments
- lint, unsafe and temporary-work explanations

Route these concerns elsewhere:

- unclear code that should be simplified rather than explained -> Style
- incorrect behaviour or invariant -> Correctness
- duplicated or obsolete implementation -> Redundancy
- public design, progress, roadmap or teaching prose -> Documentation
- missing tests for a documented invariant -> Tests

## Valid scopes

- A single production file is a valid partial audit unit.
- A leaf scope is the default complete unit.
- A composite scope is valid when comments must explain a multi-file flow or orchestration sequence.
- Contract scopes are valid when producer and consumer comments describe a shared handoff.
- Comparison scopes are rarely useful unless equivalent owners use conflicting terminology.

A complete leaf audit reads every production file plus the module entry point. It does not need to audit public documentation outside the code unless a comment cites or contradicts it.

## Audit procedure

### 1. Establish the comment contract

Before judging individual comments:

- identify what the scope owns
- identify important exclusions
- identify the main data flow and stage position
- identify invariants that a local reader cannot infer from types alone
- identify behaviour whose reason lives in another module or canonical document
- identify accepted terminology for stages, artefacts, identities and failure lanes

Use this map to decide what needs local explanation. Do not copy large parts of the architecture documents into code comments.

### 2. Review module entry-point documentation

Check that the module entry point:

- states the module's single responsibility
- names important input and output data
- explains how major files or submodules divide the work
- identifies important exclusions and neighbouring owners
- describes the main flow in execution order when the module orchestrates several steps
- points readers to the relevant owner instead of restating distant implementation
- uses current file and type names
- does not claim authority over behaviour owned elsewhere

Record a finding when a reader cannot determine where to start, what the module owns or why its files are arranged as they are.

### 3. Review file-level documentation

Every production file should have concise WHAT/WHY documentation when its role is not trivial from its module and filename.

Check that file documentation:

- states what the file owns
- states important exclusions where confusion is likely
- explains how it fits the wider module or compiler stage
- names downstream consumers when they constrain the data shape
- avoids repeating the filename or listing every type and function
- remains useful when the file is read directly
- matches the current owner after moves or refactors

A tiny file whose purpose is completely obvious from the module entry point may not need a long header. Do not require boilerplate.

### 4. Review important types and data structures

Inspect comments on important structs, enums, IDs, tables, arenas, side tables and result types.

Check whether comments explain:

- semantic ownership
- lifecycle and mutability
- identity domain and valid comparisons
- alignment between parallel data structures
- whether data may cross module, stage, thread or persistence boundaries
- whether a type is authoritative or derived
- valid and invalid states
- why fields are separated or grouped
- whether a result is success-only, diagnosed, partial or immutable
- why a compact or unusual representation is safe

Reject comments that merely restate field names or enum variants.

### 5. Review complex functions and orchestration

For each complex function or pass, check for concise landmarks that explain:

- the overall operation
- major phases and their required order
- why one phase must happen before another
- where data changes authority or representation
- why a fallback or conservative path exists
- where diagnostics are accumulated or returned
- why independent branches can or cannot continue
- where deterministic ordering is restored after parallel work
- why a no-op branch is intentional
- why a helper is called at this stage rather than an adjacent one

The main path should read as named steps. Comments should support that flow rather than compensate for an unreadable function. Route structural code problems to Style.

### 6. Review non-local intent

Search for code whose correctness depends on facts outside the local block.

Check that comments explain, where relevant:

- producer or consumer contracts
- stable identity and remapping requirements
- ownership boundaries between stages
- why source is not rescanned or reparsed
- why data remains local or cannot cross a boundary
- why an immutable artefact or side table exists
- why validation is deliberately conservative
- why optional proof falls back instead of rejecting source
- why a user-facing diagnostic belongs to this stage
- why a path is serial or deterministic despite possible parallelism
- why a clone, allocation or conversion is currently necessary
- why similar-looking code remains separate

These comments should give the local reason. They should not turn into historical essays.

### 7. Review invariants and failure paths

Check that comments identify non-obvious invariants around:

- unchecked indexing, `.unwrap()`, `unreachable!` or internal panic paths
- unsafe code
- ID, arena or table alignment
- graph acyclicity and ordering
- phase and state transitions
- cache keys and invalidation
- source remapping and deterministic merges
- ownership, alias or lifetime facts
- generated sidecars and immutable base artefacts
- target-specific assumptions
- resource cleanup and output ownership

A comment does not legalise a weak invariant. When the code cannot prove or enforce the stated condition, create a linked Correctness or Style finding.

### 8. Review diagnostic and recovery explanations

Where error handling is non-obvious, check that comments explain:

- why the path uses `CompilerDiagnostic` or `CompilerError`
- why a diagnostic is emitted at this owner
- why recovery continues or stops
- why a consumer is blocked without emitting a cascade
- why a warning is retained only on success
- why source labels or remapping happen at a particular boundary

Do not require comments for self-explanatory diagnostic construction. Route wording and user-error quality to Diagnostics.

### 9. Remove restatement and noise

Identify comments that:

- narrate the next line
- paraphrase a descriptive function or variable name
- repeat a type signature
- label trivial getters, setters, loops or matches
- restate canonical documentation without local relevance
- use section banners for minor blocks
- preserve old implementation history that Git already records
- contain TODOs with no owner, reason or current relevance
- contain speculative plans not accepted by the roadmap or design docs
- use vague claims such as "for safety", "for performance" or "temporary" without explaining the constraint
- create more visual noise than guidance

Prefer deletion when the code already says the same thing. Prefer a code rename or refactor when a comment exists only because the code is obscure.

### 10. Check staleness and contradictions

Compare every important comment with current code and canonical terminology.

Look for:

- renamed types, stages, fields or modules
- comments describing deleted passes or compatibility paths
- stale ordering claims
- comments assigning ownership to the wrong stage
- comments that describe an old API shape
- outdated target or feature support
- comments claiming a value is copied, moved, borrowed, cached or immutable when the code no longer does that
- comments that conflict with canonical docs or the progress matrix
- references to old file paths or plans

A stale comment is a finding even when the implementation is correct because it actively misleads future work.

### 11. Review grammar and presentation

Check that comments:

- use complete readable sentences where prose is needed
- use direct active language
- use current project terminology consistently
- avoid unnecessary headings, banners and decorative separators
- remain concise enough to scan with the code
- use code formatting for exact identifiers where supported by the comment form
- do not include stale issue numbers or external context as the only explanation

Do not turn a comments audit into a general prose rewrite. Preserve useful local voice where it remains clear and accurate.

### 12. Classify each correction

For every issue, choose one action:

1. add a missing ownership or intent comment
2. replace a misleading comment with the current reason
3. shorten a comment to its non-local WHAT/WHY content
4. delete a restating or obsolete comment
5. move an explanation to the module or file owner
6. route an implementation problem to another audit kind
7. leave the code uncommented because names and types already make it clear

A comment finding should include the intent that needs preserving, not only "add comments".

## Valid findings

A comments finding needs concrete evidence that the current prose:

- hides or omits important non-local intent
- assigns ownership incorrectly
- states a stale or false invariant
- makes a complex flow hard to follow
- preserves obsolete history or speculative design
- restates code and obscures the useful comments around it
- fails to justify unsafe, unchecked or lint-suppressed behaviour

Do not report a lack of comments in self-describing trivial code.

## Kind-specific preservation rules

A comments fix must preserve:

- every executable token outside comments
- accepted semantics and current support status
- all tests, fixtures and generated artefacts
- canonical documentation
- module and stage ownership

A comments finding cannot authorise a code cleanup. When correct wording depends on changing code, link the relevant Style, Correctness or Redundancy finding and keep the comment fix blocked until the owner is clear.

## Freshness invalidators

Mark a comments audit stale when the scope receives material changes to:

- module or file ownership
- principal control flow
- stage ordering or data handoffs
- important types, invariants or error paths
- comments in the reviewed scope
- terminology or canonical contracts referenced by the comments

A small implementation change does not stale the entire comments audit unless it changes the reason or invariant described by existing comments.

## Completion checklist

A complete comments audit confirms that:

- every production file and module entry point was read
- ownership and exclusion documentation was checked
- important types and complex functions were checked
- non-local intent, ordering, fallbacks and invariants were checked
- unsafe, unchecked, lint-suppressed and recovery paths were checked
- stale, contradictory, noisy and speculative comments were checked
- each missing comment has a specific intent to explain
- implementation, diagnostic, test and public-documentation issues were routed out of lane
