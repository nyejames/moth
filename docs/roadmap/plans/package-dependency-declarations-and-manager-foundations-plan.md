# Package dependency declarations and package-manager foundations plan

## Purpose

Design and implement the project-level declaration and resolver boundary that makes external Moth packages available to Stage 0.

This plan is separate from file-local dependency clauses:

```text
ProjectDependencyDeclaration
-> declares a direct external project dependency and optional project-local root alias

FileDependencyClause
-> binds names from an already registered root into one source file
```

The file-local grammar is owned by:

- delivered dependency clauses and path syntax, whose accepted contract is canonical in `docs/src/docs/packages/dependency-clauses.mtf` and `dependency-paths.mtf`

Source clauses never add an undeclared external package implicitly.

Remote registries, fetching, version solving, publishing and package-manager policy remain deferred.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md
STATUS: queued-design - implementation blocked until declaration and resolver design is accepted
CURRENT_SLICE: Design Phase 0 - audit project dependency availability and package boundary inputs
IMPLEMENTATION_SCOPE: project dependency declarations, aliases, resolver/catalog handoff and separate dependency package graphs
SUPERSEDED_DECISION: the former `import @package` config preamble is not accepted and must not be implemented
```

Keep this block concise. Git history is the implementation record. Establish the baseline in local working notes when this plan activates.

## Roadmap position

This plan remains design-gated. Its implementation starts only after its configuration, resolver and package-manager boundaries are reviewed.

Do not infer activation from the presence of this file.

## Hard prerequisites

- canonical project/package graph and immutable artefact architecture
- dependency-clause and path-syntax migration
- delivered project configuration and typed build-input contracts from the activation tree, with immutable `@project`, recursive schemas and package-boundary isolation
- unified numeric type identity, early NumericProfile selection and profile-compatible compiler/package inputs
- stable package, module, public-interface and capability fingerprints
- completed HTML mixed-target backend boundary where required by package compatibility policy

Consume the configuration spelling and owners delivered by earlier serial work. Do not restore the pre-directive grouped bootstrap or a second input parser because an older implementation used `#Config`.

## Required authorities

- `docs/compiler-design-overview.md`
- delivered build configuration values and project-boundary isolation
- `docs/build-system-design.md`
- `docs/src/developer-docs/language/overview.mtf`
- canonical package, project-configuration and numeric references
- progress matrix and roadmap
- style, testing and validation guides

## Superseded syntax decision

Earlier revisions proposed:

```moth
import @acme/ui
import @community/markdown as md
```

inside `config.moth`.

That proposal is superseded.

Reasons:

- source `import` is removed by the dependency-clause migration
- `import` becomes an ordinary identifier
- project dependency declaration and source-file binding have different ownership
- parser implementation must not choose project manifest policy

This plan does not yet select replacement syntax.

The accepted design review must choose one of:

- a restricted compiler-owned `config.moth` declaration surface
- a separate project manifest
- package-manager-maintained metadata consumed before config folding
- a programmatic resolver/catalog input for the first implementation

Do not resurrect the removed source `import` grammar under a config-only exception without explicit user review.

## Locked ownership decisions

A build-input contract declares a typed build value. It does not declare, acquire, alias or conditionally enable a package dependency.

### Project dependency declarations

A project dependency declaration:

- names one direct external package
- may define one project-local package-root alias
- participates in bootstrap before project source graph construction
- does not import package symbols into the config program
- does not expose transitive dependencies to project source
- does not use source-file visibility or re-export rules
- is deterministic and authored-order-preserving for diagnostics

### File-local dependency clauses

A file-local clause:

- references only roots already registered in the active Stage 0 namespace
- binds a namespace or selected declarations into one `.moth` file
- does not add or resolve a project dependency
- does not make transitive dependencies visible
- preserves canonical package and declaration identities beneath local aliases

The final source grammar uses one clause for one registered facade surface:

```moth
@acme/ui Button, theme
@community/markdown as md
```

Direct selections are flat binding names inside the resolved surface. They do not acquire a
package, select another provider or change package identity.

### Package aliases

- canonical package identity and project-local root alias are separate facts
- an alias replaces the canonical source spelling inside that project when policy requires it
- aliases affect namespace binding and diagnostics, not package artefact identity
- aliases never alter dependency package output identity
- alias collisions across project modules, support packages, Core, Builder and dependencies are diagnosed before source compilation

### Package boundaries

Each dependency compiles as a separate package boundary with its own:

- config and build inputs
- private `@project`
- source index
- module graph
- generated sidecar worklist
- public package facade
- capability and fingerprint facts

A dependency never sees the consuming project's `@project` or unqualified build inputs.

### Numeric profile and ABI compatibility

