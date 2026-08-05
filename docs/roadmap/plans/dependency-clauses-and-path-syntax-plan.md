# Dependency clauses and path syntax implementation plan

## Purpose

Replace the current expanded path-token and `import` grammar with one retained path syntax owner and one direct top-level dependency-clause language.

The target is:

- one file-owned path syntax table
- one `DependencyShellId` per authored top-level clause
- extensionless source and package dependencies
- explicit-extension registered provider dependencies
- static grouped and filtered namespace bindings
- explicit public re-export selections
- no source `import` keyword
- no raw token rescan after preparation
- no graph, package or provider consumer coupled to authored syntax

This plan does not implement builtin `Path`, resource identity, TIR resource anchors, asset emission or resource invalidation.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/dependency-clauses-and-path-syntax-plan.md
PLAN_ADOPTION_BASELINE: bfaacd54227811f9e2b279d5a24e3df84dc381c2
STATUS: queued - blocked until canonical module Phase 5 Gate D
CURRENT_SLICE: Phase 0A - plan adoption and authority reconciliation
PREDECESSOR: canonical-module-compilation-and-scoped-packages-plan.md Gate D
FOLLOW_UPS:
1. tir-corrections-and-simplification-plan.md
2. path-values-and-resource-linking-plan.md
BLOCKERS:
- canonical module Phase 5 has not reached its mandatory handoff
- project-level external package declaration syntax remains owned by the design-gated package dependency plan
NEXT_RESUME_ACTION: after canonical Gate D, refresh repository owners and execute Phase 0B
```

Keep this block concise. Git history is the implementation record.

## Required authorities

- `AGENTS.md`
- `docs/language-overview.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- canonical language references under `docs/src/docs/`
- style, testing and validation guides
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md`
- `docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md`

## Locked language contract

### Dependency availability and file binding are separate

A file dependency clause binds names from a root that Stage 0 already knows.

It never:

- downloads or discovers an undeclared external package
- adds a project dependency
- changes canonical package identity
- exposes transitive package dependencies
- imports provider implementation files as values

Project-level external package availability is owned by a separate declaration and resolver boundary.

```text
ProjectDependencyDeclaration
-> registers a direct external package root and optional project-local alias

FileDependencyClause
-> binds names from an already registered root into one source file
```

Do not invent project config syntax while implementing this plan.

### One `@` dependency model

At `.moth` top level:

| Form | Meaning |
|---|---|
| `@core/math` | bind the source/package namespace as `math` |
| `@core/math as maths` | bind the namespace as `maths` |
| `@core/math { sin, cos }` | bind selected names directly |
| `@core/math as maths { sin, cos }` | bind one filtered namespace as `maths` |
| `@drawing.js { draw }` | bind selected names from a registered provider |
| `@drawing.js as drawing` | bind the complete provider namespace |
| `@drawing.js as drawing { draw }` | bind one filtered provider namespace |

Rules:

- extensionless paths select source modules, source packages or binding package roots
- recognized source extensions such as `.moth`, `.mtf` and `.md` are omitted
- explicit-extension top-level paths select a registered semantic provider
- a bare explicit-extension clause requires `as` or a selection group
- an explicit-extension clause with no provider gets a targeted diagnostic
- a recognized source extension gets an extensionless-source diagnostic
- direct symbol paths such as `@core/math/sin` remain invalid
- selected symbols come through a selection group
- `@./...`, `@../...`, `@/...`, `@@name` and parent components are invalid
- dependency resolution starts from the owning module root
- dependency bindings are file-local and visible throughout the file independent of clause position
- namespace records are static visibility facts, not values
- a namespace has one provider origin
- consumer-created mixed-origin namespace aggregation is outside scope

### Selection grammar

Conceptual grammar:

```text
source_dependency_clause
    = extensionless_path
    | extensionless_path "as" identifier
    | extensionless_path selection_group
    | extensionless_path "as" identifier selection_group

provider_dependency_clause
    = explicit_extension_path selection_group
    | explicit_extension_path "as" identifier
    | explicit_extension_path "as" identifier selection_group

selection_group
    = "{" selection_entry ("," selection_entry)* [","] "}"

selection_entry
    = path_component ["as" identifier] [selection_group]
```

Rules:

- empty groups are invalid
- a leading `@` is invalid inside a group
- nested groups select actual child namespaces from the same provider interface
- any selected entry may use `as`
- duplicate selected paths and duplicate local names are diagnosed
- a group without clause-level `as` binds selected leaves or child namespaces directly
- clause-level `as` plus a group creates one filtered namespace
- an invalid inferred namespace stem requires `as`
- receiver methods remain attached to receiver types and cannot be selected separately

### Public re-exports

Inside a module root's `export:` block:

- only explicit non-empty selection groups are valid dependency re-exports
- whole namespaces cannot be re-exported implicitly
- clause-level namespace `as` is invalid
- leaf aliases define public API names
- nested groups may qualify leaf declarations
- non-leaf namespace aliases are invalid because namespace exports remain deferred
- re-exports preserve declaration origins
- re-export syntax does not create namespace values

### Source contexts

- dependency clauses are valid only at `.moth` file top level and inside a root `export:` block
- `config.moth` rejects file-local dependency clauses before path resolution
- `.mtf` files remain declarationless and cannot declare dependencies
- plain `.md` remains raw Markdown
- `#Import` build-input syntax is unrelated and unchanged
- backend runtime imports, WIT imports and `RequiredRuntimeImport` retain their domain terminology

