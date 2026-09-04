# First-party Core and Builder package programme

## Purpose

Build a useful batteries-included set of first-party Moth packages, starting with JavaScript
implementations while the compiler diagnostics and data-layout work continues in parallel.

The programme completes the common gaps in existing Core and Builder packages before adding broad new
surface area. It also establishes package terminology, implementation boundaries, progress tracking
and validation rules that keep later package work consistent.

This is a coordinating implementation plan. It is not a package API authority. Accepted package
semantics live under `docs/src/docs/packages/**` and in the relevant compiler or build-system
authority. This file records sequencing, implementation knowledge and links to the living package
plans under this directory.

## Current-state capsule

```text
STATUS: active parallel programme
CURRENT_SLICE: Phase 0 - harden package foundations and first-party dependency policy
BLOCKERS: package slices that require unstable shared frontend or diagnostic representations pause
NEXT_ACTION: finish the first-party dependency guard corrections, then create core-text.md
```

Record the active revision, worktree state and validation baseline in untracked working notes when a
phase starts. Do not pin a moving programme to a baseline commit in this file.

## Roadmap position and lifecycle

This programme runs in parallel with the active diagnostics and source-data-layout work. Diagnostics
remain the primary compiler refactor. Package work may proceed only while the current slice stays
inside the merge-isolation rules below.

The main roadmap links only this umbrella plan. Package-specific plans live in
`docs/roadmap/plans/packages/` and are linked from the tracker in this file.

Files in this directory are a deliberate exception to the normal short-lived plan lifecycle:

- the umbrella and package plans may link to one another
- package plans remain as living implementation companions after one version is complete
- completed checklists should be removed or compressed
- durable rationale, quirks, blockers and likely next work should remain
- canonical package documentation remains the semantic authority
- package plans must not become historical changelogs or duplicate the public reference

## Required authorities

Read these from the active worktree before changing package code:

- `AGENTS.md`
- `docs/compiler-design-overview.md`, especially binding-backed symbols, HIR call targets, link facts
  and target-contract validation
- `docs/build-system-design.md`, especially selected builder capabilities, package classification,
  Core and Builder package availability and external JavaScript emission
- `docs/src/docs/packages/`
- `docs/src/docs/progress/@page.moth`
- `docs/src/docs/progress/packages-and-builders/@page.moth`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- the project code-review guide supplied for package audits
- `docs/roadmap/roadmap.md`
- `index.md` as a locator only

Read `docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md` only to
preserve its ownership boundary. This programme must not implement its declaration, alias, resolver
or dependency-package graph work.

The accepted collection sorting plan lives at
`docs/roadmap/plans/packages/core-collections-sorting.md`. Treat that plan as the specialised
implementation companion for sorting and do not reconstruct its contract from this umbrella.

## Scope

This programme owns:

- a narrow audit and hardening pass over existing first-party package foundations
- useful v1 completion work for existing Core packages
- deeper expansion of `@web/canvas`
- secondary `@html` wrapper and helper work required by package additions
- one newly accepted Core package, `@core/json`, after its own design checkpoint
- JavaScript implementations and the smallest additive Rust registration changes they need
- living package implementation plans created when each package becomes active
- package and project-builder progress tracking
- integration-first regression coverage for package behaviour
- an automated first-party no-third-party-dependencies validation rule

## Non-goals

This programme does not own:

- project dependency declarations
- package aliases
- resolver or catalogue design
- package acquisition, registries, fetching, version solving, lockfiles or publishing
- transitive dependency policy
- generic binding-backed functions as a general feature
- a broad external ABI redesign
- Wasm or native implementation of every package
- async, task or event-loop language design
- exhaustive copies of JavaScript or Web Platform APIs
- a new package origin or package importance tier
- a Core cryptography package or cryptography package examples
- broad compiler diagnostic or source-layout refactors
- compatibility wrappers around stale shared compiler shapes

## Confirmed package model

Package origin and implementation backing remain independent.

Accepted origins used by current or already-designed work are:

- `Core` for compiler-owned batteries-included packages
- `Builder` for packages supplied by the selected project builder
- `ProjectLocal` for project-owned package surfaces
- `Dependency` for the future dependency-package role owned by later package-system work

Accepted backing kinds are:

- `MothSource`
- `ExternalBinding`

