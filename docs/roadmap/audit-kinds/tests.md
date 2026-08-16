# Tests Audit

Read the [Codebase Audit Guide](../audit-guide.md) and the complete [testing standards](../../src/docs/codebase/style-guide/testing.mtf) before using this guide. The testing standards own test selection, location, assertions, fixture policy and pruning. This document defines the audit procedure.

A tests audit is read-only. It may recommend additions, rewrites, moves, merges or removals inside test-owned surfaces. It does not authorise production-code changes or redefine accepted behaviour to match the current compiler.

## Purpose and boundary

Use this audit to answer whether each supported behaviour and real internal invariant has one clear, strong and maintainable regression owner.

The audit covers:

- behaviour-to-test ownership
- integration, unit, harness and backend coverage
- positive, negative, boundary and adversarial cases
- assertion strength and contractual precision
- fixture and manifest policy
- diagnostic, runtime and artefact assertions
- cross-backend parity
- redundant, obsolete or implementation-shaped tests
- test support and harness correctness
- coverage claims in the progress matrix

Route these concerns elsewhere:

- production behaviour that violates the contract -> Correctness
- user-error quality rather than test strength -> Diagnostics
- duplicated production implementation -> Redundancy
- test runtime cost with measured evidence -> Performance
- stale progress or testing documentation -> Documentation
- local readability of production code -> Style

## Valid scopes

- A leaf scope is valid when its public behaviour and hidden invariants can be mapped to all test owners.
- A composite scope is valid for subsystem, command or project behaviour spanning several modules.
- A contract scope is valid for producer-consumer handoffs and stage-boundary invariants.
- A comparison scope is valid for backend parity, equivalent source forms or repeated test families.
- A test directory alone is not a complete semantic scope. Audit the behaviour owner plus every relevant test surface.

A complete audit must inspect production contracts, existing tests, manifest metadata, support utilities and relevant progress-matrix coverage claims.

## Audit procedure

### 1. Build the contract inventory

Before counting tests:

- identify the current supported behaviour from canonical docs and the progress matrix
- separate supported, partial, experimental and deferred surfaces
- list user-visible success and failure contracts
- list target, command, source-kind and module-role variations
- list hidden internal invariants that cannot be observed through normal output
- identify existing contract IDs, primary cases and coverage notes
- identify active plans that may intentionally replace or extend the surface

Do not treat line coverage, test count or a broad progress label as proof of semantic coverage.

### 2. Map each contract to one primary test owner

For every behaviour or invariant, identify its primary owner:

- user-visible language or project behaviour -> integration case under `tests/cases/`
- pure data or local invariant -> focused unit test near the owner
- stage-boundary orchestration -> minimal pipeline or build smoke test
- end-to-end or multi-module Rust harness behaviour -> `src/compiler_tests/`
- backend artefact behaviour -> backend-specific assertions or contractual goldens
- hidden side-table or IR fact -> focused unit test only when external behaviour cannot expose it
- cross-backend semantic parity -> one integration input with backend-specific expectations

Check that secondary coverage exists only for a distinct boundary, target, diagnostic lane or hidden invariant.

Record:

- contracts with no primary owner
- contracts with several competing primary owners
- tests whose apparent owner does not match the behaviour they protect
- tests stored beside convenient implementation rather than the semantic owner

### 3. Audit integration-suite inventory and policy

Inspect the canonical integration manifest and audit inventory.

Check that:

- every case has a unique stable ID and path
- tags are present, meaningful and unique within the case
- every case has a valid role
- every non-smoke case has a contract
- each contract has at most one primary case
- primary cases always name a contract
- whole-case acceptance-only fixtures use the smoke role
- helper files remain inside the owning case and do not become accidental cases
- manifest order is intentional and deterministic
- paths remain relative, normalised and contained inside the suite root
- retained metadata is consumed directly rather than reconstructed from names or paths
- hard-policy findings are not bypassed by filtered execution
- the suite audit report accurately describes the configured assertions

Use the repository's suite-audit command when executing an audit, but do not treat its inventory as a substitute for reading the cases.

### 4. Check positive-path coverage

For each supported contract, check that tests cover:

- ordinary successful use
- the minimal valid form
- representative non-trivial use
- cross-file or cross-module use where visibility or identity matters
- interaction with defaults, aliases, generics, traits, templates or source kinds where the contract includes them
- command and target paths that claim support
- runtime output when behaviour is observable only after execution
- emitted artefacts when structure is part of the contract
- deterministic ordering or exact-once activation where relevant

Avoid multiplying near-identical happy-path fixtures. One strong primary case may own several closely related syntax forms when the contract is genuinely one behaviour.

### 5. Check negative and boundary coverage

For each supported rule, inspect coverage for:

- malformed syntax
- invalid placement or scope
- wrong namespace or visibility
- duplicate or conflicting declarations
- type mismatch and unsupported type shapes
- missing provider, unavailable symbol or facade bypass
- boundary values and empty forms
- overflow, capacity, indexing or checked-failure edges
- invalid control-flow joins and terminality
- borrow, alias, transfer and lifetime conflicts
- unsupported reachable target features
- user-input paths that must diagnose rather than panic
- deterministic conflict ordering and related locations

A failure case should prove the actual contract through stable diagnostic identity and necessary source context, not merely that compilation failed.

### 6. Check interaction and adversarial coverage

Look for combinations likely to bypass the simple path:

- nested modules, support packages and project facades
- aliases plus collisions
- multiple providers or source kinds with the same stem
- generated functions across module or package boundaries
- templates with wrappers, slots, branches, loops and runtime handoff
- control-flow joins after partial success or diagnosis
- parallel graph branches and deterministic merge
- config, project globals and entry-local settings
- JS and Wasm target partition edges
- output ownership conflicts and stale cleanup
- cache or incremental invalidation boundaries
- copy, alias, mutation and final-use combinations

Use adversarial cases only when they protect a distinct root cause. Do not create combinatorial fixtures without a clear contract.

### 7. Review unit and subsystem tests

For each unit test, check that:

- it names the invariant it protects
- the invariant belongs to the local subsystem
- external output cannot protect the same rule more directly
- the test does not freeze incidental internal layout
- setup is smaller than the behaviour under test
- assertions inspect the narrowest authoritative data
- private helper behaviour is not tested independently when the public operation already owns it
- obsolete API shapes are not preserved for test convenience
- test-only constructors do not bypass real invariants
- tests live under the module's test directory, not in production files

Retain unit tests for pure algorithms, impossible states, side-table facts, transfer rules and backend planning policy that cannot be observed from artefacts.

### 8. Review HIR, side-table and internal-IR assertions

Where internal semantic data is tested, verify that:

- the asserted representation is itself the contract or required invariant
- semantic IDs are compared in the correct local domain
- tests do not depend on incidental numeric ID assignment
- stable identities are used across module boundaries
- HIR assertions focus on semantic operations rather than formatting or debug text
- borrow, lifetime and link side-table assertions name the fact they protect
- generated functions and base artefacts remain separate where required
- assertions do not make an optimisation fact affect source legality
- derived views are not treated as a second semantic authority

Prefer a user-visible integration case when the same defect can be observed through accepted source behaviour.

### 9. Review successful backend intent

For every successful backend block, check that it has a deliberate contract beyond the universal baseline.

Verify that:

- acceptance-only is used only when no stronger case-specific assertion exists
- a whole acceptance-only case uses the smoke role
- acceptance-only is not combined with runtime, artefact, golden, absence or expected-warning assertions
- target support matches the progress matrix
- one source input is used for cross-backend parity where possible
- backend-specific expectations protect actual target differences rather than duplicate the same fixture
- unsupported target features have explicit failure coverage when reachable
- successful unreachable private code does not create false target expectations

### 10. Review runtime-output assertions

When runtime behaviour matters, check that the assertion matches the owned contract:

- use exact combined output for small deterministic results
- use ordered fragments when chronology is the contract
- use exact-once fragments for activation, mounting or helper duplication
- use required and forbidden fragments when unrelated output may coexist
- do not reconstruct channel order after execution
- do not make incidental scheduler turns or microtask counts contractual
- assert reactive or runtime updates only at supported sinks
- prefer runtime output over generated JavaScript text when execution is the actual behaviour

Check that expected output is specific enough to fail on duplicated, missing, reordered or stale behaviour.

### 11. Review diagnostic and warning assertions

For each failure or warning case, check that:

- stable diagnostic codes are asserted
- multiplicity is exact unless independent recovery justifies contains mode
- reason identity is asserted when one code has several causes
- primary and secondary locations are asserted where source mapping is part of the contract
- message fragments are used only when wording itself matters
- full rendered snapshots are avoided when structured assertions are enough
- generic "must fail" assertions are absent
- contains mode has a substantive reason
- warnings use explicit forbid, ignore or exact-code policy
- error and warning contracts remain separate

A test audit may recommend expectation changes when canonical documentation proves them wrong. It must not use current implementation behaviour as the reason.

### 12. Review artefact, absence and golden assertions

Check that:

- artefact assertions inspect the narrowest output that owns the behaviour
- presence checks are paired with absence checks where accidental emission is a risk
- goldens protect meaningful complete structure rather than broad incidental formatting
- runtime behaviour is not inferred only from emitted source text
- target-specific files, imports, helpers, assets and manifests are asserted where relevant
- output order and deduplication are tested when contractual
- stale output cleanup and ownership conflicts have focused coverage
- golden updates cannot hide unrelated changes
- the same logical input uses backend-specific expectations rather than duplicate directories where possible

### 13. Review fixtures and test support

Inspect fixture design and helpers.

Check that:

- each scenario is self-contained
- temporary paths use isolated directories and deterministic cleanup
- symlink, platform and path-normalisation cases are explicit where relevant
- shared test utilities have real multi-owner use
- one or two callers keep support local
- helpers do not construct impossible compiler state unless the test explicitly targets an invariant failure
- test setup uses real entry points where practical
- mocks and fakes do not bypass the contract under test
- fixtures use current Moth syntax and project structure
- stale files, goldens and unused helpers are removed through an accepted finding

### 14. Check redundant and obsolete coverage

Group tests by contract and root cause.

For each group, decide whether to:

1. retain one primary case
2. retain a secondary case for a distinct boundary or target
3. merge expectations into one stronger scenario
4. replace brittle unit coverage with integration coverage
5. remove obsolete coverage after the current owner is protected elsewhere
6. leave similar cases separate because they exercise different semantic owners

Look for:

- fixtures that differ only in names or formatting
- unit tests duplicated by stronger integration cases
- old API or syntax retained only in tests
- multiple goldens asserting the same artefact
- smoke cases that became redundant after a stronger primary case landed
- tests for deleted compatibility paths
- ignored or permanently disabled tests

Do not remove coverage merely to reduce suite size. Name the surviving primary owner.

### 15. Check missing coverage from progress and findings

Compare the suite with:

- progress-matrix coverage labels and notes
- accepted implementation plans
- previous correctness and diagnostic findings
- recent bug fixes
- stage and backend contracts

Verify that:

- broad coverage claims have multiple meaningful success and failure dimensions
- targeted or thin areas have an explicit reason or open finding
- every fixed user-visible defect has a regression owner
- every new internal invariant has focused coverage only when needed
- deferred features are not represented as supported success cases
- experimental cases are labelled and isolated appropriately

A stale progress entry becomes a linked Documentation finding.

### 16. Check the test harness and policy owner

When the scope includes test infrastructure, verify that:

- parsing and validation of manifests have one owner
- list, audit and execution modes use the same hard-policy evaluator
- selection filters preserve canonical order
- exact IDs, tags, contracts and backends are retained as typed metadata
- audit output describes what execution will enforce
- filtering cannot hide invalid suite policy
- runtime harness ordering matches the documented event model
- golden and artefact loaders reject unsafe paths and ambiguous state
- failure reporting distinguishes fixture, compiler and infrastructure errors
- parallel test execution preserves deterministic results

### 17. Exclude non-tests from correctness coverage

Confirm that:

- benchmark fixtures are not counted as correctness tests
- examples and documentation snippets are not treated as regression owners unless an explicit executable docs gate owns them
- manual testing is not recorded as durable coverage
- compiler acceptance without an authored assertion is not treated as a semantic contract except deliberate smoke intent
- code comments are not used as proof that an invariant is tested

### 18. Form the finding

A tests finding must state:

- the exact behaviour or invariant
- the current primary and secondary test owners
- the gap, duplication or weakness
- the canonical source of truth
- the proposed test location and role
- the narrowest assertion that proves the contract
- any cases to merge, move or remove
- production behaviour that must remain unchanged
- linked Correctness, Diagnostics or Documentation findings

## Valid findings

Valid tests findings include:

- supported behaviour with no regression owner
- several competing primary owners
- failure cases that assert only generic failure
- success cases with baseline-only intent but no deliberate smoke classification
- brittle implementation-shaped assertions
- missing backend, runtime, artefact or diagnostic dimensions
- duplicate fixtures with no distinct contract
- obsolete tests preserving deleted APIs or syntax
- harness policy that differs between audit, list and execution
- progress coverage claims unsupported by the suite

## Kind-specific preservation rules

A tests fix must preserve:

- canonical semantics and current support boundaries
- production code and generated artefacts
- meaningful existing regression protection
- one clear primary owner per behaviour
- stable diagnostic and runtime contracts unless separately corrected
- suite determinism and safety

Tests must not be weakened to match current implementation. Removing a test is valid only when its contract is obsolete or fully protected by a named stronger owner.

## Freshness invalidators

Mark a tests audit stale when the scope receives material changes to:

- supported behaviour or progress status
- integration cases, manifest metadata or expectations
- unit, harness or backend tests
- diagnostic, runtime or artefact assertion policy
- test utilities or suite infrastructure
- target coverage
- contract IDs or primary-owner assignments

A production refactor that preserves every tested contract does not automatically stale the tests audit unless it creates new untested paths or removes the invariant the tests claimed to own.

## Completion checklist

A complete tests audit confirms that:

- current contracts and deferrals were inventoried
- each behaviour and hidden invariant has one primary owner
- integration manifest and suite policy were checked
- positive, negative, boundary and adversarial dimensions were reviewed
- unit, HIR, side-table, backend, runtime, diagnostic and artefact assertions were reviewed where relevant
- fixtures, utilities, redundancy and obsolete coverage were checked
- progress-matrix coverage claims were compared with real tests
- benchmark fixtures and manual checks were excluded from correctness coverage
- every recommendation names the surviving owner and exact assertion
- production, diagnostic and documentation defects were routed into linked findings
