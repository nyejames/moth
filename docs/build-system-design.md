# Moth Build System Design

Moth's build system selects a command and capability surface, bootstraps project config, discovers source, constructs project and package graphs, schedules compiler work, plans linked artefacts and owns output writing.

This document is the single source of truth for accepted build-system, project graph, builder, tooling, link and output architecture. It describes the intended end state, including contracts that are not fully implemented yet. It is not an implementation-status report.

`docs/compiler-design-overview.md` is mandatory prerequisite reading. It owns semantic identities, public interfaces, compiler stages, module artefact contents, generated-function compilation, fingerprints and target-validation semantics. This document owns how projects and packages orchestrate those compiler contracts.

Companion authorities:

- `docs/compiler-design-overview.md` for core compiler architecture
- `docs/src/docs/codebase/language/overview.mtf` and the canonical unsuffixed references it selects for source syntax and language semantics
- `docs/src/docs/codebase/design-scope/overview.mtf` for design bias and scope boundaries
- `docs/src/docs/codebase/memory-management/overview.mtf` for reference semantics, borrow validation, lifetime topology, declared groups, ownership, GC and backend memory lowering
- `docs/src/docs/codebase/style-guide/style-guide.mtf` for implementation standards
- `docs/src/docs/progress/@page.moth` for current support and backend coverage
- `docs/roadmap/roadmap.md` and `docs/roadmap/plans/` for implementation order and genuinely deferred design

## Architectural invariants

- One command selects one artefact builder and any active tooling overlays before config schema validation begins.
- `config.moth` is one self-contained compile-time source file with no source imports or package resolution.
- Stage 0 owns one canonical graph, file ownership, legal project/module graph topology and deterministic scheduling for each project or package boundary.
- A physical module is semantically compiled once inside that boundary.
- Tokenization and declaration-shell parsing happen once. Stage 0 reuses prepared syntax for graph construction, later interface binding and module compilation.
- Structural provider references, imported symbol bindings and module-local declaration-ordering edges are different data classes.
- Successful module and dependency artefacts are immutable.
- A diagnosed module exposes no partial public interface.
- Tooling may inspect successful independent branches, but project builders receive success-only linkable project payloads.
- Entry activation, package assembly and backend partitioning never trigger deferred source compilation.
- Project builders consume compiled graphs and explicit link plans. They do not rediscover source structure.
- The build system owns output validation, writing, manifests and stale cleanup.
- Parallel scheduling, reuse and caching preserve deterministic identities, diagnostics and output order.

## Selected command and capability surface

Bootstrap starts with the command rather than with `config.moth`.

The command selects:

- the active artefact builder
- the active build profile
- active tooling overlays such as `check`
- explicit build inputs
- target intent and command-specific options

The current CLI selects the HTML builder implicitly. Final builder-selection syntax and a possible Moth-native build script system remain deferred.

The selected builder exposes a bootstrap capability surface before config compilation:

- project config schema
- entry config schema
- tooling overlay schemas
- source-backed Core and Builder packages
- binding-backed packages
- style directives
- external import providers
- builder runtime packages
- supported source file kinds
- builder-provided primitive build globals
- target-affinity and capability metadata

Frontend-owned directives and compiler-owned builtins are added to this surface. A builder cannot replace them.

One artefact builder runs per `build` or `dev` invocation. Tooling overlays extend analysis and validation. They do not become competing artefact builders.

## Project bootstrap

### Self-contained `config.moth`

`config.moth` is build-system-owned compile-time Moth source. It is not a module and produces no HIR, `start`, runtime artefact or package interface.

Config bootstrap operates on exactly one authored source identity. It does not construct:

- a package resolver
- a config import graph
- a config source set
- a second project source scan

An authored `import` declaration is rejected before path resolution with a structured diagnostic.

Config uses the ordinary compiler owners for its one file:

```text
tokenization
-> declaration-shell parsing
-> local declaration ordering
-> AST semantic checking and folding
-> folded config values
```

Config stops after the folded AST boundary. It produces no HIR or borrow facts.

Allowed source includes:

- one required open `project` const record
- private top-level helper constants declared before their uses
- top-level builder and tooling section records
- scalar and optional constants
- anonymous const records
- collections of supported folded values
- foldable templates represented by their folded string result

Rejected source includes:

- every source import, including relative, project, Core, Builder, dependency and binding-backed imports
- runtime declarations
- mutable bindings
- functions
- named support types
- traits and conformances
- standalone top-level templates
- page fragments
- `export:`
- nested config files or companion config sources

Project config creates no source-visible declarations. Its folded outputs enter the project through specialised build-system interfaces only.

Short shape:

```moth
default_channel #= "alpha"

project #= |
    name = "moth_docs",
    version #Import of String = "0.1.0",
    entry_root = "src",
    metadata = |
        channel = default_channel,
    |,
|

html #= |
    dev_output = "dev",
    release_output = "release",
|
```

`config.moth` does not select the builder. The command has already done so.

### Project record

The open `project` record is required.

`project.name` is required, must be a valid package-style identifier and provides stable project identity. It is not inferred from the checkout directory.

Compiler-owned project fields are strictly schema-validated. Additional folded metadata is allowed.