`Standard` has no accepted role. `PackageOrigin` no longer includes a Standard variant. Core is the
single first-party batteries-included family.

Prelude is visibility policy, not an origin or backing. Bare `io` is a prelude alias to `@core/io`.
The package and its metadata remain `@core/io`.

Use precise terminology:

- say **binding-backed package** for a package implemented outside Moth source
- say **dependency package** for a future separately acquired Moth package
- name the origin when it matters, such as Core or Builder
- use `external package` only when referring to an existing Rust owner whose exact name has not yet
  been safely changed

Do not perform a broad code rename only to clean up wording while diagnostics work is active.

## API and portability rules

### Moth semantics first

Design each public package contract in Moth terms before choosing its JavaScript lowering.

Core package behaviour must be backend-neutral. JavaScript is a supported backend, but it does not
define Core semantics. Deterministic domains such as text, time, dates, parsing and structured data
must produce aligned observable behaviour across backends.

A package may permit backend-local implementation identity where that identity is not part of the
contract. Random generators may use different algorithms in the first JavaScript and future backend
implementations unless a specific reproducible sequence is explicitly promised. Bounds, result
domains, fallibility and other observable rules still need one portable contract.

`@web/*` packages may expose genuine Web Platform concepts. They should still present a curated Moth
surface rather than mechanically clone every JavaScript method.

### Final shape or defer

Current external signature or language limits must not force a temporary public API.

When the correct package contract needs a value shape that Moth or the binding boundary cannot
represent yet:

1. document the intended requirement and blocker
2. keep the operation out of the public package
3. update the package progress row with the useful deferred extension
4. resume after the required language or ABI capability is stable

Do not disguise collections, maps, records or generic values behind awkward opaque handles merely to
ship a JavaScript implementation. Do not turn this rule into permission for a broad generic-external
function project.

### Implementation form

Portable Core packages keep semantic registration in the existing Core package owners. Their
JavaScript implementations stay in the JavaScript package-binding or runtime path.

Web Builder packages may use self-contained annotated JavaScript assets when they model real host
bindings, as `@web/canvas` does.

Do not move Core APIs into annotated JavaScript merely because JavaScript is currently their only
implementation. Do not introduce a common abstraction over both forms until real repeated code
proves that it reduces complexity.

### Completion target

A package slice aims for useful v1 completeness, not API exhaustiveness.

A package is complete for its current slice when:

- ordinary programs can perform the common tasks in its accepted scope
- obvious high-value gaps and asymmetric operations are closed
- semantics, fallibility and portability are explicit
- the JavaScript path is lean and emits only reachable helpers or assets
- integration coverage protects the public contracts
- canonical docs describe the implemented surface
- the packages and builders progress matrix records the best known next extensions
- remaining additions no longer block common use and move to later work

`@web/canvas` is the deliberate exception to the narrow-completion bias. It may grow further because
visual and animation-heavy programs are useful Moth compiler, runtime and language stress tests.

## First-party dependency policy

First-party Moth packages have zero third-party runtime dependencies.

Allowed implementation inputs are:

- handwritten Moth-owned source
- compiler and runtime primitives owned by this repository
- another accepted first-party package when that dependency is architecturally appropriate
- stable built-in facilities of the selected backend environment
- Web Platform facilities for packages whose Builder contract explicitly requires the web

Examples of acceptable JavaScript facilities include ECMAScript `Math`, `JSON`, `Date`, typed arrays
and language built-ins available in the package's declared runtime environment.

Forbidden inputs include:

- npm or another third-party package-manager dependency
- hidden transitive runtime dependencies
- third-party lockfiles or manifests for first-party package implementations
- copied or vendored third-party libraries
- external bare-module imports that are not explicit Moth-owned runtime modules
- bundled third-party polyfills

Small compatibility code may be handwritten and owned by Moth when the package contract justifies it.
Do not copy an external implementation into the repository to evade this rule.

Users may create their own binding-backed packages. Future dependency packages may implement the same
capability in Moth after the needed language primitives exist. The first-party validation rule must
not prohibit those future user-owned mechanisms.

Phase 0 adds one focused validation owner to `just validate`. It must:

- inspect only first-party package and runtime implementation roots
- reject package-manager manifests and lockfiles within those roots
- reject unapproved bare JavaScript module imports
- use one explicit allowlist for Moth-owned runtime modules where imports are required
- reject known vendored dependency roots
- include positive and negative tests
- avoid a repository-wide substring scan that mistakes documentation or test fixtures for production
  dependencies

## Merge isolation and branch policy

Use one dedicated long-lived worktree for this programme.

At the start of each package and after every major package phase:

1. inspect current `main`
2. inspect the active diagnostics branch while it remains unmerged
3. sync a published stable checkpoint when it changes shared shapes relevant to package work
4. resolve the migration before more package code is added
5. run the complete phase gate after the sync

Do not sync midway through a bounded phase unless the phase is blocked.

While diagnostics and source-data-layout work is active, prefer changes in:

- `src/builder_surface/core_packages/`
- `src/backends/js/package_bindings/`
- `src/projects/html_project/binding_packages/`
- package-local runtime assets
- `packages/` source-backed first-party packages
- focused tests under `tests/cases/`
- package documentation and the package progress matrix

Small additive wiring through existing extension points is allowed.

Avoid changing:

- shared diagnostic payloads, descriptors, rendering or storage
- source and path identity ownership
- the general external-package representation
- HIR call representation
- frontend stage ownership
- build graph construction
- general target-validation architecture

This boundary may be superseded after a stable shared shape lands. Adopt the new shape directly.
Never preserve obsolete types through adapters just to avoid touching the package branch.

Create a separate package branch only when one package becomes unusually invasive. The default is one
programme worktree with independently auditable phase commits.

## Per-package activation workflow

Before package implementation starts:

1. audit its current public API, implementation, tests and documentation
2. list the common missing operations and known later extensions
3. identify portability, fallibility, memory, ABI and target questions
4. settle the current v1 scope with the user
5. create or refresh its living plan under this directory
6. update this tracker and the packages and builders progress matrix with the accepted target
7. implement one bounded phase at a time
8. run the mandatory phase gate
9. compress completed phase details while preserving durable implementation knowledge

A simple package may need a short design checkpoint. `@core/io` requires a substantial scope and
prelude review before major expansion.

## Living package plan structure

Each package plan uses this compact structure:

1. **Role and authority**
2. **Current surface**
3. **Implementation notes**
4. **Design rationale worth preserving**
5. **Current work**
6. **Known gaps and next extensions**
7. **Longer-term candidates**
8. **Previous blockers and rejected approaches**
9. **Validation and integration coverage**
10. **History**

The history records major completed versions or refactors only. It is not a phase-by-phase changelog.

Create a package plan only when that package is activated. Do not add speculative placeholder files.

## Mandatory phase gate

Every code-bearing phase ends with all of the following.

### Correctness and contract audit

- compare implementation with canonical package docs and the active package plan
- verify fallibility, access, alias and return contracts
- verify Core semantics do not inherit accidental JavaScript behaviour
- verify target validation rejects unsupported reachable use before lowering
- verify package availability and prelude policy did not drift
- verify every changed public contract has one clear primary test owner

### Integration coverage threshold

The gate fails unless:

- every new or changed user-visible contract family has a primary integration case
- every newly public function is exercised end to end by the close of the package's current v1 slice
- the package owns at least one rich integration scenario that combines several operations with
  ordinary Moth semantics
- rich scenarios use relevant collections, options or results, templates, control flow, mutation,
  namespace binding or module boundaries rather than calling helpers in isolation
- every fallible surface has a success path and a meaningful failure or recovery path
- portability-sensitive edge cases have explicit regression coverage
- another backend reuses the same integration input when semantic parity becomes available
- unit tests protect only hidden invariants that integration output cannot inspect
- redundant one-function-per-fixture cases are removed when a richer scenario owns the same contract

Use runtime-output assertions for runtime behaviour. Use artifact assertions only when helper
reachability, imports or generated structure are themselves contractual.

### Code-quality audit

Apply the project style guide and code-review guide. Check for:

- duplicated registration or helper logic
- abstractions introduced before a second real use
- package-local files that now own unrelated concepts
- stale compatibility paths
- unnecessary allocation or emitted JavaScript
- helpers emitted when unreachable
- unclear WHAT and WHY comments
- tests coupled to implementation details
- avoidable line count or indirection

Fix findings before the next phase or record a deliberate, bounded deferral.

### Merge-isolation audit

