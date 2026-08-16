# Diagnostics Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. The routed architecture and style authorities define diagnostic ownership and lanes. The [testing guide](../../src/docs/codebase/style-guide/testing.mtf) defines assertion policy, but tests remain read-only during this audit.

A diagnostics audit is read-only. It records problems in user-error identity, source context, message usefulness, recovery, cascade control, warning policy and deterministic rendering. It assumes accepted source legality unless a linked Correctness finding proves that acceptance or rejection itself is wrong.

## Purpose and boundary

Use this audit to answer whether a user receives the right structured failure, at the right owner, with enough stable context to understand and fix the problem.

The audit covers:

- `CompilerDiagnostic` versus `CompilerError` lane selection
- stable codes, typed payloads and reason identities
- primary and secondary source locations
- source remapping and render context
- message specificity and practical suggestions
- recovery and cascade behaviour
- warning ownership and success-only retention
- deterministic ordering and multiplicity
- centralised diagnostic construction
- target, backend, build and tooling error boundaries

Route these concerns elsewhere:

- wrong acceptance or rejection -> Correctness
- missing or weak diagnostic assertions -> Tests
- repeated diagnostic code with no user-facing effect -> Redundancy
- code readability around error handling -> Style
- public diagnostic documentation -> Documentation
- measured diagnostic-path cost -> Performance

## Valid scopes

- A leaf scope is valid when it owns a coherent diagnostic family or validation stage.
- A composite scope is valid for end-to-end user-error flow across several local owners.
- A contract scope is valid for source preparation to semantic validation, worker to renderer remapping or compiler to build-system message handoff.
- A comparison scope is valid for equivalent source forms, targets or commands that should report consistent diagnostic identity.
- A single file can produce partial findings but does not normally complete a diagnostic family audit.

A complete audit must inspect diagnostic creation, propagation, accumulation and final rendering for the selected scope.

## Audit procedure

### 1. Inventory user-visible failure surfaces

List every invalid or unsupported condition the scope can detect.

Include, where relevant:

- malformed source syntax
- dependency, path, module and package violations
- config and build-input failures
- name, type, trait, cast and constant failures
- HIR, borrow and lifetime source failures
- target capability failures
- warning conditions
- output conflicts and user-correctable project errors
- deferred-feature and outside-scope rejection

For each condition, identify:

- the earliest owner with enough semantic context
- the expected diagnostic family and code
- the primary source location
- useful related locations
- whether recovery should continue
- whether downstream consumers should be blocked silently

Do not invent new diagnostic ownership merely to improve wording.

### 2. Verify diagnostic lane selection

Check that:

- source, config, dependency, type, rule, borrow and target-contract failures use `CompilerDiagnostic`
- impossible compiler states and infrastructure failures use `CompilerError`
- filesystem errors are classified by whether the user can correct project input or the tool itself failed
- user input cannot reach `panic!`, `todo!`, `unreachable!` or user-data-driven `.unwrap()`
- transformation failures caused by invalid internal state do not masquerade as source diagnostics
- deferred-feature diagnostics remain distinct from outside-design-scope diagnostics
- warnings do not become errors without an explicit policy owner
- diagnosed module results contain no partial interface

A wrong lane is a diagnostic finding when source legality is otherwise clear. Create a linked Correctness finding when the wrong lane also changes compilation outcome.

### 3. Check stable identity

For every diagnostic family, verify:

- the code identifies one durable semantic family
- an existing code is not reused for a different failure
- typed payload or reason enums carry the cause rather than pre-rendered strings
- reason keys are compiler-owned and not reconstructed by tests or renderers
- semantic identities such as `TypeId`, symbols or package IDs remain structured until rendering
- message wording can improve without changing diagnostic identity
- repeated occurrences retain exact multiplicity
- warnings have their own stable identity where tests or tooling consume them

Look for generic catch-all codes that hide materially different correction paths.

### 4. Check construction ownership and centralisation

Inspect where diagnostics are created.

Check that:

- construction occurs at the owner with the best semantic context
- repeated construction uses one typed constructor or narrow owner-local helper
- callers do not build near-identical prose independently
- renderers do not infer semantic reasons from text
- consumers do not reopen source or earlier IR only to recreate a better error
- low-level helpers return enough structured context for the owning stage to diagnose accurately
- a utility module has not become a second semantic owner for unrelated diagnostics
- diagnostic constructors do not take broad loosely related option bags

When construction is duplicated but user behaviour is currently identical, route the structural issue to Redundancy and keep the diagnostic finding focused on drift risk or observed inconsistency.

### 5. Check source context

For each diagnostic, inspect:

- primary location accuracy
- token or span precision
- path identity and normalisation
- one-based line and display-column behaviour where relevant
- primary label wording
- secondary locations for conflicts, origins, providers, declarations or prior use
- ordering of secondary labels
- cross-file and cross-module context
- generated or synthetic source mapping
- config and provider-backed source locations

Check that every user-facing diagnostic has a useful `SourceLocation`. A broad file-level span may be valid when no narrower source owner exists, but the report should explain why.

### 6. Check remapping and render context

Where compilation uses interned strings, worker-local deltas or remapped identities, verify:

- diagnostic data is remapped before later consumers use it
- source and string-table deltas merge in canonical order
- render context outlives every diagnostic it needs to display
- process-local IDs are not persisted or compared across unrelated boundaries
- paths stay interned until rendering instead of being cloned as display strings
- type names are rendered from semantic identities through the correct local context
- a successful or failed result carries enough self-contained context for later rendering
- parallel completion order cannot alter rendered diagnostics

Any corruption or wrong source attribution caused by remapping is also a Correctness finding.

### 7. Check message content

Read rendered wording or constructor intent for each important family.

Check that messages:

- state what failed in user terms
- name the relevant token, symbol, type, path, field or capability
- distinguish expected and found values where useful
- explain why the source is invalid rather than only that it failed
- suggest a practical correction when one is stable and unambiguous
- avoid exposing internal compiler names, indexes or implementation phases without user value
- use canonical language terminology
- distinguish unsupported, deferred and outside-scope behaviour
- avoid vague phrases such as "invalid syntax" when the owner knows the specific rule
- avoid promising a feature or recovery path that is not accepted
- avoid excessive prose that hides the actionable fact

Do not require a suggestion when several corrections are equally valid or design is pending.

### 8. Check related labels and conflict diagnostics

For duplicate, collision, visibility, alias, ownership and cross-file errors, verify:

- all materially relevant declarations are labelled
- the primary location is the source action the user should change first
- secondary labels identify origins without overwhelming the message
- locations from every conflicting owner are retained where available
- case-only collisions and namespace conflicts explain the shared scope
- public-surface leaks identify both the exported surface and unavailable origin
- borrow or lifetime conflicts identify the active uses or regions needed to understand the error
- order remains deterministic

A diagnostic should not silently pick one of several conflicts and hide the others when the contract requires a complete collision result.

### 9. Check recovery and cascade control

Trace what happens after each diagnostic.

Verify that:

- the stage continues only when remaining work can be trusted
- diagnosed providers block consumers without repeating the provider's root error
- independent branches continue where the build contract permits it
- recovery does not fabricate placeholder semantic facts that later look valid
- one malformed construct does not create an avoidable cascade of unrelated errors
- intentionally independent errors remain visible rather than over-suppressed
- contains-style test matching is used only for documented independent recovery
- recovery order is deterministic
- warnings are retained only on successful artefacts where required
- a module or request that exposes no partial result cannot leak one through another lane

Distinguish a noisy cascade from multiple genuinely independent failures.

### 10. Check command and target consistency

Compare equivalent invalid source through relevant commands and targets.

Check that:

- `check`, `build` and `dev` use the same compiler-owned semantic diagnostic identities where they share validation
- target-specific failures occur before lowering
- unsupported unreachable code does not produce target failures when the contract excludes it
- JS and Wasm validation report the source feature and assigned target clearly
- build-system context adds project or entry information without replacing the underlying compiler diagnostic identity
- output conflicts and manifest ownership errors name the path, builder and profile involved
- tooling overlays add diagnostics without duplicating target or source semantics

A command may add context or policy, but it should not reimplement the semantic rule.

### 11. Check warnings

Inventory warnings owned by the scope.

Verify that:

- warning identity is structured and stable
- warning emission is deterministic
- warnings are not emitted from failed artefacts unless the architecture explicitly permits it
- warning policy belongs to the command or test expectation rather than the semantic constructor
- duplicate warnings are not emitted by shared modules or repeated consumers
- warning source locations and suggestions remain useful
- warnings do not report accepted deferred behaviour as an error or hide an actual error as advisory
- suppression or ignore policy is explicit and testable

### 12. Check test expectations without editing them

Read relevant tests to determine the executable baseline.

Check whether they assert:

- exact diagnostic codes and multiplicity
- reason identity where one code has several semantic causes
- primary and secondary source locations where location is part of the contract
- wording only when prose itself is intentionally contractual
- warning policy separately from errors
- deterministic ordering only where order is owned

Record a linked Tests finding for missing, weak or stale assertions. Do not change expectations in the diagnostics finding.

### 13. Search for common diagnostic smells

Search the scope for:

- `format!` or owned strings embedded in semantic payloads
- ad hoc error text returned from low-level helpers
- `CompilerError` variants for user-authored failures
- source diagnostics without locations
- duplicated constructors
- broad generic "invalid" variants
- rendered type names stored instead of semantic IDs
- cloned `PathBuf`s in diagnostic payloads
- code-based branching in renderers that recreates semantic meaning
- panic paths reachable from malformed input
- diagnostics merged in hash-map or task-completion order
- tests that use generic "must fail" assertions
- TODO diagnostics or placeholder messages

### 14. Form the finding

A diagnostics finding must state:

- the invalid or warning condition
- current and expected diagnostic family
- the owner that has enough context to construct it
- affected codes, reasons, locations and recovery behaviour
- evidence from constructors, propagation or rendered output
- whether source legality remains unchanged
- linked Correctness or Tests findings where required
- exact preservation requirements for unaffected diagnostic families

## Valid findings

Valid diagnostics findings include:

- wrong error lane
- missing or unstable diagnostic identity
- inaccurate or missing source location
- misleading, vague or internally worded message
- failure to label related source origins
- avoidable cascades or over-suppression
- duplicate diagnostics from shared work
- nondeterministic ordering or multiplicity
- renderer reconstruction of semantic meaning
- stale or conflicting target and command wording
- user-driven panic on invalid input

## Kind-specific preservation rules

A diagnostics fix must preserve:

- which source programs are accepted and rejected
- every existing test unchanged unless a linked Tests finding is accepted
- stable codes not explicitly identified as incorrect
- source and semantic ownership
- deterministic ordering
- unaffected warning and recovery behaviour
- relevant performance baselines

A wording improvement cannot silently repurpose a stable code or change source legality.

## Freshness invalidators

Mark a diagnostics audit stale when the scope receives material changes to:

- diagnostic constructors, payloads, codes or reason enums
- source locations, remapping or render context
- validation ownership or recovery flow
- command, target or warning policy
- renderer wording for the reviewed families
- syntax or semantic rules that alter invalid-form classification

An unrelated successful-path refactor does not automatically stale the diagnostic audit.

## Completion checklist

A complete diagnostics audit confirms that:

- every user-visible failure and warning family in scope was inventoried
- lane, identity, construction owner and source context were checked
- rendering, remapping and deterministic ordering were checked where relevant
- message usefulness, related labels, recovery and cascades were reviewed
- command and target consistency were checked where relevant
- tests were read without editing them
- panic, ad hoc prose and duplicate-construction smells were searched
- correctness, test, redundancy and documentation concerns were routed into linked findings
