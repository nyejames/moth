# Redundancy Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. This audit applies the repository's ownership, one-current-path and data-oriented design rules to repeated work, obsolete code, misplaced abstraction and unnecessary line count.

A redundancy audit is read-only. It records structural improvements that preserve semantics, tests, diagnostics, outputs and relevant performance. It does not authorise broad redesign merely because two pieces of code look similar.

## Purpose and boundary

Use this audit to answer whether the same fact, transformation, validation or policy is owned more than once and whether every remaining layer earns its cost.

The audit covers:

- duplicated or near-duplicated functions and control flow
- repeated scans, parsing, reconstruction and conversion
- overlapping helpers, validators, registries and data types
- legacy paths, compatibility layers and obsolete scaffolding
- unnecessary wrappers, indirection and pass-through APIs
- wrong-layer or premature abstractions
- mixed-responsibility modules and over-broad utilities
- redundant state, fields, caches and intermediate representations
- unjustified line count and boilerplate
- data-oriented consolidation around one explicit owner

Route these concerns elsewhere:

- code that is merely hard to read -> Style
- observable semantic defect -> Correctness
- missing or stale comments -> Comments
- missing or duplicate test coverage -> Tests
- measured runtime or memory cost -> Performance
- stale docs or implementation maps -> Documentation
- diagnostic user experience -> Diagnostics

## Valid scopes

- A leaf scope is valid for local duplication, stale code and abstraction shape.
- A composite scope is the default for repeated work across one subsystem.
- A contract scope is valid for producer-consumer reconstruction and pass-through layers.
- A comparison scope is the default for independent owners that may repeat policy or machinery.
- A single file may produce partial findings but cannot prove that a helper should be shared beyond its owner.

A complete redundancy audit must inspect the primary scope, declared comparison scopes and adjacent owners named as required context.

## Audit procedure

### 1. Map the current owners and data flow

Before searching for similar text:

- identify the semantic or orchestration facts owned by the scope
- identify the producer and every consumer
- identify authoritative data and derived views
- identify passes, validators, registries, caches and adapters
- identify current public and internal API surfaces
- identify active roadmap work that may already replace part of the scope
- identify deliberately separate target or source-kind implementations

Draw the shortest accurate data-flow outline. Duplication is meaningful only relative to ownership and behaviour.

### 2. Search for duplicated functions and control flow

Search the primary and comparison scopes for:

- functions with equivalent names or signatures
- repeated match arms and validation branches
- near-identical constructors or error mapping
- repeated loops over the same data for the same purpose
- copied state machines or fixed-point logic
- target-specific functions whose only difference is a small policy value
- test and production helpers that independently implement the same parser or normaliser
- code copied during refactors and never consolidated or deleted

For each similarity, compare:

- inputs and preconditions
- output and side effects
- failure lane
- owner and lifecycle
- ordering and determinism
- target or feature policy
- likely future divergence

Textual similarity alone is not enough.

### 3. Search for repeated semantic work

Look beyond duplicate functions for work performed more than once.

Check for:

- source rescanning or lightweight parsing after the owning parse phase
- visibility, dependency or project topology reconstructed downstream
- constants, templates, types or traits resolved more than once
- public surfaces copied into consumers instead of bound by stable identity
- HIR or AST reopened to recreate link, effect, borrow or lifetime facts already owned elsewhere
- reachability recalculated by several consumers from raw bodies
- backend helpers rediscovered from emitted code rather than link facts
- diagnostics independently reconstructed from the same semantic condition
- output or asset dependencies scanned from rendered strings after semantic path tracking
- generated requests deduplicated at more than one layer
- repeated normalisation of paths, names, numbers or source locations

A repeated cheap operation may still be valid when retaining another representation would create worse ownership. Explain the trade-off before filing a finding.

### 4. Search for duplicated state and representations

Inventory types and fields that describe the same concept.

Check for:

- parallel structs representing old and new API shapes
- copied provider, module, type or function metadata with overlapping authority
- both local and global registries owning the same identity
- derived summaries stored as if they were authoritative
- boolean or option fields encoding a state already represented by an enum
- data copied into several contexts because ownership is unclear
- shadow maps, side tables or lookup caches that can drift from the owner
- parse, AST, HIR and backend representations retaining facts beyond their stage
- compatibility aliases and wrappers that preserve obsolete names
- separate error types carrying the same structured diagnostic data

Prefer one authoritative data record plus narrow derived indexes. Do not collapse semantically distinct lanes merely to reduce type count.

### 5. Search for legacy and obsolete paths

