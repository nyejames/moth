# Moth Build System Design

Moth's build system selects a command and capability surface, bootstraps project config, discovers source, constructs project and package graphs, schedules compiler work, plans linked artefacts and owns output writing.

This document is the single source of truth for accepted build-system, project graph, builder, tooling, link and output architecture. It describes the intended end state, including contracts that are not fully implemented yet. It is not an implementation-status report.

`docs/compiler-design-overview.md` is mandatory prerequisite reading. It owns semantic identities, public interfaces, compiler stages, module artefact contents, generated-function compilation, fingerprints and target-validation semantics. This document owns how projects and packages orchestrate those compiler contracts.

Companion authorities:

- `docs/compiler-design-overview.md` for core compiler architecture
- `docs/src/developer-docs/language/overview.mtf` and the canonical unsuffixed references it selects for source syntax and language semantics
- `docs/src/docs/design-scope/` for design bias and scope boundaries
- `docs/src/developer-docs/memory-management/overview.mtf` for reference semantics, borrow validation, lifetime topology, retained-edge liveness, declared groups, affine ownership, Retained Edge Counting and backend memory lowering
- `docs/src/developer-docs/style-guide/style-guide.mtf` for implementation standards
- `docs/src/docs/progress/@page.moth` for current support and backend coverage
- `docs/roadmap/roadmap.md` and `docs/roadmap/plans/` for implementation order and genuinely deferred design

## Task-reading guide

For every build-system task, read the opening authority text and
`Architectural invariants` in both this document and
`docs/compiler-design-overview.md`. Heading paths use `>` to name nested
sections. Read the selected heading through the next heading of the same or
higher level, including nested subsections unless the route narrows further.
Read both documents in full for cross-boundary architecture plans, broad
refactors or thorough reviews.

| Task | Read in this document | Also read when affected |
|---|---|---|
| Command selection, builder capabilities or tooling overlays | `Selected command and capability surface`; `Command and tooling policies` | The compiler sections for any new semantic capability or validation root |
| `config.moth`, project fields, `#Config`, `@project`, builder sections, entry config or bootstrap order | `Project bootstrap` and the exact relevant subsection | `docs/compiler-design-overview.md` > `Frontend stages > Stage 4: AST semantics > Constants, build configuration values and const records` when compiler folding or handoff changes |
| Source discovery, ownership, semantic source sets, check-only units or source preparation | `Source indexing and source sets`; `Prepared-source orchestration` | `docs/compiler-design-overview.md` > `Compiler input and result boundary` and the relevant Stage 1 to Stage 3 section |
| Module roots, dependency topology, support packages, project facades, namespaces or package classification | `Project and package topology` and the exact relevant subsection | The canonical unsuffixed project-structure and package language references |
| Dependency, Core or Builder source package graphs | `Project and package topology > Dependency package graphs` or `Core and Builder source package graphs` | Compiler public-interface, provenance, fingerprint and generated-function sections |
| Compile waves, diagnosed or blocked modules, deterministic merging or `ProjectCompilation` | `Deterministic scheduling and graph outcomes` | `docs/compiler-design-overview.md` > `Compiler input and result boundary` and `Diagnostics and deterministic identity` |
| Generated request aggregation, scheduling or sidecar reuse | `Generated-function boundary` | `docs/compiler-design-overview.md` > `Generated concrete functions` |
| Entry selection, package assembly, reachability or validation roots | `Entry and package link planning` | Compiler `Per-function link facts`, `Target-contract validation` and routed lifetime material |
| HTML fragment assembly, target partitioning, physical variants, runtime memory, lowering, external JavaScript or assets | `HTML project builder` and the exact relevant subsection | Compiler `Backend-facing compiler handoff` plus routed memory material for lifecycle, ABI or runtime representation changes |
| Resource registries, entry or package resource unions, URL contexts, resource output placement or conflicts | `Resource linking and output placement` | `docs/compiler-design-overview.md` > `Frontend stages > Stage 4: AST semantics > File values and resources` and the canonical resource references |
| Output roots, manifests, stale cleanup or output pipelines | `Output ownership` | The selected builder section that produces the output records |
| Development reuse, invalidation or persistent compatibility | `Incremental and persistent artefacts`; `Command and tooling policies > dev` | `docs/compiler-design-overview.md` > `Fingerprints and reuse facts` |
| Current source locations | `Build-system implementation map` | Open the owning module entry point and the compiler handoff when the task crosses that boundary |

## Architectural invariants