Public project values may contain:

- folded scalar values
- optional scalar values
- nested anonymous const records
- collections of supported folded values
- folded templates represented as strings

Project fields follow ordinary anonymous-record initializer rules. They do not gain implicit sibling scope. Reusable derived values belong in earlier private helper constants.

The completed `project` record must be available before a builder or tooling section references it.

### Direct project `#Import` fields

A direct primitive or optional field of `project` may declare a build-input contract.

Accepted imported-value types are:

- `String`
- `Int`
- `Float`
- `Bool`
- `Char`
- optional forms of those types

Nested project fields cannot declare `#Import`. Nested project fields do not provide unqualified source input values.

`#Import` is constant-source syntax rather than a source import or wrapper type.

A direct project `#Import` value resolves in this order:

1. explicit CLI or programmatic build input
2. builder-provided primitive global
3. the folded declaration default
4. a structured missing-input diagnostic

Resolution happens during config compilation before Stage 0 applies fields such as `entry_root`.

Project defaults may use the ordinary allowed single-file config constant surface. Their final folded value becomes part of the project-wide contract.

A fixed direct project field is not an import contract. When a same-name source `#Import` uses the same primitive type and optionality, the fixed field is its authoritative provider and blocks CLI override. Same-name source declarations must still agree with each other on required or default state and on the normalised default value.

### `ProjectGlobalsInterface` and `@project`

The folded `project` record produces a specialised immutable `ProjectGlobalsInterface` under the permanently reserved `@project` import root.

The interface contains:

- stable field identities
- folded backend-neutral values
- source locations
- field-level fingerprints
- project-context provenance
- no AST
- no HIR
- no runtime body

It is classified as project-local and Moth-source-backed for provenance and capability purposes, but it is not discovered as a normal source package.

`@project` exposes direct project fields as namespace members. It does not expose another value named `project`.

Normal project modules and project-owned support packages may explicitly import `@project`. It is never implicitly injected.

The following may not claim the `@project` root:

- child modules
- scoped support packages
- dependency aliases
- Core packages
- Builder packages
- binding-backed packages

`@project` cannot be directly re-exported.

Internal project modules may expose declarations derived from project values. The compiler retains project-context provenance on every affected public semantic fact. The external project package facade rejects prohibited project-context exposure.

Project field dependencies are recorded at field granularity. A field change invalidates only semantic, implementation, root or link facts that actually depend on it.

### Source `#Import` contracts

Source `#Import` is intentionally narrow so every project-wide contract can be validated before module AST compilation.

A source declaration may use only the accepted primitive or optional types listed for project fields.

A source default must be self-contained. The only accepted forms are:

- a `String` literal
- a signed `Int` literal
- a signed `Float` literal
- a `Bool` literal
- a `Char` literal
- `none` for an optional contract
- a matching primitive literal for an optional contract

Source defaults cannot contain:

- a name or constant reference
- a template
- an operator expression
- a call
- a cast
- a field projection
- a collection
- a record
- another imported value

This restriction is deliberate. Stage 0 does not run a second general constant evaluator before AST.

Header syntax preparation normalises each source contract into a small build-input shape:

```rust
pub struct SourceBuildInputContract {
    pub name: BuildInputName,
    pub value_type: BuildInputType,
    pub required: bool,
    pub default: Option<PrimitiveBuildValue>,
    pub location: SourceLocation,
}
```

Exact names may change. `BuildInputType` is limited to the accepted primitive and optional domain. `PrimitiveBuildValue` stores a normalised literal value or `none`.

The barrier validates all contracts in the command's selected source graph before module AST compilation.

Same-name contracts must agree on:

- primitive type
- optionality
- required or default state
- normalised default value

Different defaults are conflicting contracts.

The project-wide resolution order is:

1. a compatible fixed direct project field, which is authoritative and cannot be overridden
2. a resolved direct project `#Import` field
3. explicit CLI or programmatic input for a source-only contract
4. a builder-provided primitive global
5. the shared source default
6. a missing-input diagnostic

A direct project `#Import` contract and every same-name source contract must still agree before the resolved project value is supplied to source modules.

Unknown explicit inputs are diagnosed only after every selected source contract is known.

The resolved value enters module AST as an ordinary folded constant. It creates no runtime wrapper or HIR category.

### Builder and tooling sections

Every top-level const record other than `project` is a potential builder or tooling section.

The active artefact builder project section is required, even when empty.

Each builder or tooling overlay declares separate recursive schemas for project settings and entry settings.

A schema may declare:

- accepted fields
- nested record shapes
- folded value shapes
- required fields
- defaulted fields
- closed value domains
- stable section and field identities where useful

The active builder and active tooling sections are schema-validated. Unknown fields in an active section are diagnostics.

Inactive or unavailable sections are still parsed, name-resolved and folded. They are not schema-validated and are not retained in `ProjectCompilation`.

This permits one config file to contain future or inactive sections without loading every schema.

Duplicate section names are rejected. A section name cannot collide with another top-level constant.

Builder and tooling sections cannot declare `#Import` fields. They consume already folded project values and use backend-neutral folded values rather than builder-specific nominal types.

