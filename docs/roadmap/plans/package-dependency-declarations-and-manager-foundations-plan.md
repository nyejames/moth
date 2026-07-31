# Package dependency declarations and package-manager foundations plan

## Purpose

Design and then implement the minimal project dependency declaration and resolver boundary needed for external Moth packages, while leaving remote registry, fetching, version solving and package-manager policy to a later dedicated system.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md
STATUS: queued-design - implementation blocked until the design review is accepted
CURRENT_SLICE: Design Phase 0 - inventory package identity, config bootstrap and dependency graph boundaries
REVIEW_BASELINE: 47dbf3fd1dfa3e8df3d02cef05001de695ea80ee
LAST_GOOD_COMMIT: none until the design checkpoint and first implementation slice are accepted
BRANCH: main
IMPLEMENTATION_SCOPE: config dependency declarations, aliases, resolver/catalog handoff, separate dependency package graphs
```

Keep this block concise. Git history is the implementation record.

## Roadmap position

This plan runs after the HTML mixed JavaScript/Wasm backend plan.

Do not begin implementation merely because this file exists. Design Phases 0 through 2 must be reviewed and accepted first.

## Hard prerequisites

- canonical project/package graph and immutable artefact architecture
- grouped project config
- imported build values and project-boundary isolation
- stable package, module, public-interface and capability fingerprints
- completed HTML mixed-target backend boundary

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- package sections of the progress matrix and roadmap
- style, testing and validation guides

## Accepted preliminary decisions

The intended config surface is a restricted dependency preamble:

```moth
import @acme/ui
import @community/markdown as md

project #= |
    name = "my_app",
    entry_root = "src",
|

html #= ||
```

Accepted rules:

- Config dependency declarations use ordinary root import spelling and optional `as` alias syntax.
- They declare project dependencies. They do not import package symbols into the config program.
- Declarations form one contiguous preamble before config constants.
- Only package roots are valid. Grouped symbols, re-exports and child symbol paths are invalid.
- Canonical package identity and the consumer's local alias are separate facts.
- `import @acme/ui as widgets` makes `@widgets` the only local project spelling for that dependency.
- The same canonical package may be declared once only.
- Only direct dependencies are visible to project source. Transitive dependencies remain private to their package graphs.
- Core, Standard, Builder and project-local support packages are not declared through this preamble.
- `@project`, project modules, relative paths and parent traversal are invalid.
- Each dependency compiles as a separate package boundary with its own config, source index, private `@project`, artefacts and external facade.
- A dependency never sees the consuming project's `@project` or unqualified build inputs.

## Design questions that must be finalised

Before implementation, decide and document:

1. canonical package-name grammar and registry ownership
2. how a canonical name maps to a resolved package source or precompiled artefact
3. the `DependencyCatalog`/resolver input supplied to bootstrap
4. whether the first implementation supports only programmatically supplied resolved packages or one local development source
5. version-constraint ownership and whether version syntax belongs in config, lock metadata or a future command surface
6. lockfile identity, reproducibility and offline behaviour
7. local path, Git and registry override policy
8. alias collisions across package, child-module, support-package and source namespaces
9. capability compatibility across builders and targets
10. security, provenance, dependency-count and future package-quality policy
11. package-manager command boundaries such as add, remove, update and audit
12. persistent artefact and package-cache compatibility

Do not invent syntax while implementing a parser slice. Update the authority documents and this plan after the design review.

## Allowed pre-design groundwork

Before the design is final, only read-only audits and architecture-neutral cleanup are allowed:

- remove assumptions that every source package is Core, Builder or ProjectLocal
- keep `PackageOrigin::Dependency` and source/binding backing orthogonal
- ensure separate package boundary IDs never leak into the project boundary
- make package graph inputs explicit and deterministic
- preserve immutable facade and capability fingerprint contracts
- add no user syntax, resolver fallback, filesystem convention or placeholder registry

## Target data boundary

After design acceptance, use a small data-oriented handoff equivalent to:

```rust
pub struct PackageDependencyDeclaration {
    pub canonical_name: CanonicalPackageName,
    pub local_root: PackageImportRoot,
    pub location: SourceLocation,
}