- One command selects one artefact builder and any active tooling overlays before config schema validation begins.
- `config.moth` is one self-contained compile-time source file with no source dependency clauses or package resolution.
- Stage 0 owns one canonical graph, file ownership, legal project/module graph topology and deterministic scheduling for each project or package boundary.
- A physical module is semantically compiled once inside that boundary.
- Tokenization and declaration-shell parsing happen once. Stage 0 reuses prepared syntax for graph construction, later interface binding and module compilation.
- Stage 0 schedules one compiler-owned module compilation service and consumes its outcome. It does not sequence interface binding, declaration ordering, AST, HIR or borrow stages, and it does not mutate compiler semantic state.
- Structural provider references, structural file references, dependency symbol bindings and module-local declaration-ordering edges are different data classes.
- Successful module and dependency artefacts are immutable.
- A diagnosed module exposes no partial public interface.
- Tooling may inspect successful independent branches, but project builders receive success-only linkable project payloads.
- Entry activation, package assembly and backend partitioning never trigger deferred source compilation.
- Project builders consume compiled graphs and explicit link plans. They do not rediscover source structure.
- The build system owns output validation, writing, manifests and stale cleanup.
- Graph and input membership is conservative and comes from the authored file path; output emission is exact and comes from entry or package reachability. Reachability never decides graph membership, and graph membership never forces emission.
- Resource origin, byte source, emitted output path and rendered URL are four separate facts owned by the build system after the compiler has published semantic resource facts.
- Builders assign every reachable resource use one semantic URL context. No builder scans rendered HTML, CSS, Markdown or arbitrary strings to rediscover resources.
- Build configuration may specialise executable behaviour, but it cannot change source discovery, semantic source sets, dependency graphs, declaration or export existence, or package topology.
- Source semantics remain target and platform agnostic.
- Builders and backend capability metadata map stable source semantics to target-specific artefacts.
- Builders must not expose target identity through `#Config`.
- A statically decided Bool `if` is specialised by Stage 4 before HIR and downstream executable analysis.
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

Explicit command inputs are parsed into typed primitive values before config or source contracts are
matched. Programmatic command APIs construct the same typed carrier. Builder-provided primitive globals
must express stable semantic configuration; target or platform identity is not a valid builder-provided
`#Config` global. Target intent remains build-system input and is not source-visible configuration.

One artefact builder runs per `build` or `dev` invocation. Tooling overlays extend analysis and validation. They do not become competing artefact builders.

## Project bootstrap

### Self-contained `config.moth`

`config.moth` is build-system-owned compile-time Moth source. It is not a module and produces no HIR, `start`, runtime artefact or package interface.

Config bootstrap operates on exactly one authored source identity. It does not construct:

- a package resolver
- a config dependency graph
- a config source set
- a second project source scan

An authored dependency clause is rejected before path resolution with a structured diagnostic.

Config uses the ordinary compiler owners for its one file, through one named compiler service:

```text
tokenization
-> declaration-shell parsing
-> local declaration ordering
-> AST semantic checking and folding
-> folded config values
```

Config stops after the folded AST boundary. It produces no HIR or borrow facts.

That sequence is compiler-owned. The build system is a client of the config compilation service: it supplies the one authored source and consumes folded values, authored key locations and diagnostics. It does not compose the stages itself, and config bootstrap is not a second build-owned frontend pipeline. Config schema definition, validation policy and application to the project record stay build-owned. See `docs/compiler-design-overview.md` > `Frontend stages > Stage 2: header syntax and interface binding > Project config compilation service`.

Allowed source includes:

- one required open `project` const record
- private top-level helper constants declared before their uses
- top-level builder and tooling section records
- scalar and optional constants
- anonymous const records
- collections of supported folded values
- foldable templates represented by their folded string result

Rejected source includes:

- every source dependency clause, including relative, project, Core, Builder, dependency and binding-backed clauses
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
    version #Config of String = "0.1.0",
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

### Direct project `#Config` fields

A direct primitive or optional field of `project` may declare a build-config contract.

Accepted build-configuration value types are:

- `String`
- `Int`
- `Float`
- `Bool`
- `Char`
- optional forms of those types

Nested project fields cannot declare `#Config`. Nested project fields do not provide unqualified source input values.

`#Config` is a compiler-owned qualifier on a compile-time declaration. It is not a type constructor,
source dependency clause or wrapper type. The semantic type of `value #Config of T` is `T`.

A direct project `#Config` value resolves in this order:

1. explicit CLI or programmatic build input
2. builder-provided primitive global
3. the folded declaration default
4. a structured missing-input diagnostic

Resolution happens during config compilation before Stage 0 applies fields such as `entry_root`.

Project defaults may use the ordinary allowed single-file config constant surface. Their final folded value becomes part of the project-wide contract.

A fixed direct project field is not a `#Config` contract. When a same-name source `#Config` uses the same primitive type and optionality, the fixed field is its authoritative provider and blocks CLI override. Same-name source declarations must still agree with each other on required or default state and on the normalised default value.

### Command build-input typing

The command accepts repeated `--input name=value` arguments. The command parser types each value
immediately, before any project or source contract is discovered:

1. Split at the first `=` and preserve every later `=` in the value.
2. Validate the input name as lower_snake_case.
3. Infer exact lowercase `true` and `false` as `Bool`.
4. Infer a complete valid signed Moth whole-number literal as `Int`.
5. Infer a complete valid Moth decimal-point or exponent literal as `Float`.
6. Infer a complete valid single-quoted Moth character literal as `Char`.
7. Infer a complete valid double-quoted Moth string literal as `String`.
8. Infer every other value, including empty text after `name=`, as `String`.

If a value starts with a quote, it must be a complete valid quoted literal. A malformed quoted
literal is a command-input diagnostic, not a String fallback. The shared `numeric_text` grammar and
materialisation helpers own numeric validation; whole-number overflow and invalid or non-finite
Float materialisation are rejected.

Bare `none` is String text. Optional absence comes from omission and contract/default resolution. A
concrete `T` input may satisfy a matching `T?` contract as a present value. No other coercion occurs;
in particular, `Int` does not satisfy `Float`.

Use explicit quotes to force an ambiguous String:

```bash
moth build . --input analytics=true
moth build . --input retries=4
moth build . --input ratio=0.75
moth build . --input api_url=https://example.com
moth build . --input 'label="true"'
moth build . --input "separator=':'"
```

Programmatic command APIs construct the same typed carrier and do not define another conversion
policy.

### `ProjectGlobalsInterface` and `@project`

The folded `project` record produces a specialised immutable `ProjectGlobalsInterface` under the permanently reserved `@project` dependency root.

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

Normal project modules and project-owned support packages may explicitly declare a dependency on `@project`. It is never implicitly injected.

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

### Source `#Config` contracts

Source `#Config` is intentionally narrow so every project-wide contract can be validated before module AST compilation.

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
- another resolved build configuration value

This restriction is deliberate. Stage 0 does not run a second general constant evaluator before AST.

Header syntax preparation normalises each source contract into a small build-input shape:

```rust
pub struct SourceBuildConfigContract {
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
2. a resolved direct project `#Config` field
3. explicit CLI or programmatic input for a source-only contract
4. a builder-provided primitive global
5. the shared source default
6. a missing-input diagnostic

A direct project `#Config` contract and every same-name source contract must still agree before the resolved project value is supplied to source modules.

Unknown explicit inputs are diagnosed only after every selected source contract is known.

The resolved value enters module AST as an ordinary folded constant. It creates no runtime wrapper,
dependency symbol category, HIR node or new visibility rule.

### Static Bool executable specialisation

`#Config of Bool` uses ordinary `if`; no `#Config if` syntax exists. Both branches complete Stage 4
frontend validation before a known Bool selects the executable branch. The selected branch keeps its
lexical scope. Only active executable work reaches HIR and downstream generated-function, borrow,
lifetime, link, target and backend systems. Static selection changes executable facts, not source
structure or graph topology. See the compiler authority for the exact Stage 4 ownership contract.

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

Builder and tooling section fields cannot declare `#Config`. They consume already folded project values and use backend-neutral folded values rather than builder-specific nominal types. This is a permanent ownership boundary.

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

Dependency clauses, aliases, helper constants, support types and source `#Config` declarations live outside the block in the normal root file.

The block uses the root file's ordinary compile-time visibility. It may reference:

- dependency-bound constants
- `@project`
- same-file constants declared before the block
- resolved source `#Config` constants
- foldable local const-record types
- selected-builder compile-time values available through normal module dependency clauses

Same-file forward references remain invalid.

Header syntax records its local dependencies. AST folds it through the ordinary module semantic path.

The block creates no ordinary module symbol, HIR or project-global value.

It cannot contain `project` or change project-level builder behaviour.

It may contain active artefact-builder and tooling-overlay sections.

Active entry sections are schema-validated. Inactive sections are parsed and folded but not schema-validated.

The block is optional. Its active artefact-builder section is also optional so tooling-only metadata remains possible.

Every normal module selected into the current command's semantic graph has its block validated whether or not an entry activates it. Modules reached through dependency clauses never apply their entry metadata to the declaring file.

Only active artefact-builder settings contribute entry activity.

### Fixed bootstrap order

The command and bootstrap flow is:

```text
select command, artefact builder, build profile and tooling overlays
-> parse explicit command inputs into typed primitive values
-> construct compiler and builder bootstrap capability surface
-> compile and validate config
-> resolve direct project #Config values
-> derive entry_root and @project
-> build the canonical source index and provider graphs
-> collect and resolve selected source #Config contracts
-> compile dependency-ordered waves
   -> bind provider interfaces
   -> order local declarations
   -> run AST semantics
      -> validate both ordinary-if branches
      -> specialise known Bool control flow
      -> commit active generated requests and executable summaries
   -> lower and validate HIR
   -> borrow-validate
   -> produce local lifetime constraints, lifetime facts and exported summaries
-> publish the module's completed generated delta
-> assemble a success-only ProjectCompilation
-> plan entry/package roots and exact reachable unions
-> instantiate lifetime summaries with builder lifecycle roots
-> validate complete lifetime topology
-> complete intervals, frontiers and epochs
-> produce backend-neutral memory requirements
-> target-affinity analysis and partition
-> target validation
-> create candidate physical-variant scopes
-> target/profile-specific family/layout refinement
-> revalidate affected refined family-edge facts
-> memory-strategy planning
-> ValidatedMemoryPlan
-> backend lowering
-> collector-free artefact verification when required
```

`check` performs every step through `ValidatedMemoryPlan` and stops before lowering.

Config compilation tokenizes and parses one self-contained `config.moth`, orders config declarations,
resolves direct project `#Config` sources while AST folds config, and validates the completed project
record and active project sections. Inactive config sections are folded during config compilation even
though their schemas are not active. Project config creates no source dependency graph. Source `#Config`
resolution happens only after the selected graph exists; it does not affect graph construction.

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
- every owned `.moth` file reachable through source dependency clauses
- every reachable builder-supported source asset such as `.mtf` or `.md`
- any other source-kind input explicitly defined as semantic by the selected builder

A builder-supported source asset becomes reachable through either route: a top-level dependency
clause, or a structural content-file reference in expression position. The two routes produce the
same membership and the same single preparation of that file.

Only the semantic source set contributes declarations, HIR, the public interface and module link facts.

Provider-backed explicit-extension files are owned through their provider contract. They produce binding-backed interfaces and runtime facts rather than ordinary module declarations.

### Check source set