Project and entry schemas do not share fields. There is no `ProjectAndEntry` or equivalent shared-scope escape hatch. Project and entry settings do not implicitly inherit, merge or override one another.

Complex release optimisation remains outside the fast frontend path unless correctness requires it.

### Entry-local `config:` blocks

An entry `config:` block is root-local builder metadata. It is not an embedded `config.moth` source file.

Placement rules:

- valid only at the top level of a normal module root
- at most one block per normal root
- invalid in normal non-root files
- invalid in support roots
- invalid in the project package facade
- invalid inside `export:`
- invalid inside executable bodies
- invalid in `config.moth`

The block contains section records only.

Imports, aliases, helper constants, support types and source `#Import` declarations live outside the block in the normal root file.

The block uses the root file's ordinary compile-time visibility. It may reference:

- imported constants
- `@project`
- same-file constants declared before the block
- resolved source `#Import` constants
- foldable local const-record types
- selected-builder compile-time values available through normal module imports

Same-file forward references remain invalid.

Header syntax records its local dependencies. AST folds it through the ordinary module semantic path.

The block creates no ordinary module symbol, HIR or project-global value.

It cannot contain `project` or change project-level builder behaviour.

It may contain active artefact-builder and tooling-overlay sections.

Active entry sections are schema-validated. Inactive sections are parsed and folded but not schema-validated.

The block is optional. Its active artefact-builder section is also optional so tooling-only metadata remains possible.

Every normal module selected into the current command's semantic graph has its block validated whether or not an entry activates it. Imported modules never apply their entry metadata to an importer.

Only active artefact-builder settings contribute entry activity.

### Fixed bootstrap order

The command and bootstrap flow is:

```text
select command, artefact builder, build profile and tooling overlays
-> construct compiler and builder bootstrap capability surface
-> compile and validate config
-> derive entry_root and @project
-> build the canonical source index and provider graphs
-> resolve build-input contracts
-> compile dependency-ordered waves
   -> bind provider interfaces
   -> order local declarations
   -> run AST semantics
   -> lower and validate HIR
   -> borrow-validate
   -> produce local lifetime constraints, lifetime facts and exported summaries
-> complete the generated-function worklist
-> assemble a success-only ProjectCompilation
-> plan entry/package roots and exact reachable unions
-> instantiate and validate complete lifetime topology
-> plan target assignments and validate them
-> lower backend artefacts
```

Config compilation tokenizes and parses one self-contained `config.moth`, orders config declarations, resolves direct project `#Import` sources while AST folds config, and validates the completed project record and active project sections. Inactive config sections are folded during config compilation even though their schemas are not active. Project config creates no source import graph.

## Source indexing and source sets

After config supplies `entry_root`, Stage 0 builds one canonical source index for the project boundary.

Directory-project `entry_root` must be a relative directory strictly below the project root.

Reject:

- an empty path
- `.`
- parent components
- an absolute path
- a path outside the project root
- a symlink-resolved path equal to the project root

Single-file compilation remains a separate synthetic-module mode.

The source index owns:

- canonical logical source identities
- normal and support root discovery
- the optional project-root package facade
- nearest module ownership
- builder-supported source-kind candidates
- explicit provider-owned files
- extensionless namespace identities
- path collision facts
- deterministic discovery order

`package_folders` and default `/lib` scanning do not exist. Project-local source packages are structural `+*.moth` packages or the optional project-root facade.

### Owned source set

A module's `OwnedSourceSet` contains every recognised source file whose nearest root is that module.

Ownership determines:

- legal filesystem boundaries
- collision scope
- diagnostic attribution
- orphan detection
- deterministic inventory identity

The semantic source set determines the module's semantic source fingerprint. Check-only units have separate tooling fingerprints. Ownership alone does not inject declarations into the compiled module.

### Semantic source set

A module's `SemanticSourceSet` contains:

- its root file
- every owned `.moth` file reachable through source imports
- every reachable builder-supported source asset such as `.mtf` or `.md`
- any other source-kind input explicitly defined as semantic by the selected builder

Only the semantic source set contributes declarations, HIR, the public interface and module link facts.

Provider-backed explicit-extension files are owned through their provider contract. They produce binding-backed interfaces and runtime facts rather than ordinary module declarations.

### Check source set

`check` also examines owned `.moth` files that are not in the canonical semantic source set.

Each orphan becomes a check-only source unit under its nearest module namespace. It may be parsed, bound and semantically diagnosed with the same provider interfaces and visibility rules, but it does not silently add declarations to the canonical module artefact or public interface.

A check-only unit cannot become a backend root or link input.

This distinction lets tooling diagnose abandoned or disconnected source without changing import semantics.

## Prepared-source orchestration

Stage 0 asks the compiler to perform tokenization and header syntax preparation once for each selected source candidate.

Prepared syntax may contain:

- tokens or source-kind payloads
- declaration shells
- import shells
- structural provider references
- local declaration-ordering hints
- source `#Import` contract shells
- dormant root activity shells
- compile-time fragment placement metadata
- diagnostics and warnings
- deterministic string-table deltas or remap information

Stage 0 consumes structural provider references to finalise graphs. It does not bind source symbols itself.

