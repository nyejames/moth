# Correctness Audit

Read the [Codebase Audit Guide](../audit-guide.md) before using this guide. Correctness is judged against the routed canonical authorities and the [progress matrix](../../src/docs/progress/@page.moth), not against current implementation behaviour alone.

A correctness audit is read-only. It records supported semantic defects, broken internal invariants, invalid stage ownership, unsafe user-driven failure paths and inconsistent producer-consumer contracts. It does not authorise design changes, test changes or implementation of deferred features.

## Purpose and boundary

Use this audit to answer whether the selected scope accepts, rejects, transforms and hands off data exactly as the accepted current contract requires.

The audit covers:

- supported language, compiler, build and project semantics
- internal state and transformation invariants
- producer-consumer handoffs
- user-input safety
- deterministic behaviour
- identity, indexing and remapping correctness
- error-class and partial-result correctness
- concurrency, caching and invalidation correctness
- backend-neutral and target-validation boundaries
- borrow, lifetime and ownership legality where routed by the scope

Route these concerns elsewhere:

- wording, source labels, recovery quality or cascades -> Diagnostics
- missing or weak regression coverage -> Tests
- code shape without a behavioural defect -> Style
- repeated or obsolete implementation paths -> Redundancy
- measured speed or memory cost -> Performance
- stale status or canonical prose -> Documentation

## Valid scopes

- A leaf scope is valid when the owner can be checked against explicit inputs, outputs and invariants.
- A composite scope is valid for end-to-end subsystem behaviour that no leaf owns alone.
- A contract scope is the default for producer-consumer handoffs, stage boundaries and cross-module artefacts.
- A comparison scope is valid only when parity itself is a contract, such as backend-independent validation or equivalent target behaviour.
- A single file can produce partial findings but cannot normally mark a leaf correctness audit complete.

A complete correctness audit must inspect the primary implementation, the required producer and consumer context and every current contract relevant to that scope.

## Audit procedure

### 1. Define the supported contract

Before reading implementation details:

- identify the exact canonical documents that own the behaviour
- read the progress-matrix rows for current status and backend coverage
- read active roadmap plans that may deliberately leave the surface partial or replace it soon
- separate accepted end-state design from currently supported behaviour
- list explicit exclusions, deferred edges and experimental paths
- identify tests that form the immutable executable baseline

Record the supported contract in concrete terms:

- valid inputs and required outputs
- invalid inputs and required rejection stage
- success, diagnosed, blocked and internal-failure outcomes
- ownership of each semantic fact
- deterministic ordering or identity requirements
- target or command restrictions

Do not report missing deferred work as a defect.

### 2. Map inputs, outputs and state transitions

For the primary scope, identify:

- every input type, identity domain and precondition
- every output type, side table, artefact and diagnostic lane
- which data is authoritative and which data is derived
- which owner creates, mutates, freezes, remaps or consumes each value
- valid stage or phase transitions
- whether partial output is permitted
- which consumers assume success-only data
- which local indexes or IDs may cross the boundary

Check that the implementation follows the documented direction of data flow. Later stages must not reopen source or reconstruct facts already owned by earlier stages.

### 3. Enumerate invariants and invalid states

Build a concrete invariant list before tracing code.

Check for invariants around:

- graph shape, ordering and acyclicity
- arena, table, side-table and parallel-vector alignment
- ID provenance and valid comparison domains
- phase, state and root-role transitions
- immutable artefacts and generated sidecars
- success-only versus diagnosed or blocked results
- source locations and remapping
- public versus private identities
- local versus stable cross-module identities
- exact-once compilation, activation or emission
- reachability and root selection
- ownership, alias, transfer and lifetime topology
- output ownership and stale cleanup
- cache keys, fingerprints and invalidation
- target assignments and cross-target edges

For data-oriented structures, verify that the data representation makes invalid combinations difficult to create. Boolean clusters, optional parallel fields and independently mutable tables deserve explicit invariant checks.

### 4. Trace the successful path

Follow representative valid inputs from entry to output.

Check that:

- each fact is computed once by its owner
- retained syntax or semantic data is reused rather than reparsed
- local and cross-module identities remain in the correct domain
- validation runs before consumers that rely on it
- immutable results are not mutated after publication
- downstream stages receive all required context explicitly
- successful artefacts contain no errors
- optional optimisation facts do not affect source legality
- target lowering preserves the backend-neutral contract
- output or runtime activation happens exactly when the builder or command owns it

Trace more than one path when branches differ by module role, target, command, source kind, generic materialisation or failure capability.

### 5. Trace rejected and failure paths

Inventory invalid input classes and internal failure classes.

Check that:

- malformed or unsupported user input cannot panic
- user-authored failures become structured diagnostics
- internal invariant failures use the internal error lane
- diagnosed providers expose no partial interface
- blocked consumers are not semantically compiled
- independent graph branches continue only where allowed
- a failed generated request blocks only its actual consumers unless the contract requires a wider abort
- failure does not leave partially published mutable state
- cleanup, temporary files and output manifests remain coherent after failure
- recovery does not accidentally accept invalid input
- missing mandatory proof is rejected while missing optional optimisation proof falls back conservatively