`check` also examines owned `.moth` files that are not in the canonical semantic source set.

Each orphan becomes a check-only source unit under its nearest module namespace. It may be parsed, bound and semantically diagnosed with the same provider interfaces and visibility rules, but it does not silently add declarations to the canonical module artefact or public interface.

A check-only unit cannot become a backend root or link input.

This distinction lets tooling diagnose abandoned or disconnected source without changing dependency semantics.

## Prepared-source orchestration

Stage 0 asks the compiler to perform tokenization and header syntax preparation once for each selected source candidate.

Prepared syntax may contain:

- tokens or source-kind payloads
- declaration shells
- dependency clause shells
- structural provider references
- structural file references
- local declaration-ordering hints
- source `#Config` contract shells
- dormant root activity shells
- compile-time fragment placement metadata
- diagnostics and warnings
- deterministic string-table deltas or remap information

Stage 0 consumes structural provider references to finalise graphs. It does not bind source symbols itself.

Stage 0 also consumes structural file references. Each physical-target-bearing structural file
reference gets one filesystem resolution. `SiteRoot`, extensionless and
`SourceKindNoFileValue` facts carry no physical target and never enter this resolution path. A
content-source reference brings that `.mtf` or `.md` into the appropriate semantic source set
through its normal source-kind adapter, before the consuming module reaches AST. A resource
reference becomes a build input and watch interest, validated against the same ownership rules
the resource model uses. Stage 0 may register an unhashed byte source; it does not read contents,
hash an unused file, choose an output path, render a URL or decide whether the resource reaches an
output. The resolved outcome is published so AST interprets it without probing the filesystem
again.

What Stage 0 creates for a resource is a build input, not a semantic identity. It holds the
canonical physical source, its owning root, the validated logical target and the watch interest. The
stable semantic origin is created later, by AST, and associated with that physical source only when
the module publishes successfully. So a diagnosed module contributes no origin association, and a
watch interest recorded for a missing target carries no manufactured resource identity. One
canonical file may back several distinct logical origins, and equal origins must agree on their
byte-source facts.

Every authored physical-target-bearing file-value path is graph-active, independently of AST
reachability, constant folding and static branch specialisation. `SourceKindNoFileValue` is
retained only as a structural diagnostic fact with no physical target, semantic-source membership,
physical-source or watch record. Graph membership is never decided by output reachability, and `#Config`
cannot alter file dependency topology.

Because a newly discovered content source may itself contain file references, discovery is a
monotone worklist rather than a single pass. Stage 0 seeds the set from the module root and its
dependency-clause sources, prepares each not-yet-prepared source once, consumes that source's
structural file references, adds any newly discovered `.mtf` or `.md` to the module's semantic
source set, and repeats until no source is added. Header aggregation and local declaration ordering
run afterwards, on the settled set.

The loop's rules are what make it deterministic:

- membership deduplicates by canonical source identity, never by authored spelling
- every authored reference location is retained separately for diagnostics
- each physical content source is prepared exactly once however many references reach it
- a repeated reference is not a cycle; a real dependency cycle through synthetic `content`
  declarations is diagnosed later by local declaration ordering, not by discovery
- resource files found in newly added sources enter the same build-input registry as any other
- ordering and diagnostics do not depend on the order in which the worklist happened to insert

Retained source `#Config` contract shells do not create structural provider edges, dependency symbol
bindings or topology changes. Their later resolved values are consumed only by ordinary module AST
semantics.

When a provider interface is available, the compiler's interface-binding phase resolves retained dependency clauses into stable dependency symbol bindings and final visibility. Binding does not reparse source.

The three classes remain distinct:

- structural provider references for Stage 0
- structural file references for Stage 0 graph and input resolution
- dependency symbol bindings for compiler visibility and AST
- local declaration-ordering edges for compiler Stage 3

Stage 0 never implements a competing dependency grammar or lightweight scanner that later reparses the same syntax surface.

Provider-backed discovery remains serial while it mutates shared package identities, provider caches, resolution tables or diagnostic identity. Parallel provider discovery requires deterministic provider deltas and remapping first.

Stage 0 produces structure, resolved build-input contracts and compiler inputs. It does not type-check executable bodies, generate HIR or perform borrow validation.

Provider-independent source preparation is Stage 0's only reach into the compiler before a module is ready. Stage 0 decides which source candidate to prepare, when to prepare it and how to schedule preparation work across threads. Tokenization and header-preparation semantics stay compiler-owned behind one preparation call, and the exception ends at prepared syntax: Stage 0 consumes structural provider and file references and does not parse expressions, bind source symbols, order declarations or enter AST, HIR or borrow stages.

## Project and package topology

Terminology is strict:

- A module is one directory-scoped compilation and visibility unit rooted by `@*.moth` or `+*.moth`.
- A package is a named reusable `@...` dependency root and future distribution unit.
- A binding is a typed bridge to an implementation outside Moth source.
- A prelude is implicit dependency policy rather than a package kind.
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

### Module-root-relative dependency clauses