The directly linked Moth graph shares one NumericProfile, selected before input
materialisation and config compilation. Separate package input namespaces do not
permit separate Int widths or Float precisions. Source dependencies compile for
the selected profile. A precompiled dependency must match that profile or be
rebuilt/rejected, never adapted silently at an import. This matters even when
its exported types are explicit-width because private arithmetic and folded
constants may use Int/Float.

Preserve the distinct canonical identities of Int/Float, fixed I*/U*/F* types,
Byte and Number scales through facade projection, generated requests and reuse.
Include the profile in semantic artefact compatibility and the existing ABI,
layout and physical-variant inputs. Error.code uses the delivered U32 contract,
not a package-local signed-code adapter. Preserve source aliases independently
of all these facts.

Foreign WIT/components retain explicit foreign widths and separate value
conversion contracts. They do not inherit Moth's profile or replace the semantic
interface of a Moth-source dependency. F16 and Byte need explicit foreign mapping
contracts. Package compatibility does not authorise a WIT loader, new numeric
conversions or a Wasm Number runtime in this plan.

## Accepted preliminary rules

- only direct dependencies are visible to project source
- transitive dependencies remain private to their package graphs
- the same canonical package may be declared once only
- Core, Builder and project-local support packages are not declared through the external dependency surface
- `@project`, project modules, relative paths and parent traversal are invalid package declarations
- source dependencies never trigger implicit package acquisition
- Stage 0 performs no undeclared filesystem probing
- dependency compilation order is deterministic
- package artefacts retain canonical identity beneath consumer-local aliases
- source-backed and precompiled package inputs use one resolver boundary
- package acquisition remains outside compiler semantics

## Design questions that must be finalised

Before implementation, decide and document:

1. canonical package-name grammar and its owner
2. the project declaration storage surface
3. whether aliases are optional or mandatory for names outside the local grammar
4. how canonical names map to resolved source or precompiled artefacts
5. the exact `DependencyCatalog` or resolver input supplied to bootstrap
6. whether the first implementation supports only programmatic resolved packages or one local development source
7. version-constraint ownership and syntax location
8. lockfile identity, reproducibility and offline policy
9. local path, Git and registry override policy
10. alias collisions across package, child-module, support and synthetic namespaces
11. capability compatibility across builders and targets, consuming the fixed numeric profile contract rather than redefining it
12. security, provenance and dependency-count policy
13. package-manager command boundaries
14. persistent artefact and package-cache compatibility
15. how future package versions affect stable package output prefixes

Do not invent declaration syntax while implementing parser or config slices.

## Allowed pre-design groundwork

Before design acceptance, only read-only audits and architecture-neutral cleanup are allowed:

- remove assumptions that every source package is Core, Builder or ProjectLocal
- keep `PackageOrigin::Dependency` orthogonal to source or binding backing
- ensure package boundary IDs never leak into project semantic identity
- make package graph inputs explicit and deterministic
- preserve immutable facade, numeric-profile and capability fingerprint contracts
- keep package output-prefix encoding injective over full stable package identity
- add no user syntax, resolver fallback, filesystem convention or placeholder registry

## Target data boundary

Conceptual declaration records:

```rust
pub struct ProjectDependencyDeclaration {
    pub canonical_name: CanonicalPackageName,
    pub local_root: PackageImportRoot,
    pub location: SourceLocation,
}

pub struct ResolvedDependencyCatalog {
    pub packages: Vec<ResolvedDependencyPackage>,
}
```

Exact names may change.

Required invariants:

- declarations retain authored order for diagnostics
- duplicate canonical packages fail before resolution
- duplicate local roots fail before source compilation
- resolution and compilation use deterministic canonical order
- local aliases are project namespace facts
- package artefacts retain canonical identity
- transient maps accelerate lookup, final records remain contiguous
- resolver results contain no fallback search policy
- unknown source package roots diagnose rather than probe the filesystem

## Resolver contract

The resolver/catalog boundary owns:

- declaration to canonical package resolution
- source versus precompiled package input
- compatibility and capability facts, including the already-selected numeric profile
- package dependency metadata
- structured missing, duplicate and incompatible package failures
- future version and lockfile integration

It does not own:

- source-file binding
- config constant visibility
- project source dependency discovery
- package acquisition policy
- backend lowering
- output placement

Compiler and build orchestration consume resolved records only.

## Non-goals before an explicit design expansion

- remote registry implementation
- network fetching
- version solver
- lockfile implementation
- publishing
- arbitrary URL syntax
- transitive dependency visibility
- implicit dependency discovery from source clauses
- config-visible dependency symbols
- compatibility for `package_folders` or `/lib`
- restoring source `import`
- resolver fallback filesystem search

## Design phases

### Design Phase 0 - repository and authority audit

- inventory config bootstrap, package identity, package registries, separate graphs, facades, fingerprints and capabilities
- trace one hypothetical dependency from declaration through resolver, graph compilation, provider binding, generated sidecars and linking
- trace alias ownership separately from canonical package identity
- inspect package output-prefix requirements from the resource authority
- verify where the selected numeric profile reaches source compilation and precompiled compatibility checks
- record every unresolved ownership question
- produce no implementation code