Diagnostic presentation belongs to Diagnostics. This audit checks the correct failure class, stage and effect on compilation.

### 6. Check stage and ownership boundaries

Search the scope and required context for boundary leaks.

Verify that:

- Stage 0 owns discovery, graph construction and scheduling rather than source semantics
- tokenization and declaration-shell parsing happen once
- interface binding consumes provider interfaces without copying or reparsing private source
- AST owns semantic resolution, folding and generic request creation
- TIR remains AST-local
- HIR is the first backend-facing semantic IR
- borrow and lifetime analyses read validated HIR and write side tables without rewriting HIR
- build and link planning consume compiler-owned facts rather than scanning source or AST
- target validation runs before lowering
- backends do not reconsider source legality or project topology
- output writing stays with the build system

Use the routed authorities for the actual scope. Do not turn this checklist into a claim that every task must inspect every compiler stage.

### 7. Check identity and remapping correctness

Where IDs, indexes or interned data are involved, verify:

- donor-local IDs never escape through public interfaces
- stable identities derive from canonical semantic ownership rather than source order or thread completion
- aliases and re-exports preserve origin identity while changing only their own binding identity
- remapping completes before a consumer uses worker-produced data
- source and string-table deltas merge in canonical order
- cached or serialised forms do not treat process-local IDs or absolute paths as semantic identity
- diagnostic, type, function and package IDs are not compared across unrelated domains
- generated request keys include every semantic input required for uniqueness
- data tables cannot be indexed with IDs from another owner

Look for sentinel IDs, untyped integer indexes and conversions that discard ownership information.

### 8. Check declaration, visibility and dependency correctness

When the scope handles names or graph edges, verify:

- structural provider references, dependency symbol bindings and local ordering edges remain distinct
- one clause or source relationship creates the documented edge only once
- visibility does not bypass module or package facades
- private declarations cannot leak through public semantic surfaces
- collisions are rejected consistently without precedence or shadowing where the language forbids it
- local declaration ordering remains deterministic and does not absorb provider declarations
- diagnosed or absent providers cannot produce partially bound names
- support-package, project-facade and normal-module roles obey their distinct dependency rules
- source-kind resolution cannot silently fall through to another candidate

### 9. Check type and semantic fact correctness

Where types or semantic values are involved, verify:

- semantic decisions use the owning `TypeEnvironment` and `TypeId` domain
- canonical types cross module boundaries instead of donor-local handles or rendered names
- parse-only representations do not drive AST, HIR or backend decisions after semantic resolution
- mutability and access do not manufacture distinct type identities
- contextual coercion occurs only at explicit receiving boundaries
- constant and template folding happen once in the owning module
- generated functions use concrete validated identities and do not mutate base artefacts
- traits, casts and generic evidence are resolved before HIR
- backend checks inspect semantic types through explicit paired environments

Use feature-specific canonical references for any language surface under review.

### 10. Check control flow, ordering and determinism

Verify that:

- source-order guarantees remain stable where required
- graph waves, parallel merges and diagnostic ordering do not depend on worker completion order
- matches and joins cover every valid internal state
- loops, fixed-point worklists and retry paths terminate under documented conditions
- arbitrary iteration limits do not hide missing convergence rules
- entry activation, generated work and output emission happen exactly once
- stable ordering is retained in public interfaces, diagnostics, artefacts and manifests
- hash-map or filesystem traversal order cannot leak into deterministic output unless explicitly normalised
- cancellation or early failure cannot reorder already accepted results

### 11. Check concurrency and shared-state safety

Where the scope uses parallelism, threads or shared caches, verify:

- only independent work runs concurrently
- shared identity assignment remains deterministic
- mutable registries and caches have one clear serial owner or deterministic merge protocol
- worker-local deltas are complete and remapped before publication
- no consumer observes partially initialised data
- locks do not protect a data model that should instead be immutable or stage-local
- duplicate work cannot race to publish competing canonical results
- diagnostic and output order is independent of task completion order
- failure and cancellation leave shared state reusable or deliberately discarded

Do not report lock cost as a performance defect without evidence. This section checks correctness.

### 12. Check caching, reuse and invalidation

Where reuse exists or is planned in the current implementation, verify:

- cache keys contain every semantic and compatibility input
- public-interface changes invalidate semantic consumers
- implementation-only changes relink or regenerate without unnecessary semantic recompilation
- dormant root, runtime dependency and documentation changes invalidate only their documented consumers
- failed or diagnosed artefacts are not reused as successful data
- stale data cannot survive a changed provider identity, config, target capability, ABI or layout
- persistent forms can remap interned strings and paths safely
- incompatible artefacts are discarded rather than partially repaired
- project-specific provenance prevents unsafe cross-project reuse