Source dependency clauses resolve from the declaring file's owning module root rather than the file's physical directory.

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
@accounts Account
```

This resolves to `src/accounts.moth`. It does not search beside `renderer.moth`.

Rules:

- `@./...` has no supported meaning.
- Parent components are invalid.
- Paths may traverse ordinary unrooted directories owned by the same module.
- Reaching a child normal module or support package ends filesystem traversal and exposes only its facade.
- Paths such as `@child/internal` cannot bypass a facade.
- Scoped support packages are injected by package name.
- Provider clauses use an explicit owner and do not silently reintroduce file-relative lookup.
- One clause resolves one provider, module facade or package surface. Direct selections are flat
  binding names inside that resolved surface and never create independent provider edges.
- Compiler interface binding consumes the Stage 0 namespace and does not probe ordered fallback candidates.

A normal module may declare dependencies on:

- ordinary files it owns
- unrooted directories it owns
- direct child normal modules
- support packages visible in its lexical scope
- registered Core, Builder and dependency packages
- provider files explicitly permitted by the active builder

A normal module may not declare dependencies on:

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

`@markdown` is visible to `site`, `pages` and `article`. Its private descendants may be depended on by the `markdown` facade, but consumers cannot address them through `@markdown/parser` or another implementation path.

For a support package `S` whose nearest ancestor normal module is `P`:

- `S` is visible to `P`.
- `S` is visible to normal sibling modules and their descendants.
- `S` is not visible above `P`.
- `S` is not visible outside `P`'s subtree.
- `S` is not depended on from its own private implementation descendants.
- Another support package in the same owner scope cannot depend on `S`.

The support facade may depend on:

- ordinary files it owns
- any descendant module in its private subtree
- support packages from a strictly outer scope
- registered packages

It may not depend on its parent, normal sibling consumers or same-scope support siblings.

Consumers see only the support facade's `export:` surface.

The same support-package name may appear in disjoint scopes. Overlapping scopes are rejected with diagnostics that point to both declarations and explain the overlap.

Direct normal-sibling dependency clauses remain disallowed. A future design may revisit them only with real project evidence, cycle diagnostics and a reason the shared behaviour cannot live in a scoped support package.

### Project package facade

The project-root `+*.moth` facade is a canonical API-only module compiled through the ordinary compiler pipeline with project-facade visibility supplied by Stage 0.

It may define and export its own legal API-only declarations.

Stage 0 gives the facade a special assembly namespace rooted at `entry_root`. Through that namespace it may reference the public interfaces of descendant modules below `entry_root`, regardless of ordinary lexical module visibility.

The facade:

- never bypasses an `export:` boundary
- is not visible to internal project modules
- cannot declare a dependency on `@project`
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

No dependency clause uses precedence, nearest-match shadowing or ordered fallback.

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

- dependency clause syntax
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

Build-configuration namespaces are scoped to one project or package compilation boundary. A consuming command's unqualified CLI or programmatic inputs do not implicitly satisfy a dependency's `#Config` contracts.

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

For each module job, Stage 0 calls one compiler-owned module compilation service and handles its outcome:

```text
ready module + completed provider interfaces
-> build one compiler input value
-> call the compiler module compilation service
-> Success / Diagnosed / CompilerError
-> deterministic string-identity remap and atomic publication
```

The compiler's own local semantic sequence inside that call is interface binding, local declaration ordering, AST semantics, public-interface projection, HIR lowering and validation, borrow validation, generated semantic completion and lifetime facts. That sequence is compiler-owned. Stage 0 never invokes its steps individually, constructs a public-interface draft, mutates HIR or reruns a compiler analysis. See `docs/compiler-design-overview.md` > `Compiler input and result boundary > Canonical module compilation service`.

Directory modules and synthetic single-file compilation use the same service after their own Stage 0 preparation path.

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
- generated functions completed while compiling those modules

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

## Generated-function boundary

The compiler owns generic template validation, call-site inference, request identity, generated HIR, generated borrow facts, and generated lifetime facts and summaries.

The build system owns:

- project-wide or package-wide request aggregation
- the published set every request is deduplicated against, lent as an immutable view
- boundary request availability: which providers have published, and therefore which module compiles next
- completed sidecar storage and transactional publication
- sidecar placement
- reuse across entries

Build-owned generated scheduling means boundary request availability, publication and reuse. It does not include request deduplication, generated HIR materialisation, HIR mutation, borrow rechecks or call-summary convergence. Those are compiler semantics completed inside one module compilation transaction, from an immutable view of already published generated identities and summaries that the build system supplies. The build system owns the set a request is measured against; the compiler decides that an already published identity needs no new materialisation.

Requests are keyed by stable generic declaration identity, canonical concrete type identities and required evidence identities.

The fixed point is reached inside one module's compiler transaction, not by build-side rescheduling. A request raised
while materialising another generated body is canonicalised and completed in the same transaction, so the boundary sees
one finished generated delta per module and never schedules a second pass to converge generated work. Module waves come
from the module dependency graph; generated requests do not add or reorder them.

Each successful generated sidecar entry carries its own generated-local type context, HIR, borrow facts, lifetime facts and summaries, link facts and fingerprints. It does not mutate a base module artefact.

Only requests committed from the Stage 4 specialised active AST reach the project or package
boundary. Generic calls in an inactive static branch are frontend-validated but do not cause
materialisation or generated sidecar work. The same compiler-owned provisional request boundary
applies to a compile-time-true assertion message: the message is validated, then its inactive
request delta is discarded before generated-function publication. Build orchestration does not
filter assertion requests after the AST stage.

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

Normal modules reached through dependency clauses expose public interfaces without executing root work.

Support modules and the project package facade never execute root work.

Entry assembly never triggers parsing, type checking, HIR generation, generic inference, borrow validation or lifetime-region validation.