### Removed `import` keyword

The cutover is one-way:

```moth
-- removed
import @core/math { sin }

-- current after cutover
@core/math { sin }
```

Rules:

- remove `TokenKind::Import`
- remove `import` from keyword classification
- lowercase `import` becomes an ordinary identifier
- `import = 1` parses as an ordinary declaration when otherwise valid
- old `import @...` shape gets one dedicated migration diagnostic
- the diagnostic spans the old keyword and clause and renders the replacement
- use a new stable diagnostic code
- no compatibility parser, warning-only acceptance or feature flag

### Explicit-extension expression paths during this plan

This plan does not define the final builtin `Path` value.

Outside top-level dependency-clause position, explicit-extension path expressions retain the existing compile-time path behavior until the resource plan replaces that lane atomically.

Do not add new asset semantics, public resource identity or another path value representation here.

## Data model

### One file-owned path syntax table

Replace expanded path token payloads with one dense file-local table.

Conceptual shape:

```rust
pub struct PathSyntaxId(u32);

pub struct PathSyntaxTable {
    paths: Vec<PathSyntax>,
    selections: Vec<PathSelectionNode>,
}

pub struct PathSyntax {
    root: InternedPath,
    alias: Option<StringId>,
    selection_range: Option<PathSelectionRange>,
    location: SourceLocation,
}

pub struct PathSelectionNode {
    component: StringId,
    alias: Option<StringId>,
    child_range: Option<PathSelectionRange>,
    location: SourceLocation,
}
```

Exact names may change.

Invariants:

- `TokenKind::Path(PathSyntaxId)` carries one dense handle
- one authored path tree exists once
- group shape and alias locations are preserved
- grouped syntax is not expanded into one complete path allocation per leaf
- `FileTokens` or its accepted successor owns the table
- string remapping visits the table once
- source identity rebinding updates table locations once
- frozen generic syntax retains only referenced path rows
- no second stable path-token vocabulary exists
- no later phase reparses source or reconstructs group shape

### One shell per authored clause

Use typed phases:

```text
ScannedDependencyClause
-> RetainedDependencyClause { DependencyShellId }
-> BoundDependencyClause
```

`DependencyShellId` contains the real `FileId` and clause ordinal.

One authored clause gets one shell ID regardless of selected leaf count. Selected rows use clause-local indexes.

Stage 0 and provider binding consume retained typed facts. They do not inspect tokens or authored keywords.

### Coherent terminology

Migrate source-language concepts coherently:

- `ImportShellId` -> `DependencyShellId`
- `FileImport` -> `FileDependencyClause`
- import clause parser -> dependency clause parser
- structural provider import -> structural provider dependency
- source import access -> source dependency access
- import visibility environment -> binding or visibility environment where accurate

Do not rename:

- source `#Import`
- JavaScript runtime imports
- WIT imports
- external runtime-import metadata

## Data-oriented and performance requirements

- one path tree per authored path
- one shell per authored clause
- dense file-local IDs
- contiguous path and selection rows
- construction maps dropped after binding unless repeated queries justify retention
- no full path allocation per selected leaf
- no token rescan after preparation
- no filesystem probing after Stage 0 namespace resolution
- no path-string or suffix join for provider identity
- no cloned provider semantic payload per alias
- no stage trait or dynamic-dispatch hierarchy

Add counters for:

- path syntax rows
- selection rows
- dependency clauses
- retained shells
- bound source/package clauses
- bound provider clauses
- token rescans, which must be zero in directory compilation

## Work protocol

Each code-bearing phase ends with:

```bash
cargo fmt --all
just validate
```

Also run:

```bash
cargo run --quiet -- tests --audit
```

when fixture metadata or canonical integration cases change.

Stop when:

- another durable path representation appears necessary
- Stage 0 needs raw source or token grammar
- project dependency availability becomes implicit
- mixed-origin namespace values appear necessary
- old and new source grammar would coexist
- more than two unlisted stage boundaries change
- a phase exceeds roughly 12 production files or 600 net production lines without an approved split
- a user failure would require `CompilerError`

## Phase 0 - adoption and activation

### Phase 0A - queued adoption

- add this plan and the split index
- mark the old combined plan as superseded
- update the package dependency plan's ownership boundary
- do not change the roadmap in this slice
- do not change source syntax or implementation

