# Style Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. The repository [style guide](../../src/docs/codebase/style-guide/style-guide.mtf) remains the implementation-style authority. This document defines the procedure for auditing against it.

A style audit is read-only. It records findings about readability, local organisation, API shape and implementation quality. It does not authorise semantic changes, test changes, broad architectural consolidation or speculative optimisation.

## Purpose and boundary

Use this audit to answer whether the code is easy to read, review, extend and remove without first reverse-engineering its structure.

The audit covers:

- module and file organisation
- naming, imports and local layout
- API and data-shape clarity
- function and control-flow readability
- abstraction size and placement
- data-oriented implementation patterns
- lint, macro and unsafe-code hygiene
- local clone, allocation and ownership noise when the concern is code shape rather than measured speed

Route these concerns elsewhere:

- missing, stale or noisy comments -> Comments
- duplicated work, legacy paths or wrong owners -> Redundancy
- observable or invariant failures -> Correctness
- user-error behaviour -> Diagnostics
- missing or weak tests -> Tests
- measured runtime or memory cost -> Performance
- public or canonical prose -> Documentation

## Valid scopes

- A leaf scope is the default complete style-audit unit.
- A single file may be audited as a partial leaf audit.
- A composite scope is valid when module organisation, public API shape or repeated local conventions must be reviewed together.
- Contract and comparison scopes are not ordinary style scopes. Use them only when the finding concerns the readability of an explicit shared boundary, not implementation similarity.

A complete leaf audit inspects every production file in the scope, its module entry point and the public API exposed to adjacent owners.

## Audit procedure

### 1. Establish the local owner

- Open the module entry point first.
- Identify the single responsibility owned by the scope.
- Identify its input data, output data and adjacent producers or consumers.
- Check that names and module structure make that ownership visible before implementation details are read.
- Record any file whose responsibility is unclear or whose name no longer matches what it owns.
- Do not infer a new architecture merely because the current layout is untidy. Route ownership changes to Redundancy or Correctness as appropriate.

### 2. Review module and file structure

For each module and file, check:

- the module entry point acts as a structural map rather than a storage place for unrelated implementation
- files group one coherent task or data owner
- unrelated concepts are not mixed merely because they execute in the same stage
- related private behaviour is deepened through submodules rather than scattered across broad utility files
- files are not split so aggressively that basic control flow requires constant navigation
- large files and functions remain coherent rather than using size alone as justification for splitting
- re-exports expose a narrow intentional surface
- private implementation details remain private
- test code does not live in production implementation files
- feature-gated or platform-specific code remains easy to locate and reason about

When suggesting a split, name the responsibility that moves and the data boundary it receives and returns. Do not recommend a new file only to reduce line count.

### 3. Review data shape and state representation

Prefer data-oriented code where data ownership and stage transitions are explicit.

Check that:

- structs represent meaningful data records or stage results rather than objects with broad behavioural ownership
- compiler passes operate over explicit inputs, stores, arenas, tables, side tables or immutable artefacts
- data used together is stored and passed together when that improves review and prevents mismatched state
- data with different lifetimes or owners is not hidden inside one broad context object
- context structs reduce noisy parameter threading without becoming mutable global bags
- enums represent meaningful states instead of clusters of booleans
- named result structs replace tuple-heavy returns when field meaning matters
- IDs and indexes make their owning arena or table clear
- parallel vectors, maps or side tables have an obvious alignment invariant and owner
- stage-local data does not gain methods that obscure which pass mutates or consumes it
- dynamic dispatch, trait-object hierarchies and object-style wrappers are not used where explicit data plus narrow operations are clearer
- generic abstractions do not hide concrete compiler-stage ownership
- data layout choices are justified by the actual access pattern rather than fashion

Data-oriented design does not require converting every struct into parallel arrays. Do not propose a layout rewrite without a clear readability, ownership or measured performance benefit.

### 4. Review API shape

Inspect public and important internal functions, types and modules.

Check for:

- descriptive input and result types
- narrow functions that expose one operation or query
- explicit state transitions rather than hidden mutation
- context or input structs where several related parameters always travel together
- no boolean-heavy call sites whose meaning is unclear without reading the signature
- no defaulted or optional parameters preserving an obsolete API shape
- no broad trait bound or generic parameter that exists only to make one call site abstract
- borrowed views for read-only queries where cloning a whole collection would obscure intent
- owned return values only where ownership transfer is real
- consistent naming between producer and consumer sides of a handoff
- no helper whose name is broader than the behaviour it actually owns
- no public type or function exposed only because internal modules are poorly arranged

API compatibility is not a goal for pre-release internal code. A style finding may recommend one clearer current shape, but removal of old paths belongs to a linked Redundancy finding when the change is structural.

### 5. Review functions and control flow

For each non-trivial function, check that:

- the function name matches its complete responsibility
- the main path reads as a sequence of named steps
- unrelated phases are separated into narrow helpers
- early returns make exceptional or terminal paths clearer
- complex validation uses explicit loops and control flow rather than nested combinator chains
- iterator chains remain simple transformations without hidden mutation or error accumulation
- large matches are grouped by meaning and split when branch handling has independent responsibilities
- named intermediate values expose data flow and avoid repeated expressions
- temporary variables clarify state rather than preserve stale intermediate forms
- closures stay small and local
- control-flow joins, fallbacks and no-op branches remain visible
- deeply nested branches are not hiding a missing helper or state enum
- error propagation uses the correct local result shape without type gymnastics
- unsafe code, if any, has the smallest possible scope and a clear invariant owner

Do not reward compressed code merely for using fewer lines. Prefer the shortest form that remains obvious during review.

### 6. Review naming, imports and local layout

Check that:

- names use full domain terms instead of unexplained abbreviations
- similarly named types and helpers have distinct roles
- stage and phase names match canonical terminology
- imports keep long paths out of implementation bodies without flooding the module namespace
- aliases exist only when they improve clarity
- type ordering moves from high-level concepts to supporting details
- related statements and match arms are grouped visually
- blank lines reveal logical steps and major control-flow boundaries
- macros are small, declarative and easier to understand than the repeated code they replace
- section banners mark real phase boundaries rather than disguising an oversized file
- formatting follows `rustfmt`

Comment wording and coverage belong to the Comments audit. A style audit may note that code cannot be read without distant context, but the comment-specific remedy must be recorded in the Comments lane.

### 7. Review copying, allocation and ownership noise

Without making performance claims, inspect code shape for:

- `.clone()`, `.to_owned()` or collection rebuilding used to avoid a clearer borrow or ownership boundary
- owned `String`, `PathBuf` or diagnostic prose stored where interned or borrowed identity is already available
- whole-collection cloning for one lookup or iteration
- repeated conversion between equivalent local representations
- wrapper allocations introduced only to satisfy an awkward API
- defensive copies that indicate unclear mutation ownership

A local style finding may recommend a simpler ownership shape when behaviour and complexity remain unchanged. Route any claim about runtime or memory improvement to Performance and any cross-stage representation consolidation to Redundancy.

### 8. Review errors, lints and exceptional paths

Check that:

- user-driven paths do not use `panic!`, `todo!`, unchecked indexing or unjustified `.unwrap()`
- internal invariant failures are visibly distinguished from recoverable or user-facing failures
- local result aliases and boxed errors clarify rather than hide the error lane
- lint suppressions are narrow, documented and still needed
- `allow(dead_code)` does not preserve forgotten implementation
- `cfg` branches expose one understandable current path per target or feature
- temporary debug printing and instrumentation are absent from normal code

A wrong acceptance result or wrong diagnostic lane is not merely style. Route it to Correctness or Diagnostics.

### 9. Decide the right structural action

For each style issue, classify the smallest valid correction:

1. rename or reorder locally
2. simplify a function or API without moving ownership
3. introduce a narrow named type or helper inside the same owner
4. split a mixed-responsibility file inside the same owner
5. route a cross-owner concern to Redundancy or Correctness
6. leave the code unchanged because the current explicit form is clearer than an abstraction

State why the selected action improves reviewability. Do not use "cleaner" or "more idiomatic" as the full justification.

### 10. Perform the final style pass

After inspecting every file:

- compare naming and data shapes across the complete leaf scope
- check that one local convention is not implemented several different ways without reason
- identify the main readability bottleneck rather than listing only cosmetic issues
- confirm that every finding can be fixed without changing semantics, tests, outputs or measured performance
- route cross-kind discoveries into linked findings
- record deliberate local complexity that should remain because it mirrors a real semantic distinction

## Valid findings

A style finding needs concrete code evidence and one of these impacts:

- slows review or obscures data flow
- makes an API easy to misuse
- hides state or ownership transitions
- mixes responsibilities inside one owner
- uses an abstraction that is broader or cleverer than the task
- creates avoidable naming, layout or control-flow ambiguity
- conflicts with the repository style guide

Do not record purely subjective formatting preferences that `rustfmt` already decides.

## Kind-specific preservation rules

A style fix must preserve:

- accepted and current semantics
- all existing tests unchanged
- diagnostics and source locations
- public interfaces and generated artefacts
- stage and module ownership
- deterministic behaviour
- relevant performance baselines

A style finding cannot authorise a compatibility layer, a new cross-stage abstraction or a test rewrite.

## Freshness invalidators

Mark a style audit stale when the scope receives material changes to:

- module or file organisation
- public or important internal APIs
- principal data structures or context objects
- control-flow shape in substantial functions
- naming conventions across the scope
- lint, macro or unsafe-code policy

Small bug fixes or generated-only changes do not automatically stale the whole style audit unless they materially alter the reviewed code shape.

## Completion checklist

A complete style audit confirms that:

- every production file and the module entry point were read
- module responsibility and public surface were checked
- data shape and data-oriented design were reviewed
- important APIs and functions were reviewed
- naming, imports, layout, lints and exceptional paths were reviewed
- local copy and ownership noise was checked without unsupported performance claims
- comment, redundancy, correctness, diagnostic, test and performance concerns were routed out of lane
- every finding names a concrete correction and preserved invariants