The implicit `start` is non-exported, cannot be bound through a dependency clause and is infallible. The builder does not define a fallible start channel or an error-fragment policy.

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
- resource uses

Module-wide summaries may be cached as derived indexes. They are not the linking authority.

The build system does not repeatedly scan source, rebuild dependency clauses or reopen AST to discover runtime dependencies.

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

Modules without HTML artefact activity remain available to the graph but produce no route, runtime glue or emitted resources.

### Mixed-target planning and validation

Source contains no target-selection annotations and cannot query whether a function will become
JavaScript or Wasm. `#Config` cannot carry target or backend identity. Automatic partitioning and
capability rejection are builder/compiler services over platform-agnostic source; builder capability
surfaces expose stable semantics rather than physical target names.

The fixed sequence is:

```text
entry/package roots
-> exact reachable function/effect union
-> instantiate lifetime summaries with builder lifecycle roots
-> validate complete lifetime topology
-> complete intervals, frontiers and epochs
-> produce backend-neutral memory requirements
-> target-affinity analysis and partition
-> target validation
-> create candidate physical-variant scopes
-> target/profile-specific family/layout refinement
-> revalidate affected refined family-edge facts
-> memory-strategy planning
-> ValidatedMemoryPlan
-> backend lowering
-> collector-free artefact verification when required
```

Everything up to and including backend-neutral memory requirements is target-independent and shared. Everything from candidate physical-variant scope onwards is per target/profile variant.

`check` performs every step through `ValidatedMemoryPlan` and stops before lowering.

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

Partitioning is entry-specific. A physical variant is not complete, and cannot be deduplicated, until its memory plan exists. The required order is:

```text
partition
-> candidate physical variant
-> target validation
-> target/profile memory planning
-> memory-plan fingerprint
-> final physical-variant key
-> deduplication/reuse
```

Target partition first creates a candidate physical-variant scope. Memory planning then creates the final physical plan for that scope. Only after that may physical variants be deduplicated. Variants whose pre-plan layout identity matches are not reusable before their memory plans exist.

The final conceptual physical-variant key contains:

- module identity
- selected concrete function set
- target assignment
- build profile
- ABI identity
- layout identity
- runtime capability requirements
- relevant backend config fingerprint
- memory-plan fingerprint

The memory-plan fingerprint covers a stable normalised representation of:

- the post-refinement allocation-family graph
- the selected strategy per family
- region and group placement
- affine cleanup decisions
- hidden-destination physical requirements
- REC representation decisions
- cleanup plans
- destruction plans
- physical coalescing decisions

Donor-local or process-local indexes are never fingerprinted directly.

Entries with the same key reuse one variant. Different keys produce separate JavaScript companion or Wasm variants.

One source function may be JavaScript in one entry variant and Wasm in another.

Each selected module variant has a generated JavaScript companion facade. Wasm is emitted per selected module variant.

### Link planning and lifetime topology

Project and package link planning instantiates local lifetime summaries with builder lifecycle roots and validates the complete lifetime topology before target assignment. Linking does not reopen source or mutate HIR.

`ProjectCompilation` or the link plan conceptually carries project-level validated lifetime topology. That topology is shared and target-independent, and it does not carry one project-global physical memory plan. Exact Rust shape remains open.

Its handoff to physical planning is a set of backend-neutral memory requirements: allocation-family identity, validated lifetime owner, intervals, frontiers and epochs, retained-edge and retention-domain facts, retention cardinality, REC candidacy facts, group membership, affine transfer and cleanup candidates, hidden-destination constraints, lifecycle constraints and external-boundary constraints. They must not contain any selected physical strategy, host-GC representation, allocator choice, counter layout, arena layout or target-specific handle representation.

Builder-supplied page, mount, request, frame and arena roots are lifecycle inputs, not builder-specific source-law exceptions. Builder lifecycles cannot change language validity. Lifecycle-root instantiation is what lets reactive and mounted storage that outlives a lexical function still satisfy one lifetime owner and the retained-edge outlives rule.

Exported lifetime summaries participate in the public-interface fingerprint. They carry result provenance, retention, detached stored-result effects, cardinality, whole-domain kills, exit-specific success and error effects and frontier-enabling effects. They never carry donor-local allocation-family, region, counter or concrete-frontier indexes. Caller and link-level lifetime analysis derives concrete cleanup frontiers after combining these effects with local aliases, other retention domains, future edge creation and builder lifecycles. Topology-relevant implementation and link facts invalidate affected assemblies. Exact persistent encoding remains deferred.

External boundary profile and capability metadata belong on the builder surface conceptually so backends receive closed WIT-value or host-binding classifications rather than inventing retention graphs.

### Memory-strategy plans

After link-level topology validation succeeds, and after build-owned target partition and target validation have established a candidate physical variant, the compiler-owned memory planner selects one physical strategy per allocation family: stack or inline placement, static affine cleanup, inferred region allocation, explicit-group bulk reclamation, Retained Edge Counting or a host garbage-collected representation. The planner produces a `ValidatedMemoryPlan` containing allocation-family layouts, selected representations, affine cleanup decisions, region and group placement, cleanup and destruction plans, REC decisions and physical coalescing decisions.

Validated topology and backend-neutral memory requirements remain shared link facts. Each target/profile physical variant receives its own `ValidatedMemoryPlan` after partition and validation.