When a provider interface is available, the compiler's interface-binding phase resolves retained import shells into stable imported symbol bindings and final visibility. Binding does not reparse source.

The three classes remain distinct:

- structural provider references for Stage 0
- imported symbol bindings for compiler visibility and AST
- local declaration-ordering edges for compiler Stage 3

Stage 0 never implements a competing import grammar or lightweight scanner that later reparses the same syntax surface.

Provider-backed discovery remains serial while it mutates shared package identities, provider caches, resolution tables or diagnostic identity. Parallel provider discovery requires deterministic provider deltas and remapping first.

Stage 0 produces structure, resolved build-input contracts and compiler inputs. It does not type-check executable bodies, generate HIR or perform borrow validation.

## Project and package topology

Terminology is strict:

- A module is one directory-scoped compilation and visibility unit rooted by `@*.moth` or `+*.moth`.
- A package is a named reusable `@...` import root and future dependency or distribution unit.
- A binding is a typed bridge to an implementation outside Moth source.
- A prelude is implicit import policy rather than a package kind.
- Library is informal wording only.

### Module roots and dormant work

A directory contains at most one module root.

- `@*.moth` defines a normal module.
- `+*.moth` inside a project source tree defines an API-only scoped support module.
- One optional project-root `+*.moth` beside `config.moth` defines the external project package facade.
- The suffix after `@` or `+` is cosmetic.
- `config.moth` is not a module root.

A normal module may own dormant top-level runtime work and page fragments.

Support modules and the project package facade are API-only:

- no implicit `start`
- no top-level runtime statements
- no page fragments
- no route or builder artefact
- ordinary runtime code inside functions remains valid

`export:` is the only public visibility marker.

Every normal module in the command's semantic graph has dormant root work fully compiled, borrow-validated and locally lifetime-analysed. Entry assembly activates already compiled work only.

### Module-root-relative imports

Source imports resolve from the importing file's owning module root rather than the file's physical directory.

Example:

```text
src/
├── @site.moth
├── accounts.moth
└── internal/
    └── deep/
        └── renderer.moth
```

Inside `renderer.moth`:

```moth
import @accounts { Account }
```

This resolves to `src/accounts.moth`. It does not search beside `renderer.moth`.

Rules:

- `@./...` has no supported meaning.
- Parent components are invalid.
- Paths may traverse ordinary unrooted directories owned by the same module.
- Reaching a child normal module or support package ends filesystem traversal and exposes only its facade.
- Paths such as `@child/internal` cannot bypass a facade.
- Scoped support packages are injected by package name.
- Provider imports use an explicit owner and do not silently reintroduce file-relative lookup.
- Compiler interface binding consumes the Stage 0 namespace and does not probe ordered fallback candidates.

A normal module may import:

- ordinary files it owns
- unrooted directories it owns
- direct child normal modules
- support packages visible in its lexical scope
- registered Core, Builder and dependency packages
- provider files explicitly permitted by the active builder

A normal module may not import:

- its parent
- an ancestor
- a normal sibling
- a grandchild directly
- a sibling's descendant
- an unrelated branch
- another module's private file path

A child module re-exports anything its parent should see from deeper descendants.

Valid normal-module topology is acyclic by construction. Stage 0 retains a defensive cycle validator for malformed internal state and future extensions.

### Scoped support packages

A `+*.moth` support root exposes a package named by its containing directory.

Example:

```text
site/
├── @site.moth
├── markdown/
│   ├── +package.moth
│   ├── parser/
│   │   └── @parser.moth
│   └── rendering/
│       └── @rendering.moth
└── pages/
    ├── @pages.moth
    └── article/
        └── @article.moth
```

`@markdown` is visible to `site`, `pages` and `article`. Its private descendants may be imported by the `markdown` facade, but consumers cannot address them through `@markdown/parser` or another implementation path.

For a support package `S` whose nearest ancestor normal module is `P`:

- `S` is visible to `P`.
- `S` is visible to normal sibling modules and their descendants.
- `S` is not visible above `P`.
- `S` is not visible outside `P`'s subtree.
- `S` is not imported from its own private implementation descendants.
- Another support package in the same owner scope cannot import `S`.

The support facade may import:

- ordinary files it owns
- any descendant module in its private subtree
- support packages from a strictly outer scope
- registered packages

It may not import its parent, normal sibling consumers or same-scope support siblings.

Consumers see only the support facade's `export:` surface.

The same support-package name may appear in disjoint scopes. Overlapping scopes are rejected with diagnostics that point to both declarations and explain the overlap.

Direct normal-sibling imports remain disallowed. A future design may revisit them only with real project evidence, cycle diagnostics and a reason the shared behaviour cannot live in a scoped support package.

### Project package facade

The project-root `+*.moth` facade is a canonical API-only module compiled through the ordinary compiler pipeline with project-facade visibility supplied by Stage 0.

It may define and export its own legal API-only declarations.

Stage 0 gives the facade a special assembly namespace rooted at `entry_root`. Through that namespace it may reference the public interfaces of descendant modules below `entry_root`, regardless of ordinary lexical module visibility.

The facade:

- never bypasses an `export:` boundary
- is not visible to internal project modules
- cannot import `@project`
- cannot expose a semantic fact that depends on project-private context
- has no root runtime activity
- emits no route