A missing cache is not a correctness defect. Incorrect reuse is.

### 13. Check memory, borrow and lifetime legality

When routed by the scope, verify:

- borrow validation always runs on validated HIR
- missing inferred transfer proof falls back to borrowing rather than rejecting legal source
- lifetime-region and escape validation remains mandatory and backend-independent
- GC does not legalise invalid topology
- external value-only boundaries do not retain Moth references
- exported access, alias, retention and outlives summaries match the implementation contract
- cross-module analysis consumes summaries rather than opening callee HIR as local control flow
- allocation or group identities do not enter type identity unless the canonical design explicitly says otherwise
- builder lifecycle roots instantiate summaries without changing language validity

Use the memory authorities in full where required. Do not infer memory semantics from backend behaviour.

### 14. Check backend and output preservation

Where the scope reaches targets or artefacts, verify:

- target checks run over explicit reachable roots
- unsupported unreachable private functions do not fail validation
- mixed-target assignments and permitted edges are validated before lowering
- lowerers receive closed, validated plans
- language-owned checked operations are not weakened by target-native behaviour
- JS and Wasm paths preserve the same accepted source semantics
- project builders assemble routes and assets without reopening source semantics
- backends and builders return output records rather than writing final outputs directly
- manifests and stale cleanup cannot delete another builder or profile's files

### 15. Challenge suspicious code paths

Search for correctness risks including:

- `panic!`, `todo!`, `unreachable!`, `.unwrap()` and unchecked indexing on user-influenced paths
- ignored `Result`, `Option` or diagnostic values
- fallback branches that silently accept unknown states
- duplicate validators with different rules
- stale feature flags or compatibility paths
- `unsafe` blocks with unenforced assumptions
- sentinel values and magic indexes
- comments claiming an invariant that code does not enforce
- partial object construction followed by fallible work
- cloned or reconstructed semantic facts that can drift from their owner
- `HashMap` iteration feeding deterministic output
- temporary limits, retries or convergence caps with no contract

Each suspected defect needs a trace to an observable result or broken invariant.

### 16. Compare against tests without changing them

Read relevant tests as evidence and baseline.

Check whether:

- tests encode the canonical current contract
- implementation paths exist that bypass the tested owner
- equivalent inputs reach untested alternate branches
- the observed implementation contradicts a passing test because the assertion is too weak
- tests and docs conflict

Do not change or recommend changing tests inside the correctness finding. Create a linked Tests finding when coverage or an incorrect expectation needs separate action.

### 17. Form the finding

A correctness finding must state:

- the exact supported contract or internal invariant
- the implementation path that violates it
- a reproducible input, state trace or proof where possible
- the earliest correct owner of the fix
- affected consumers and boundaries
- why the issue is not accepted deferral or design-pending work
- required linked Diagnostics, Tests or Documentation findings
- preservation requirements for all unaffected behaviour

A design preference or missing future feature is not a correctness finding.

## Valid findings

Valid correctness findings include:

- accepting invalid supported input
- rejecting valid supported input
- emitting an internally inconsistent successful artefact
- exposing partial interfaces after diagnosis
- using the wrong semantic owner or identity domain in a way that can change behaviour
- user-driven panic or internal corruption
- nondeterministic identity, diagnostic or output behaviour
- invalid cache reuse or incomplete invalidation
- cross-stage reconstruction that can diverge from the owned fact
- backend behaviour that weakens the language contract
- borrow or lifetime legality that differs by backend

## Kind-specific preservation rules

A correctness fix must preserve:

- all correct accepted and rejected behaviour
- every existing test unchanged unless a linked Tests finding is accepted
- diagnostic quality and stable identity unless a linked Diagnostics finding is accepted
- current deferred and experimental boundaries
- performance unless an explicit measured trade-off is approved
- deterministic identities, ordering and output
- architecture ownership outside the accepted fix scope

A fix that makes the suite pass by changing the contract, test or progress matrix is invalid.

## Freshness invalidators

Mark a correctness audit stale when the scope receives material changes to:

- supported semantics or progress-matrix status
- principal algorithms or state transitions
- producer-consumer interfaces
- identity, remapping or ordering logic
- error classification or recovery behaviour
- caching, invalidation or concurrency
- target validation or backend handoff
- borrow, lifetime or ownership analysis

A local style, comment or documentation-only edit does not stale correctness unless it changes the interpreted contract.

## Completion checklist

A complete correctness audit confirms that:

- the current supported contract and deferrals were established
- inputs, outputs, states and invariants were mapped
- success and failure paths were traced
- stage ownership and handoffs were checked
- identity, ordering, concurrency and reuse were checked where relevant
- memory and target boundaries were checked where relevant
- user-input panic paths and suspicious fallbacks were searched
- relevant tests were read without changing them
- every finding names an exact violated contract and root owner
- diagnostic, test, performance and documentation issues were routed into linked findings