The build system owns target partition, physical-variant orchestration, the build profile and target/backend capability metadata, and invokes the planner once per candidate physical variant. The planner stays compiler-owned. Backends consume the finished plan and never select their own strategy or reconsider source legality. Imprecise planning retains conservatively; a missing physical strategy after successful topology validation is `CompilerError`.

### Backend capability metadata and collector-free verification

Each backend declares whether it supports collector-free release lowering. This is backend capability metadata consumed by the builder, not a source-visible or project-visible no-GC mode, and no `config.moth` field selects it.

- A backend advertising full memory control must lower every accepted topology in a release build without a tracing or reachability collector.
- Debug and development profiles may deliberately use a garbage-collected representation for simpler lowering, faster compilation and instrumentation.
- GC-native backends may use their host collector on any profile.
- Every profile and backend runs semantically equivalent mandatory borrow and lifetime-topology validation and accepts exactly the same source.

A project builder may verify after reachability and memory planning that a produced artefact contains no tracing runtime, and report that as an artefact property. Verification reports a fact about the emitted artefact; it never changes source legality or observable behaviour.

### Runtime and memory

Each page owns one runtime instance and one memory shared by its linked Moth Wasm variants.

Linked Moth Wasm variants import the page runtime rather than owning separate memories.

This one-page runtime/memory contract applies to linked Moth Wasm variants. It does not require imported WIT components to share page memory. Imported components own private runtime memory and cross the boundary only through closed value conversion profiles.

Project-level runtime bytes may be emitted once and instantiated separately for each page.

Wasm lowering consumes explicit selected-function, import, export, capability, layout, validated lifetime and `ValidatedMemoryPlan` inputs.

Full-control runtime memory support must eventually cover allocator, inferred region, explicit-group, REC counter and destruction-plan behaviour. The current basic linear-memory page and heap-base planning is migration debt, not the accepted end state.

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

### Resource emission

The HTML builder consumes the compiler's semantic resource facts. It does not rediscover files from
rendered text.

It decides:

- which reachable resource uses become emitted outputs
- the URL context for each reachable use
- deduplication of origins, physical sources, reads and warnings
- user-facing resource warnings

Resource bytes are returned as ordinary output records. The general rules live in
`Resource linking and output placement`.

## Resource linking and output placement

The compiler publishes semantic resource facts. The build system owns every physical decision made
from them.

### Graph activity versus emission

Stage 0 registers an input and watch interest for every physical-target-bearing authored file-value
path, before any reachability question. Registration may create an unhashed byte-source record. It
never forces a byte read.

Emission is decided later and exactly. A resource that no reachable output uses is never read,
hashed or emitted, even though it remains a known and watchable build input.

A site-root piece is not a resource. It has no origin, no byte source, no filesystem target, no
watch interest and no union membership. Builders render it from the selected artefact's
project-origin policy, and never prepend that origin to an ordinary resource URL.

Having no `ResourceId` is exactly why the site root needs its own owner. It cannot ride the resource
union, so three decisions are explicit:

- A builder must supply a site-root policy or reject a reachable site-root use during target
  contract validation. A backend never guesses `/`. A target with no meaningful site root is a
  legitimate rejection, not a reason to invent a default.
- A site-root piece inside a dependency's exported constant renders from the final consuming
  artefact's project-origin policy, not from the dependency package's own config or build origin.
  Public folded values carry these pieces across package boundaries, so the consuming build decides
  their text.
- Selected functions and metadata record their own site-root use, collected by the same structural
  walk that collects resource uses. That fact drives target capability validation, physical variant
  specialisation and rerendering of static fragments. Changing the project origin invalidates the
  affected physical and output variants; it does not invalidate public semantic interfaces or
  recompile semantic consumers.

### Boundary-wide source registry

One registry spans project, source-package and provider results for a compilation boundary.

- one semantic origin record per stable origin
- one physical byte-source record per canonical file or generated payload
- several origins may reference one byte source
- equal stable origins must agree on their byte-source facts
- canonical source paths are build IO facts only
- content hashes are output invalidation facts, never semantic identity
- module source deltas merge transactionally, and only for publishable module results

Preparation and semantic validation neither read nor hash resource bytes. Conflict validation
completes before any resource byte is read.

### Exact resource unions

Entry planning unions resource uses from the selected `start`, reachable source and generated
functions, runtime fragments, compile-time fragments, selected entry settings and reachable provider
runtime requirements.

Package planning unions externally selected exports, resource-bearing exported folded values,
reachable source and generated implementations and provider runtime requirements permitted by the
package target.

Unions are computed from per-function link facts and metadata owners without rescanning HIR, and an
unused private resource contributes nothing.

### Output placement

- project-local resources preserve their path relative to `entry_root`
- source, Core, Builder and dependency package resources use one injective package output prefix followed by their package-relative path
- provider-managed resources use the provider's declared stable output path
- generated provider resources use their declared path and generated bytes

The package-prefix encoder is one build-system owner. It must be injective over the stable package
origin, canonical package name and any future package-instance identity. Consumer aliases do not
change output identity.

### URL contexts

The URL context is the artefact whose URL resolution rules observe the emitted string. It is not
automatically the JavaScript or Wasm file that contains the generated code.

- ordinary page HTML uses the page document
- inline CSS uses the page document
- standalone CSS uses the stylesheet
- page runtime code uses the active page document unless the builder defines a different sink
- another builder supplies its own explicit context policy