Use repository search, history context and active plans to find:

- compatibility wrappers and forwarding functions
- deprecated or alternate entry points
- old API adapters
- stale structs, fields, variants and trait methods
- fallback paths used only by deleted callers
- obsolete feature flags and `cfg` branches
- dead code and unjustified `allow(dead_code)`
- old target or builder paths retained beside the current path
- duplicated migration state
- stale TODO implementations
- tests, fixtures or goldens for removed behaviour
- comments and docs naming deleted systems

Production deletion belongs to this lane. Test removal and documentation correction require linked Tests or Documentation findings.

### 6. Review abstraction ownership

For every shared helper, trait, utility module and generic component, ask:

- is the behaviour actually identical?
- is there one clear owner?
- do all callers depend on that owner naturally?
- does the abstraction expose the semantic distinction callers need?
- does it reduce total control-flow complexity rather than move it?
- can it remain stage-local?
- will a common utility hide where a rule belongs?
- is the generic surface larger than the real set of use cases?
- does dynamic dispatch or a trait hierarchy obscure concrete data flow?
- does the abstraction force conversions or wrapper types at every call site?

Reject abstractions created only because two functions share a few lines.

### 7. Prefer data-oriented consolidation

When consolidation is valid, prefer one explicit data owner and narrow operations over object-style indirection.

Check whether the better shape is:

- one canonical table or arena plus stable IDs
- one immutable artefact consumed by several readers
- one stage-local context containing data used together
- one enum carrying meaningful state instead of parallel booleans
- one pass that produces all related side-table facts
- one policy table consumed by target-specific code
- one data-driven dispatcher instead of several copied matches
- one conversion at the boundary rather than conversions in every consumer
- one registry with explicit origin and backing axes rather than class-like subtype trees

Do not force structure-of-arrays, arena storage or table-driven logic when the data set is tiny or the resulting code is less clear. Data-oriented design follows access and ownership patterns.

### 8. Review indirection and wrapper layers

Trace important calls from entry point to actual work.

Look for:

- one-line forwarding functions with no ownership, validation or policy
- wrapper structs around one value with no semantic identity
- context objects that merely pass another context unchanged
- adapters translating between equivalent shapes
- traits implemented by one concrete type with no justified boundary
- registries that contain one hard-coded entry and no accepted extension surface
- builder or backend layers that only rename compiler-owned data
- helper modules that hide a direct call behind several re-exports
- callback or closure layers where explicit sequential flow is clearer
- error conversions that add no information

Keep a wrapper when it enforces a real boundary, stable identity, capability, lifetime or invariant. Record that reason if similar wrappers are removed elsewhere.

### 9. Review parameter and result plumbing

Check for:

- long repeated parameter lists that represent one input or context
- functions receiving data they never use
- results carrying fields no consumer reads
- data passed through several layers only to reach one owner
- tuple-heavy return values duplicated across callers
- options and booleans that preserve obsolete modes
- repeated extraction of the same fields at every call site
- ownership transfers or clones caused by poor API shape
- broad contexts that make every helper appear coupled to everything

A context struct is valid when fields share owner and lifetime. Splitting or deepening a module may be better when they do not.

### 10. Review passes and traversals

Inventory every traversal over important stores, graphs, HIR, AST, files and artefacts.

For each traversal, record:

- purpose
- owner
- required data
- output
- ordering
- whether it could produce facts another pass later reconstructs

Look for:

- several passes that can safely produce related facts together
- one pass mixing unrelated concerns that should split despite fewer traversals
- repeated filtering and indexing that one derived index could own
- convergence loops caused by incomplete earlier outputs
- validators that traverse the same shape with overlapping rules
- repeated sorting where canonical order could be retained

Do not merge passes solely to reduce traversal count when it mixes semantic owners, complicates error handling or harms clarity. Performance claims require Performance evidence.

### 11. Review diagnostics and error plumbing for duplication

Check for:

- repeated constructors for the same diagnostic family
- several layers wrapping and unwrapping the same error
- formatted prose duplicated across branches
- both `CompilerDiagnostic` and `CompilerError` versions of one user failure
- repeated source-location cloning or conversion
- diagnostic merge and sort policy implemented in several places

Route user-facing quality issues to Diagnostics. This lane owns structural centralisation only when behaviour remains fixed.

### 12. Review target and source-kind specialisation

Compare JS, Wasm, builder, provider and source-kind paths.

For each similarity, decide whether it is:

- language-owned shared behaviour that should move before target lowering
- compiler-owned policy that both backends should consume
- builder-owned orchestration that no backend should duplicate
- genuinely target-specific lowering that should remain separate
- source-kind preparation that should converge on an ordinary declaration shape
- superficial similarity with different runtime contracts

Do not create a shared backend abstraction that erases meaningful target differences.

### 13. Review module and file boundaries

Identify files or modules that:

- own several unrelated concepts
- repeat private helpers from another sibling
- expose internals to avoid a better submodule boundary
- use a broad `utils` or `common` owner
- have entry points that no longer describe the flow
- retain old directories after ownership moved
- duplicate types across sibling modules

Choose one structural action:

- deepen the current module with private submodules
- extract genuinely shared behaviour into a clear owner
- move behaviour to the stage that owns the fact
- merge tiny artificial layers
- delete obsolete files
- leave local because sharing would be worse

### 14. Review line count and boilerplate carefully

Search for code that can be removed or generated by a clearer data representation.

Check for:

- repeated field-by-field copies
- exhaustive matches that only map equivalent variants
- boilerplate constructors and accessors
- duplicated target or capability tables
- manual delegation
- repeated test setup in production support code
- comments preserving obsolete implementation detail
- defensive branches for states types could exclude

Do not optimise for the smallest diff or fewest lines. A named intermediate, explicit match or narrow type is worth its lines when it improves correctness and review.

### 15. Classify every similarity

Every proposed consolidation must choose one outcome:

1. **Leave local** because the similarity is superficial or owners differ.
2. **Extract locally** into a narrow helper inside one subsystem.
3. **Move to a common owner** that both callers already depend on.
4. **Restructure data flow** so the repeated work disappears without a helper.
5. **Delete** an obsolete or unreachable path.
6. **Merge** artificial wrappers or parallel APIs into one current path.
7. **Split** a broad owner before any sharing decision.

State why the chosen outcome is better than the alternatives.

### 16. Check necessity against accepted design

For every major type, pass, registry or subsystem in scope, ask:

- does current accepted design still require it?
- is it implementing a supported, partial, deferred or abandoned surface?
- does an active plan replace it?
- is it a temporary bridge that should already be gone?
- would deleting it expose a missing required owner elsewhere?
- is its only caller a test, benchmark or compatibility path?

Do not delete code for accepted deferred work unless the roadmap says the current scaffold is obsolete.

### 17. Form the finding

A redundancy finding must state:

- the repeated or obsolete concept
- every affected owner and path
- whether behaviour is identical or only similar
- the current source of authority
- the selected action: remove, merge, move, split, rewrite or leave local
- expected code and data-flow simplification
- why the change does not alter semantics
- linked Tests, Documentation or Performance findings
- required searches proving the old path is fully removed

## Valid findings

Valid redundancy findings include:

- two owners computing the same semantic fact
- reparsing or reconstruction after the owner already produced the data
- compatibility wrappers or parallel APIs in a pre-release codebase
- stale fields, variants, modules or adapters with no current owner
- repeated validators that can drift
- wrong-layer shared helpers
- broad utilities obscuring ownership
- unnecessary conversion and wrapper chains
- files or modules with mixed responsibilities
- line count caused by real structural duplication or obsolete scaffolding

## Kind-specific preservation rules

A redundancy fix must preserve:

- accepted semantics and current support boundaries
- every existing test unchanged unless a linked Tests finding is accepted
- diagnostic identity, source context and recovery
- deterministic ordering and identities
- public and backend artefacts
- relevant performance baselines
- one clear owner for every retained fact

The final implementation must contain one current path. Transitional duplication is not an acceptable completed fix.

## Freshness invalidators

Mark a redundancy audit stale when the scope receives material changes to:

- module or stage ownership
- APIs, data structures or intermediate representations
- passes, validators, registries or helper layers
- target or source-kind implementations
- feature removal or migration
- declared comparison scopes

A local bug fix does not automatically stale redundancy unless it adds, removes or changes a structural path.

## Completion checklist

A complete redundancy audit confirms that:

- owners and data flow were mapped before comparing code
- duplicate functions, state, representations and semantic work were searched
- legacy, compatibility and dead paths were searched
- abstractions, wrappers, parameter plumbing and traversals were reviewed
- data-oriented consolidation options were considered without forcing them
- target and source-kind specialisation was compared carefully
- module boundaries and line-count causes were reviewed
- every similarity was classified as leave, extract, move, restructure, delete, merge or split
- tests, docs, diagnostics and performance concerns were routed into linked findings
- every removal finding requires proof that no old path remains