Inspect the phase diff for unnecessary changes to diagnostics, source identity, HIR, Stage 0 and
shared package machinery. Sync stable upstream shape changes at this boundary. Remove compatibility
adapters made obsolete by the sync.

### Validation

Run at minimum:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
```

Also run focused package integration cases and JavaScript artifact checks required by the package
plan.

### Documentation and tracking closeout

- update canonical package docs for accepted behaviour
- update `docs/src/docs/progress/packages-and-builders/@page.moth`
- update the compiler progress matrix only for compiler or backend status
- update this tracker
- retain useful blockers and next extensions in the living package plan
- keep implementation logs in Git history rather than growing the plan

No later phase starts while a gate finding is unresolved or unrecorded.

## Programme tracker

The order below is the default. Adjacent items may move when current `main` makes another package
materially safer to implement. Record the reason in the tracker rather than silently reordering work.

| Order | Work item | Living plan | Current state | High-level v1 target |
|---|---|---|---|---|
| 0 | Package foundations | this plan | Active next | Remove speculative package kinds, enforce terminology and add the first-party dependency guard |
| 1 | `@core/text` | `core-text.md` | TODO: create when activated | Close common Unicode-aware inspection, search and transformation gaps without accepting temporary ABI-shaped APIs |
| 2 | `@core/random` | `core-random.md` | TODO: create when activated | Complete common scalar random generation and specify portable observable rules while allowing unpromised generator identity to differ by backend |
| 3 | `@core/math` | `core-math.md` | TODO: create when activated | Audit the broad existing Float surface, fill common omissions and preserve finite-result boundaries |
| 4 | `@core/time` | `core-time.md` | TODO: create when activated | Complete the common Duration, TimeMark and Timestamp slice, then stop before an unreviewed civil-time or time-zone design |
| 5 | `@web/canvas` | `web-canvas.md` | TODO: create when activated | Expand drawing, state, path, transform, text, image and pixel workflows deeply enough to support substantial visual stress-test programs |
| 5a | `@html` | `html.md` | TODO: create only when needed | Add source-backed wrappers or broadly useful helpers required by canvas and HTML package work, without turning `@html` into a framework |
| 6 | `@core/io` | `core-io.md` | TODO: create when activated | Run a dedicated scope and prelude review, then close only the agreed common gaps |
| 7 | `@core/collections` | `core-collections.md` | TODO: create when activated | Audit common non-sorting gaps and integrate specialised collection work without duplicating its accepted contracts |
| 7a | Collection sorting | [core-collections-sorting.md](./core-collections-sorting.md) | Accepted and queued behind mixed-backend prerequisites | Preserve the accepted stable-by-default `sort` contract and implement it only after its mixed-backend prerequisites |
| 8 | `@core/json` | `core-json.md` | Accepted package, TODO: design when activated | Design and implement a useful JSON v1 without reflection, generic derivation or a representation Moth cannot express correctly |
| 9 | Programme review | this plan | Deferred | Audit useful completeness, choose whether another candidate deserves a package design and leave the tracker coherent |

### Package-specific boundaries

#### `@core/text`

Prioritise everyday Unicode-aware operations. Likely areas for design include trimming, case
conversion, replacement, location/search operations and character-aware inspection.

Collection-producing operations such as split and join must wait when the correct collection
signature cannot cross the binding boundary. Do not replace them with opaque iterators or JS arrays.

#### `@core/random`

Keep bounds and result-domain semantics portable. Decide seeded or reproducible generation only in
the package design checkpoint. Backend-local algorithms remain permitted until sequence identity is
explicitly promised.

#### `@core/math`

Start with an inventory because the package is already broad. Add common omissions, not specialised
numeric subfields. Coordinate with the accepted numeric redesign and avoid freezing legacy numeric
shapes into new APIs.

#### `@core/time`

Keep monotonic time, elapsed durations and UTC timestamps distinct. JavaScript `Date` behaviour is an
implementation source, not a semantic authority. Civil dates, time zones and locale-sensitive
formatting need their own careful design before implementation.

#### `@web/canvas`

A broad package slice is intentional. Prefer coherent workflows over a mechanical Web API mirror.
Exercise mutable opaque handles, error recovery, runtime asset reachability, image and pixel data,
templates and long-running visual programs.

Update `@html` wrappers only where source-owned methods or helper composition clearly improve Moth
usage.

#### `@core/io`

Do not assume its final shape from this plan. Its dedicated design must decide:

- which capabilities belong in the large prelude Core package
- which capabilities should always be visible through bare `io`
- which areas should become focused Core packages
- which areas are Web or another builder's responsibility
- how input, output, event and teardown concepts remain coherent

The current broad future list stays candidate input until that review.

#### `@core/collections`

Do not redesign sorting. The accepted sorting plan already lives in this directory. Retain its
contract and link it from the collection plan.

Review non-sorting gaps against collection, borrow and memory-management authorities. Operations such
as whole-domain clear may carry lifetime and retained-edge meaning, so package convenience must not
outrun those contracts.

#### `@core/json`

The package is accepted as this programme's one new Core package. Its representation is not accepted
yet.

The package design must decide parsing, serialization, value inspection, object and array access,
construction, number handling, ordering and error semantics. It must not assume reflection, automatic
struct conversion or generic derivation. Defer any operation whose final Moth value shape cannot be
represented.

## Candidate capability domains

These lists guide later design review. They are not accepted package names, APIs or roadmap
commitments.

Likely Core candidates:

- structured data formats beyond the accepted JSON work
- portable text and binary encoding after the `Byte` design is usable
- URL and URI parsing and construction
- richer text pattern search, with regular expressions considered only after semantics are chosen
- networking and HTTP after async, task and host-capability design
- filesystem and path operations after cross-target capability ownership is clear

Likely Web Builder candidates:

- DOM querying and mutation
- browser storage
- navigation, location and history
- clipboard and file selection
- browser-specific input and event surfaces
- animation and frame scheduling
- fetch after the async and task model exists

Cryptography is deliberately absent. Do not add it as a Core candidate, example or implied future
package.

A candidate becomes a package only after a focused design checkpoint establishes:

- why the capability belongs in first-party Moth
- its origin and builder availability
- its exact scope
- portable or host-specific semantics
- the required Moth value shapes
- its fallibility and capability boundaries
- whether existing packages already own the need

Only then add a package row to the packages and builders progress matrix.

## Roadmap and progress ownership

The main roadmap:

- links this umbrella programme once
- records that it is active in parallel with diagnostics
- does not list each child package plan
- retains the later package dependency and manager foundations work as a separate item

This umbrella:

- owns implementation order
- links living package plans
- records blockers and current package activation state
- keeps candidate capability domains explicitly non-authoritative

The packages and builders progress matrix:

- records implemented package and project-builder status
- records the most useful accepted next extensions while context is fresh
- does not design package APIs
- does not track backend implementation details
- adds a package row only after its scope is deliberately accepted

The compiler progress matrix:

- records language, compiler, memory and backend status
- retains target support even when a package exposes the feature
- does not duplicate package API inventories

Canonical package docs:

- own accepted public types, functions, semantics, fallibility and limitations
- must be updated in the same phase that changes an accepted contract

Living package plans:

- preserve implementation rationale, quirks, prior blockers, rejected approaches and future work
- never override canonical documentation

## Programme phases

### Bootstrap - branch and tracking setup

Delivered before Phase 0:

- split package and project-builder progress into its own matrix
- link package reference pages to that matrix
- create this umbrella plan and living-plan rules
- move the accepted collection sorting plan into the package programme directory
- sync the package branch with the published diagnostics data-layout checkpoint

### Phase 0 - package foundations hardening

In progress:

- removed `PackageOrigin::Standard` and Standard-tier documentation
- kept `PackageOrigin::Dependency` for later package-system work
- added `just first-party-deps` to `just validate` with scoped first-party roots
- the guard now reuses the HTML JS module scanner, inspects inventoried Core helper and inline JS,
  and allows only exact `RuntimeModuleRegistry` specifiers
- no cryptography Core package examples were present in canonical docs

Do not activate `@core/text` until this phase stays green.

### Phase 1 - activate the living package workflow

- create `core-text.md` from the required living-plan structure
- complete the `@core/text` API and implementation audit
- settle its useful v1 scope with the user
- update the tracker and package progress row
- verify that package-plan links and lifecycle wording remain accurate
- commit the accepted text plan before implementation starts

Mandatory closeout: documentation, design-boundary, merge-isolation and validation audit. No text
implementation belongs in this phase.

### Phase 2 - `@core/text` current v1 slice

Follow `core-text.md`. Implement only the accepted, representable surface.

Mandatory closeout: full phase gate plus the text rich integration scenario.

### Phase 3 - `@core/random` current v1 slice

Create and accept `core-random.md`, then implement its bounded v1 surface.

Mandatory closeout: full phase gate plus statistical tests that remain deterministic and contractual.
Do not use flaky distribution thresholds as the only correctness evidence.

### Phase 4 - `@core/math` current v1 slice

Create and accept `core-math.md`, then fill only the high-value gaps found by its audit.

Mandatory closeout: full phase gate plus finite-result, domain edge and integration coverage.

### Phase 5 - `@core/time` current v1 slice

Create and accept `core-time.md`, then complete the bounded duration and timestamp surface.

Mandatory closeout: full phase gate plus deterministic parsing, conversion and monotonic-time
contract coverage. Keep wall-clock tests independent of the machine's current date and time.

### Phase 6 - `@web/canvas` expansion and optional `@html` wrappers

Create and accept `web-canvas.md`. Create `html.md` only when the wrapper work is substantial enough
to need its own implementation memory.

Mandatory closeout: full phase gate plus at least one substantial visual-program integration case,
runtime asset reachability checks and failure coverage for unavailable handles or invalid operations.

### Phase 7 - `@core/io` scope and current v1 slice

Create `core-io.md` and complete the dedicated prelude and package-scope review before implementation.

The design checkpoint may split implementation into later phases. Do not infer the answer from the
candidate list in current docs.

Mandatory closeout for every accepted IO implementation phase: full phase gate plus rich console,
input, lifecycle and recovery coverage appropriate to that phase.

### Phase 8 - `@core/collections` follow-up and sorting handoff

Create `core-collections.md` and audit non-sorting gaps. Follow
`core-collections-sorting.md` only when its mixed-backend prerequisites are ready.

Do not use this phase to bypass those prerequisites or reopen the accepted sorting contract.

Mandatory closeout: full phase gate plus fixed and growable collection scenarios that exercise
mutation, aliasing, fallibility and retained identity.

### Phase 9 - design and implement `@core/json`

Create and accept `core-json.md` before registering a package symbol.

If the correct design is not representable, retain the accepted package as blocked and record the
missing capability. Do not ship a temporary public representation.

Mandatory closeout for implementation: full phase gate plus nested data, Unicode strings, numeric
edges, invalid input, recovery, round-trip and helper-reachability coverage.

### Phase 10 - programme review and next candidate decision

- audit every package against useful v1 completeness
- remove stale tracker detail
- verify living plans preserve only useful implementation memory
- verify package docs and progress rows agree with implementation
- audit the no-third-party-dependencies guard against all first-party implementations
- review candidate domains without automatically accepting another package
- either select one candidate for a new design checkpoint or leave the programme parked with a clear
  next action

Mandatory closeout: programme-wide correctness, style, documentation, integration-suite and merge
audit.

## Stop conditions

Stop the current package phase and request review when:

- the public API would be shaped by a temporary JavaScript or ABI limitation
- implementation needs a third-party dependency
- a Core contract would expose backend-specific behaviour that was not deliberately accepted
- work requires generic binding-backed functions or a broad ABI redesign
- work crosses into unstable diagnostic payload or source-layout representation
- a compatibility wrapper would preserve a shared type already replaced upstream
- source clauses begin acquiring packages
- package aliases, resolver fallback or transitive visibility enter the programme
- a new package origin or importance tier appears
- `@core/io` scope or prelude visibility expands without its dedicated review
- collection sorting behaviour diverges from its accepted plan
- cryptography appears as a first-party Core proposal or example
- one package phase crosses more than two unlisted subsystem boundaries

## Completion condition

This programme is ready to park when:

- package foundations use only accepted origin and backing concepts
- the first-party dependency guard is enforced by normal validation
- each activated package has a living implementation companion
- existing packages have coherent useful v1 surfaces for the implemented JavaScript target
- `@core/json` is either correctly implemented or explicitly blocked on a named language capability
- package docs and both progress matrices agree with reality
- every completed package has rich integration coverage
- all remaining work is recorded as a concrete package extension or an explicitly unaccepted
  capability candidate