A builder that cannot assign a context to a reachable resource use rejects it before lowering.

URL rendering picks the validated resource output path, computes a lexical relative path from the
context artefact's parent, uses `/` separators, percent-encodes each UTF-8 segment, prefixes
same-or-descendant paths with `./`, retains parent-relative `../` prefixes, and never prepends a
project HTML origin.

A source or generated function that constructs a resource-bearing runtime String may lower
differently for different entry URL contexts. The relevant normalised URL map or its fingerprint
participates in physical variant identity. It never enters source legality, canonical HIR identity,
public interfaces or semantic module identity.

### Conflicts

- one origin used by many entries emits once when output placement is identical
- the same output path and the same origin deduplicate
- the same output path and different origins fail with both useful locations
- resource output conflicts with HTML, CSS, JavaScript, Wasm, manifest and provider output
- unchanged provider use and ordinary resource use deduplicate only when origin and output path agree
- transformed or generated provider output has distinct identity
- all output paths and conflicts validate before hashing, metadata reads or byte reads
- warnings such as large-resource warnings are emitted once per reachable physical source
- conflict diagnostics use semantic origins and authored use locations, never reconstructed strings

### Invalidation

- a stable origin change is a semantic change and follows the compiler's fingerprint owners
- a byte-only change invalidates the content fingerprint, re-emits affected outputs and may invalidate a provider transform cache, without recompiling semantic consumers
- a route, output root or URL context change replans URLs and outputs and invalidates affected output or physical variant keys, without reopening source legality

Reachable file bytes are hashed once per build state, emitted bytes are read once per physical
source, and one read may feed several output records when distinct origins deliberately map to
distinct paths.

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

Config-value dependencies participate in the existing public-interface, implementation,
dormant-root, runtime-dependency and compatibility fingerprints. Static branch selection does not
create a separate fingerprint family. Changing a Bool configuration value may change active
implementation, effect, link or root facts, but it does not change source graph or declaration
identity. Dependency artefacts include their own configuration namespace and provenance.

Documentation-only changes regenerate documentation or editor indexes without invalidating semantic consumers or executable instances.

### Physical variant invalidation

Semantic public-interface invalidation and physical variant invalidation are separate concerns.

- A change to memory-plan inputs invalidates the affected physical variants without necessarily recompiling semantic consumers. Shared validated topology and backend-neutral memory requirements can survive a replanned variant.
- Two variants with different memory-plan fingerprints can never reuse one emitted physical variant, even when every pre-plan key component matches.
- A change to a provider's public-interface fingerprint still invalidates semantic dependants under the ordinary rules; that is independent of whether any memory plan changed.

Private dependency implementation may use the dependency's own `@project` only when no external package export reaches it. Its config dependence contributes to implementation and compatibility keys. Any exported declaration with direct or transitive dependence is rejected before package assembly.

### Persistent compatibility

Persistent serialisation is a later implementation of the same boundaries.

A serialised module, package or generated artefact is reusable only when compatible with:

- compiler semantic artefact format version
- relevant language semantics version
- stable project or package identity
- source fingerprint
- config fingerprint
- dependency public-interface fingerprints
- required Core capability-interface fingerprints
- required Builder capability-interface fingerprints
- target-independent frontend feature configuration
- embedded ABI or layout policy
- generated request identity where applicable

When physical variants themselves become persisted artefacts, their compatibility data must additionally include the memory-plan fingerprint or an equivalent normalised plan identity. A persisted variant is reusable only when that identity matches.

Process-local string IDs and absolute filesystem paths are not compatibility identities. Persistent artefacts store canonical logical identities and self-contained or remappable string data.

Incompatible artefacts are discarded and rebuilt.

Normal builds do not attempt best-effort deserialisation, partial migration or compatibility repair.

## Build-system implementation map

Current paths are navigation aids rather than permanent architecture.

- Project bootstrap, config schema and validation: `src/build_system/project_config.rs`, `src/build_system/project_config/validation.rs`, `src/projects/settings.rs`. Compiling `config.moth` to folded values is a compiler service under `src/compiler_frontend/single_source_compilation/`; this owner supplies the source and applies the result.
- Source indexing, graph construction and scheduling: `src/build_system/create_project_modules/`, `src/build_system/build.rs`. Stage 0 graph edges keep module identity, retained shell identity and diagnostic location rather than cloning file-owned paths.
- Module preparation and publication: `src/build_system/create_project_modules/module_preparation.rs` prepares source, `compilation.rs` schedules one `compile_module` call per ready module, and `module_artifact_store.rs` plus `generated_store.rs` publish what it returns.
- Builder capability surface: `src/builder_surface/`
- Commands and tooling overlays: `src/projects/cli.rs`, `src/projects/check.rs`, `src/projects/dev_server/`
- HTML project builder and entry assembly: `src/projects/html_project/`
- JavaScript and Wasm lowerers: `src/backends/js/`, `src/backends/wasm/`
- Output writing and manifests: build-system output and cleanup owners
- Tests, validation and roadmap: `tests/cases/`, `src/build_system/tests/`, `justfile`, `docs/roadmap/`

Compiler frontend, AST, HIR, borrow and target-validation locations are mapped in `docs/compiler-design-overview.md`. Nothing in this map sequences a semantic stage; `xtask/src/architecture_boundary.rs` enforces that.