### Design Phase 1 - freeze declaration and alias semantics

- choose the project declaration storage surface
- specify canonical package-name grammar
- specify alias replacement and duplicate policy
- specify namespace collision rules
- specify direct-only visibility and transitive privacy
- specify config-folding isolation
- specify migration and diagnostic ownership
- update language and build authorities for review

Mandatory review: no parser or config implementation before acceptance.

### Design Phase 2 - freeze resolver and package-manager handoff

- define resolver/catalog input and result classes
- define source and precompiled package descriptors
- define compatibility and capability facts using the delivered numeric profile and canonical scalar identities
- define the first supported resolution source
- define future lockfile and package-manager ownership without implementing it
- decide local development dependency policy
- define deterministic dependency graph order
- define canonical package output identity inputs

Mandatory review: implementation remains blocked until Phases 1 and 2 are accepted.

## Implementation phases after design acceptance

### Phase 3 - extract project dependency declarations

- parse only the accepted project-level declaration surface
- retain authored order and source locations
- keep declarations outside config constant visibility
- reject source-file dependency-clause shapes in config unless the accepted design explicitly chooses a related but distinct config grammar
- preserve `import` as an ordinary identifier after the source grammar cutover
- diagnose duplicates and invalid package roots before resolver calls

### Phase 4 - build the project dependency table

- validate canonical names and local aliases
- validate collisions against project, support, Core, Builder and synthetic roots
- produce deterministic declaration records
- register only direct local roots
- keep transitive roots private

### Phase 5 - resolve through the catalog

- resolve each declaration exactly once
- diagnose missing, duplicate, incompatible and unsupported packages before project source compilation
- reject precompiled numeric-profile mismatches or select a matching source rebuild without implicit numeric adapters
- perform no undeclared filesystem probing
- keep package acquisition outside compiler semantics
- retain resolved package identity beneath local aliases

Review gate: declaration, resolution and compilation must have separate owners.

### Phase 6 - compile separate dependency package graphs

- compile dependencies in deterministic dependency order
- give each package its own config, inputs, source index, graph, generated worklist and private `@project`
- pass the same numeric profile to every directly linked Moth compilation service before numeric inputs or constants materialise
- publish only immutable facade artefacts to consumers
- preserve capability, interface and compatibility fingerprints
- reject public or reachable executable dependence on private package project context
- preserve package resource origins and output-prefix identity when resource support is complete

### Phase 7 - register aliases for project source

- register each direct dependency root in Stage 0
- bind file-local dependency clauses through completed facade interfaces
- preserve canonical origins beneath local aliases
- reject canonical spelling when policy says an alias replaces it
- keep transitive dependency roots unavailable
- do not change file-local clause grammar in this phase

### Phase 8 - tooling, tests and documentation

- add fixture catalogs without production fallback discovery
- add diagnostics for missing declarations, collisions and transitive access
- update scaffolds only when a package source is available through the resolver
- update project, package and configuration docs
- update roadmap and progress only for implemented behavior

## Required end-to-end contracts

- one direct dependency declaration resolves once
- local alias binds the facade while preserving canonical package identity
- duplicate canonical declaration fails
- duplicate local root fails
- undeclared source clause does not acquire a package
- transitive dependency remains private
- dependency package cannot see consumer `@project`
- dependency package has its own generated sidecars and graph identity
- reversed declaration order does not change canonical compilation order
- no resolver fallback path is probed
- source and precompiled descriptors share one consumer boundary
- package output identity does not depend on consumer alias
- source packages share all four supported numeric-profile combinations with their consumer
- an incompatible precompiled profile is rejected even when only private implementation arithmetic uses Int/Float
- exported fixed widths, Byte, Number scale and U32 Error.code preserve canonical identity beneath aliases
- foreign WIT projections remain distinct from Moth-native package interfaces

## Stop conditions

Stop and request review when:

- implementation requires inventing declaration syntax
- file-local clauses begin acquiring packages
- transitive dependencies become project-visible
- alias identity enters package artefacts
- resolver code starts probing fallback filesystem locations
- package acquisition enters compiler semantics
- one package boundary needs the consumer's project globals
- compatibility policy requires backend-specific parser behavior or implicit numeric-profile adapters
- an implementation phase crosses more than two unlisted stage boundaries

## Validation

Every accepted code-bearing phase requires:

```bash
cargo fmt --all
just validate
```

Run architecture review whenever declaration, resolver, package graph, alias or facade ownership changes.

## Deferred follow-ups

- remote registry and fetching
- version solver and lockfile
- publishing
- package-manager commands
- security and package-quality policy
- local, Git and registry overrides
- persistent package caches
- package audit and dependency-count policy