Structural facade dependencies ensure providers compile before the facade.

The compiler produces an immutable facade module artefact and public interface.

`ProjectPackageAssembly` is a separate link plan over:

- the compiled facade artefact
- selected descendant public interfaces
- reachable generated functions
- package runtime requirements permitted by the target

Assembly never recompiles or mutates the facade.

A project may be both an application and a package. Without the facade it has no externally consumable Moth package surface.

The facade package identity comes from `project.name`.

### Namespace and collision policy

No import uses precedence, nearest-match shadowing or ordered fallback.

Reject overlapping visible identities between:

- `@project`
- scoped support packages
- direct child normal modules
- extensionless source files
- internal directory path segments
- the external project package name
- Core package roots
- Builder package roots
- dependency aliases
- case-only variants

Recognised extensionless source kinds share one namespace. `docs.moth`, `docs.mtf`, `docs.md` and `docs/` cannot coexist where each would mean `@docs`.

Explicit-extension provider files may coexist with a same-stem directory only when syntax remains unambiguous.

Diagnostics point to every conflicting declaration and explain the scope in which the identities overlap.

### Package classification

Packages are classified on independent axes.

```rust
enum PackageOrigin {
    Core,
    Standard,
    Builder,
    ProjectLocal,
    Dependency,
}

enum PackageBacking {
    MothSource,
    ExternalBinding,
}
```

Accepted mappings include:

- `@html`: Builder origin and MothSource backing
- Core packages such as `@core/io`: Core origin and ExternalBinding backing
- `@web/canvas`: Builder origin and ExternalBinding backing
- scoped `+*.moth`: ProjectLocal origin and MothSource backing
- project-root facade: ProjectLocal origin and MothSource backing
- annotated project-local `.js`: ProjectLocal origin and ExternalBinding backing
- dependency source package: Dependency origin and MothSource backing

`Standard` remains valid even when no current package uses it.

Origin and backing classify provenance and implementation. They do not change:

- import syntax
- namespace precedence
- visibility
- export or facade privacy
- receiver-method behaviour

Source and binding registries remain separate because discovery, semantic and runtime needs differ.

A precompiled artefact preserves the package's semantic backing classification. Precompiled is an artefact storage state rather than another `PackageBacking` variant.

### Dependency package graphs

A source dependency compiles as a separate package graph. It is not merged into the consuming project's module graph.

Each dependency owns:

- its own config
- its own private `@project`
- its own source index and module graph
- immutable compiled module artefacts
- its external package facade
- semantic and compatibility fingerprints

A dependency never sees the consuming project's `@project`.

Dependencies compile against the active target builder's frontend capability surface. Compatibility records the Core and Builder capability interfaces actually used rather than only a builder class name.

Consumers use the dependency package facade and immutable package artefacts.

No declaration exposed through a dependency package facade may directly or transitively depend on that dependency's private `@project`.

The prohibition applies to both public semantic facts and executable implementation. It covers:

- exported constants and defaults
- canonical public types
- generic bounds and templates
- trait evidence
- receiver surfaces
- access and effect summaries
- exported function bodies
- source or generated functions reachable from an exported declaration
- compile-time-derived implementation facts
- every other public-interface or executable fact selected by the facade

A declaration that depends on private `@project` remains internal to the dependency. It cannot be selected by the external facade, re-exported through the facade or reached from an exported function.

Private declarations may use the dependency's own `@project` only when no external package export can reach or expose them. Their config dependence remains part of the dependency's implementation and compatibility fingerprints.

Persistent or precompiled dependency artefacts may later replace source compilation without changing this semantic model.

Package declaration syntax, registries, remote fetching, version solving and lockfiles remain deferred.

Imported build-value namespaces are scoped to one project or package compilation boundary. A consuming command's unqualified CLI or programmatic inputs do not implicitly satisfy a dependency's #Import contracts. 

A dependency resolves its contracts from its own config, defaults and compatible builder-provided globals. No implicit cross-boundary input lookup or same-name inheritance is allowed.

### Core and Builder source package graphs

Source-backed Core and Builder packages compile as separate immutable package graphs. Their private implementation does not join the consuming project graph.

They do not receive the consuming project's `@project`.

A builder package that genuinely requires project-specific compile-time input must receive an explicit builder-owned synthetic interface declared in capability metadata. That interface:

- is not `@project`
- is not implicitly injected
- carries provenance and fingerprints
- makes the resulting package artefact project-specific

Pure package artefacts remain reusable when their required capability fingerprints match.

Binding-backed Core and Builder packages remain virtual semantic interfaces rather than source module graphs.

## Deterministic scheduling and graph outcomes

### Compile waves

Stage 0 finalises structural edges and produces deterministic dependency-ordered compile waves.

A source provider compiles before a consumer that needs its public interface.

Within a ready wave, parallel work is allowed only when:

- graph dependencies permit it
- identity assignment is deterministic
- string-table deltas merge in canonical order
- diagnostics and warnings are ordered independently of completion time
- completed payloads are remapped before consumers use them

For each module job:

```text
receive retained syntax and completed provider interfaces
-> bind import shells
-> order local declarations
-> run AST semantics
-> lower and validate HIR
-> borrow-validate
-> produce local lifetime constraints, lifetime facts and exported summaries
-> return Success or Diagnosed
```

Local module compilation cannot validate every cross-module or builder-lifecycle relationship by itself. Project and link planning instantiate lifetime summaries over the reachable call graph and builder-supplied lifecycle roots.

A source provider diagnosis blocks its semantic consumers. Independent branches continue.

A `CompilerError` aborts the project or package compilation.

### Graph compilation outcome

The build system records a batch result that can preserve useful independent work for tooling.

```rust
pub struct GraphCompilationOutcome {
    pub successful: Vec<CompiledModuleArtifact>,
    pub diagnosed: Vec<ModuleDiagnostics>,
    pub blocked: Vec<BlockedModule>,
}
```

`BlockedModule` records the module and required provider that prevented semantic compilation. It is not a user-facing cascade diagnostic by default.

Rules:

- A diagnosed module exposes no partial interface.
- A blocked module is not semantically compiled.
- Independent successful artefacts may remain available to `check` and future LSP analysis.
- Shared module diagnostics are emitted once.
- The renderer does not hide duplicated work. The graph prevents duplicate module diagnostics from being produced.

### Success-only `ProjectCompilation`

`ProjectCompilation` is assembled only when every artefact required by the selected entries or package surface succeeded.

Conceptual shape:

```rust
pub struct ProjectCompilation {
    pub structure: ProjectModuleGraph,
    pub project_globals: ProjectGlobalsInterface,
    pub modules: Vec<CompiledModuleArtifact>,
    pub generated: Vec<GeneratedFunctionSidecar>,
    pub entries: Vec<EntryAssembly>,
    pub package_facade: Option<ProjectPackageAssembly>,
}
```

It is coherent and linkable. A project builder never receives diagnosed or blocked required modules.

For `build` and `dev`, any diagnosed required module, generated request or package surface prevents backend invocation.

For `check`, the command may retain successful independent artefacts internally while reporting diagnostics. It does not pretend a partial `ProjectCompilation` is linkable.

## Command and tooling policies

### `build`

`build` compiles the union required by:

- builder-selected artefact entries
- the optional project package facade when one exists
- direct and transitive source providers
- required Core and Builder source package graphs
- dependency package facades and artefacts
- generated requests discovered by the fixed-point worklist

It performs target validation, backend lowering and output writing when compilation succeeds.

### `dev`

The first `dev` build compiles the complete graph required by its selected entries and package policy.

Later rebuilds reuse successful in-memory artefacts according to the fingerprint and invalidation rules below. Dev-server orchestration does not create a second compiler or builder architecture.

### `check`

`check` compiles:

- every discovered project module below `entry_root`
- check-only orphan source units
- the optional project package facade
- required Core and Builder source package graphs
- required dependency package surfaces
- reachable generated requests

It applies selected-target planning and validation to actual linkable roots without backend code generation or output writing.

Unsupported target features in unreachable private functions do not fail `check`.

### Tooling overlays

`check` and future LSP support are overlays over the selected builder surface.

An overlay may add:

- diagnostics
- lint policy
- analysis outputs
- tooling config schema
- additional callable validation roots

It does not duplicate target packages, source kinds, directives, binding metadata or capability definitions.

Tooling-only entry config never creates an artefact entry.

## Generated-function worklist

The compiler owns generic template validation, call-site inference, request identity, generated HIR, generated borrow facts, and generated lifetime facts and summaries.

The build system owns:

- project-wide or package-wide request aggregation
- deterministic deduplication
- worklist scheduling
- sidecar placement
- reuse across entries

Requests are keyed by stable generic declaration identity, canonical concrete type identities and required evidence identities.

The worklist continues until no generated function requests another instance.

Each successful generated sidecar entry carries its own generated-local type context, HIR, borrow facts, lifetime facts and summaries, link facts and fingerprints. It does not mutate a base module artefact.

Cross-package instances belong to the consuming compilation. Dependency base artefacts remain immutable.

A diagnosed generated request blocks only entries or package exports that require it. The build system does not expose a partial generated artefact.

## Entry and package link planning

### Entry candidates and selection

Builder-relevant root activity selects normal modules as artefact entries.

For HTML, entry activity includes:

- dormant root runtime work
- compile-time page fragments
- runtime page fragments
- resolved active HTML entry settings

A tooling-only section does not create an artefact entry.

Prepared root-activity shells identify early candidates. When entry status depends on folded metadata, the candidate module compiles before final selection.

One canonical normal module may produce several `EntryAssembly` values. The HTML builder initially produces at most one route entry per normal module.

### Entry assembly

An `EntryAssembly` selects one already compiled normal module and activates only that module's:

- compiler-synthesised `start`
- dormant top-level runtime work
- runtime page fragments
- compile-time page fragments
- resolved active entry settings
- entry-owned runtime requirements

Imported normal modules expose public interfaces without executing root work.

Support modules and the project package facade never execute root work.

Entry assembly never triggers parsing, type checking, HIR generation, generic inference, borrow validation or lifetime-region validation.

The implicit `start` is non-exported, non-importable and infallible. The builder does not define a fallible start channel or an error-fragment policy.