### Phase 0B - activate after canonical Gate D

- verify canonical module Phase 5 Gate D is accepted
- refresh `main`, owners, test counts and baseline commit
- verify the retained preparation and provider facts are syntax-independent
- record any owner moves from Phase 5 cleanup
- update language, compiler and build authorities with the accepted dependency grammar
- keep current support status truthful until the grammar cutover
- stop for activation review

## Phase 1 - retained path syntax and clause ownership

### Goal

Replace expanded path payloads and token rescans while preserving current `import @...` behavior for this internal phase only.

### Work

- introduce `PathSyntaxId` and `PathSyntaxTable`
- thread the table through file tokens and preparation
- preserve nested group and alias locations
- change path tokens to dense handles
- remap and rebind table rows once
- freeze referenced rows for generic bodies
- introduce `DependencyShellId`
- give one shell to each current authored import clause
- introduce scanned, retained and bound dependency clause types
- make Stage 0 consume retained clause facts
- delete provider-reference token rescanning
- migrate internal terminology where the owner is now grammar-neutral
- preserve current user grammar and behavior

### Tests

- nested selection tree preservation
- alias location preservation
- one shell for many selected leaves
- path table string remap
- source identity rebind
- frozen generic path round trip
- Stage 0 performs no token rescan
- existing import integration cases remain unchanged

### Exit gate

- one path table owns all authored paths
- one retained dependency clause owner feeds Stage 0 and binding
- no raw token rescan remains
- no user syntax changed
- stop for review

## Phase 2 - breaking dependency grammar cutover

### Goal

Remove `import`, implement direct dependency clauses and migrate the repository in one accepted slice.

### Work

- remove `TokenKind::Import` and keyword policy
- parse path clauses directly at top level
- implement extensionless source/package forms
- implement registered explicit-extension provider forms
- implement namespace, alias, direct group and filtered namespace semantics
- implement nested selection groups
- enforce one provider origin
- implement explicit grouped public re-exports
- implement migration diagnostics
- prove `import` is an ordinary identifier
- reject clauses in config and `.mtf`
- keep `#Import` unchanged
- migrate all `.moth` sources, fixtures, packages, benchmarks and generated-test input
- migrate executable docs and scaffolds
- update compiler-owned code highlighting
- update the external VS Code grammar on its own branch
- search for obsolete current-syntax `import @` and classify migration tests explicitly

### Required integration contracts

- bare extensionless namespace
- aliased namespace
- direct grouped bindings
- filtered namespace
- nested child namespace
- aliased child binding
- grouped public re-export
- provider grouped clause
- provider namespace clause
- invalid bare provider clause
- providerless explicit-extension diagnostic
- recognized source-extension diagnostic
- invalid empty group
- invalid nested `@`
- duplicate selected path and local-name diagnostics
- old syntax migration diagnostic
- ordinary identifier named `import`
- config and `.mtf` rejection
- receiver method visibility parity

### External highlighting gate

```bash
npm test
npm pack --dry-run
```

### Exit gate

- one current dependency grammar remains
- no old parser or fallback exists
- every executable source and doc uses the new syntax
- source and external highlighters agree
- stop for review

## Phase 3 - consolidation and final handoff

### Work

- delete obsolete import-oriented parser owners and diagnostics
- remove migration-only internal names that no longer describe ownership
- consolidate path and dependency indexes under their actual stage owners
- ensure graph and provider consumers accept only retained typed clauses
- remove stale import terminology from compiler docs and implementation maps
- keep runtime import terminology unchanged
- run duplicate-work counters
- review progress matrix and docs status

### Final deletion audit

- no `TokenKind::Import`
- no source import parser
- no expanded grouped path payload
- no one-shell-per-selected-leaf behavior
- no provider token rescan
- no path-string or suffix provider joins
- no dependency clause in config or `.mtf`
- no compatibility grammar
- no project dependency auto-registration from source clauses
- no mixed-origin namespace value

### Final validation

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
```

## Exit state and handoff

The plan is complete when:

- each authored path has one retained syntax tree
- each authored dependency clause has one shell identity
- Stage 0 consumes retained typed facts without tokens or source text
- source syntax has no `import` keyword
- file-local clauses reference only registered roots
- provider and source/package clauses use one binding substrate
- all syntax, docs and editor tooling agree

Then run the TIR corrections and simplification plan.

After TIR acceptance, the resource plan consumes:

- `PathSyntaxId` and retained path trees
- `DependencyShellId` and bound clauses
- module-root ownership
- exact provider and package identities
- one source preparation pass
- no source rescan

## Deliberately deferred

- builtin `Path`
- resource identity and source registry
- TIR and HIR resource anchors
- public resource values
- asset placement and URL rendering
- resource-only dev updates
- external package declaration syntax and resolver policy
- package registries, versions, lockfiles and fetching