pub struct ResolvedDependencyCatalog {
    pub packages: Vec<ResolvedDependencyPackage>,
}
```

Required invariants:

- declarations stay in authored order for diagnostics
- resolution and compilation use deterministic canonical order
- aliases are consumer-local namespace facts
- package artefacts retain canonical identity
- transient maps accelerate lookup, final data uses contiguous records
- Stage 0 never probes fallback filesystem locations for an undeclared package

## Non-goals

Unless Design Phase 2 explicitly expands scope:

- no remote registry implementation
- no network fetching
- no version solver
- no lockfile implementation
- no package publishing
- no arbitrary path or URL syntax in `config.moth`
- no transitive dependency visibility
- no implicit dependency discovery from source imports
- no config-visible dependency symbols
- no compatibility path for `package_folders` or `/lib`

## Design phases

### Design Phase 0: Repository and authority audit

- Inventory config import shells, package identity, source/binding registries, separate package graphs, facades, fingerprints and builder capabilities.
- Trace one hypothetical dependency from declaration through resolver input, graph compilation, interface binding, generated sidecars and linking.
- Record every unresolved ownership question.
- Produce no implementation code.

### Design Phase 1: Freeze declaration and alias semantics

- Specify package-name grammar.
- Specify the exact preamble grammar and placement.
- Specify alias replacement, duplicate declarations and namespace collision diagnostics.
- Specify direct-only visibility and transitive privacy.
- Specify config folding isolation.
- Update language and build-system authorities for review.

### Design Phase 2: Freeze resolver and package-manager handoff

- Define the resolver/catalog input and error classes.
- Define source/precompiled package descriptors and compatibility facts.
- Define what the first implementation can resolve without a package manager.
- Define future lockfile and package-manager ownership without implementing it.
- Decide local development dependency policy.

Mandatory review gate: implementation remains blocked until Design Phases 1 and 2 are accepted.

## Implementation phases after design acceptance

### Phase 3: Extract dependency declarations from config

- Permit only the accepted dependency preamble.
- Retain import shells once and classify them without provider binding.
- Exclude dependency declarations from config AST visibility and constant ordering.
- Keep every other config import invalid.

### Phase 4: Build the declared dependency table

- Validate canonical names, aliases, duplicates and namespace collisions.
- Produce deterministic declaration records.
- Register only direct local roots into the project namespace.
- Do not resolve transitive names into project visibility.

### Phase 5: Resolve through the catalog boundary

- Resolve every declaration exactly once through the accepted catalog interface.
- Diagnose missing, duplicate, incompatible or unsupported packages before project source compilation.
- Perform no undeclared filesystem probing or fallback search.
- Keep package acquisition outside compiler semantics.

Review gate: verify declaration, resolution and compilation are three separate owners.

### Phase 6: Compile separate dependency package graphs

- Compile dependencies in deterministic dependency order.
- Give each package its own config, inputs, source index, graph, generated worklist and private `@project`.
- Publish only immutable facade artefacts to consumers.
- Preserve capability and public-interface fingerprints.
- Reject public or reachable executable dependence on private dependency project context.

### Phase 7: Bind aliases into project source

- Register each local dependency root in the Stage 0 namespace.
- Bind source imports through completed facade interfaces.
- Preserve canonical origin identities beneath local aliases.
- Reject canonical spelling when an alias replaced it.

### Phase 8: Tooling, tests and documentation

- Add fixture catalogs for integration tests without introducing production fallback discovery.
- Add diagnostics for missing declarations, collisions and transitive access.
- Update scaffolds only when a package source is available through the accepted resolver boundary.
- Update roadmap, progress and package documentation.

## Stop conditions

Pause when:

- implementation needs an unresolved version/path/lockfile decision
- a package is found through fallback scanning
- aliases change canonical package identity
- transitive dependencies become source-visible
- config folding needs provider symbols
- project inputs leak into a dependency boundary
- a second package graph or registry representation appears
- a temporary parser surface would constrain the future package manager

## Validation

Every code-bearing phase after design acceptance runs:

```bash
cargo fmt
just validate
```

## Final audit

Verify:

- config declarations are metadata-only and preamble-only
- aliases are project-wide local roots, not alternate canonical identities
- only direct dependencies are visible
- dependencies compile as separate immutable package graphs
- no config symbol import, fallback filesystem search or input leakage exists
- future package-manager ownership remains explicit and unimplemented where deferred