### Package assembly

`ProjectPackageAssembly` selects the compiled project facade, descendant public interfaces, generated functions and permitted runtime requirements needed for the external package surface.

It does not change semantic visibility or bypass `export:`.

A package assembly diagnosis prevents publication or package-target lowering. It does not mutate compiled base artefacts.

### Per-function reachable unions

The compiler records link facts per executable function. The build system computes exact reachable unions for each entry or package assembly.

A union may include:

- linked source functions
- generated functions
- binding-backed calls
- helper and capability families
- reactive features
- numeric and cast operations
- maps and target-gated features
- runtime paths and assets

Module-wide summaries may be cached as derived indexes. They are not the linking authority.

The build system does not repeatedly scan source, rebuild imports or reopen AST to discover runtime dependencies.

### Target-validation roots

The build system supplies explicit roots to compiler-owned validation.

Roots may include:

- an entry's active `start` and linked callable graph
- reachable generated functions
- externally callable project package exports
- additional callable roots declared by the selected builder or tooling overlay

`check` invokes the same planning and validation semantics as the corresponding build, then stops before lowering.

## HTML project builder

The HTML builder owns route, document, browser-runtime and mixed JavaScript and Wasm artefact policy. These choices are not core language semantics.

### Entry and fragment assembly

For each HTML entry, the builder:

1. selects the active normal module
2. activates its already compiled dormant root work
3. merges compile-time fragments at their recorded runtime insertion indexes
4. creates runtime fragment slots
5. invokes active `start` once through the selected runtime path
6. hydrates runtime fragments in source order
7. assembles route HTML and companion artefacts

HIR carries runtime code only. Compile-time fragments and document structure live in compiler metadata and entry plans.

Modules without HTML artefact activity remain available to the graph but produce no route, runtime glue or tracked assets.

### Mixed-target planning and validation

The fixed sequence is:

```text
entry or package roots
-> exact reachable function and effect union
-> instantiate local lifetime summaries with builder lifecycle roots
-> validate complete lifetime topology
-> target-affinity and capability analysis
-> deterministic target partition
-> validate assigned functions and permitted cross-target edges
-> lower selected functions
```

`check` runs the same sequence and stops before lowering.

Partition rules:

- `start` is JavaScript-owned.
- DOM, browser, project JavaScript and other JS-required dependencies force the containing function to JavaScript.
- JavaScript requirements propagate backwards to transitive callers.
- Neutral console IO does not force JavaScript ownership.
- Remaining supported functions default to Wasm.
- No Wasm-owned Moth function may call a JavaScript-owned Moth function after propagation.
- JavaScript-owned functions may call Wasm-owned functions through generated wrappers.
- Every decision records an explicit reason.
- Partitioning is independent of development or release mode.
- Canonical HIR and module artefacts remain shared.

Target affinity comes from semantic package and capability metadata rather than package-name checks.

Validation is a compiler service over the completed build-owned partition. A target failure is reported before target lowering begins.

### Physical variants

Partitioning is entry-specific. Physical variants are deduplicated by a conceptual key containing:

- module identity
- selected concrete function set
- target assignment
- ABI identity
- layout identity
- runtime capability requirements
- relevant backend config fingerprint

Entries with the same key reuse one variant. Different keys produce separate JavaScript companion or Wasm variants.

One source function may be JavaScript in one entry variant and Wasm in another.

Each selected module variant has a generated JavaScript companion facade. Wasm is emitted per selected module variant.

### Link planning and lifetime topology

Project and package link planning instantiates local lifetime summaries with builder lifecycle roots and validates the complete lifetime topology before target assignment. Linking does not reopen source or mutate HIR.

`ProjectCompilation` or the link plan conceptually carries project-level validated lifetime topology. Exact Rust shape remains open.

Builder-supplied page, mount, request, frame and arena roots are lifecycle inputs, not builder-specific source-law exceptions. Builder lifecycles cannot change language validity.

Exported lifetime summaries participate in the public-interface fingerprint. Topology-relevant implementation and link facts invalidate affected assemblies. Exact persistent encoding remains deferred.

External boundary profile and capability metadata belong on the builder surface conceptually so backends receive closed WIT-value or host-binding classifications rather than inventing retention graphs.

### Runtime and memory

Each page owns one runtime instance and one memory shared by its linked Moth Wasm variants.

Linked Moth Wasm variants import the page runtime rather than owning separate memories.

This one-page runtime/memory contract applies to linked Moth Wasm variants. It does not require imported WIT components to share page memory. Imported components own private runtime memory and cross the boundary only through closed value conversion profiles.

Project-level runtime bytes may be emitted once and instantiated separately for each page.

Wasm lowering consumes explicit selected-function, import, export, capability, layout and validated lifetime plans.

Wasm LIR is structured and backend-owned. It is not a second frontend semantic authority.

The final design removes:

- dispatcher-loop control flow as the durable backend shape
- `moth_start`
- per-module memories
- helper-export booleans
- the `i64` Int bridge architecture

These paths are deleted rather than retained through compatibility adapters.

### Lowerer use cases

The HTML JavaScript path:

- lowers the selected JavaScript function set
- emits required runtime helpers only
- renders compile-time fragments into the document
- emits runtime fragment slots
- invokes active `start` once
- hydrates runtime fragments in source order

The HTML page-bundle path uses referenced external-function metadata to emit only the glue wrappers and module imports required by that entry.

HTML-JS reactive mounting remains a JavaScript-owned concern. Ordinary page-fragment assembly is shared with mixed output.

The standalone JavaScript backend may emit a complete bundle when explicitly asked to include every HIR function.

The core standalone Wasm backend owns:

- HIR-to-Wasm-LIR lowering
- Wasm runtime contracts
- request validation
- optional binary emission
- backend debug output

The HTML-Wasm path is project-builder orchestration around that backend.

### External JavaScript

Provider-backed external JavaScript has two emission levels.

Build-level runtime emission deduplicates:

- runtime assets
- required module specifiers
- shared provider runtime files

Entry-level glue generation emits only:

- wrappers for external functions referenced by the selected JavaScript bundle
- required import preambles
- required import-map entries

Direct builder packages and provider-created packages use the same binding identity and runtime asset model.

### Tracked assets

The compiler records semantic path usages while rendering or lowering source values.

The HTML builder decides:

- which paths become emitted assets
- output paths relative to the final route
- deduplication
- conflicts
- user-facing asset warnings

Tracked assets are returned as ordinary output records.

## Output ownership

Artefact builders own output-path settings and defaults in their private project config section.

Builders that produce no artefacts register no output settings.

HTML defaults remain:

- development: `dev`
- release: `release`

A selected builder may override those defaults through its active project section.

Every output root must be:

- relative to the project root
- outside `entry_root`
- free of parent traversal
- contained by the project output policy

The build system owns:

- output-root validation
- skip-unchanged writes
- output manifests
- stale artefact cleanup
- conflict diagnostics

Backends and project builders produce output records. They do not write final project outputs directly.

Output ownership is keyed by stable builder identity and build profile.

Development and release builds cannot silently claim the same root.

An existing manifest owned by another builder or profile causes a structured conflict before writing.

One builder never deletes files owned by another manifest.

Ordinary builder invocations have no force-overwrite escape hatch.

### Deliberate output pipelines

Future minification, obfuscation or another output transformation requires an explicit ordered pipeline.

A transformer receives:

- the previous stage's manifest
- declared input artefacts
- a bounded output contract

The final manifest records the complete pipeline identity.

Independent builders cannot simulate a pipeline by writing over one another's output roots.

Pipeline syntax and implementation remain deferred.

## Incremental and persistent artefacts

The compiler owns the contents of public-interface, implementation, dormant-root, runtime-dependency and documentation fingerprints. The build system owns invalidation and compatibility policy over them.

### In-memory reuse

The first development build compiles the complete required graph.

Later builds reuse successful in-memory artefacts.

A changed module rebuilds.

Semantic dependants rebuild only when the provider's public-interface fingerprint changes. Access and effect summaries are part of that fingerprint. There is no separate exported-effect fingerprint.

Entries relink or regenerate when a linked input changes, including:

- implementation fingerprint
- dormant root-activity fingerprint
- runtime-dependency fingerprint
- generated functions
- active entry settings
- project-field dependencies
- backend config that affects partitioning or output

Documentation-only changes regenerate documentation or editor indexes without invalidating semantic consumers or executable instances.

Private dependency implementation may use the dependency's own `@project` only when no external package export reaches it. Its config dependence contributes to implementation and compatibility keys. Any exported declaration with direct or transitive dependence is rejected before package assembly.

### Persistent compatibility

Persistent serialisation is a later implementation of the same boundaries.

A serialised module, package or generated artefact is reusable only when compatible with:

- compiler semantic artefact format version
- relevant language semantics version
- stable project or package identity
- source fingerprint
- config fingerprint
- imported public-interface fingerprints
- required Core capability-interface fingerprints
- required Builder capability-interface fingerprints
- target-independent frontend feature configuration
- embedded ABI or layout policy
- generated request identity where applicable

Process-local string IDs and absolute filesystem paths are not compatibility identities. Persistent artefacts store canonical logical identities and self-contained or remappable string data.

Incompatible artefacts are discarded and rebuilt.

Normal builds do not attempt best-effort deserialisation, partial migration or compatibility repair.

## Build-system implementation map

Current paths are navigation aids rather than permanent architecture.

- Project bootstrap and config: `src/build_system/project_config/`, `src/projects/settings.rs`
- Source indexing, graph construction and scheduling: `src/build_system/create_project_modules/`, `src/build_system/build.rs`
- Builder capability surface: `src/builder_surface/`
- Commands and tooling overlays: `src/projects/cli.rs`, `src/projects/check.rs`, `src/projects/dev_server/`
- HTML project builder and entry assembly: `src/projects/html_project/`
- JavaScript and Wasm lowerers: `src/backends/js/`, `src/backends/wasm/`
- Output writing and manifests: build-system output and cleanup owners
- Tests, validation and roadmap: `tests/cases/`, `src/build_system/tests/`, `justfile`, `docs/roadmap/`

Compiler frontend, AST, HIR, borrow and target-validation locations are mapped in `docs/compiler-design-overview.md`.
