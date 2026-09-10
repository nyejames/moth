# Moth Build System Design

Moth's build system selects a command and capability surface, bootstraps project config, discovers source, constructs project and package graphs, schedules compiler work, plans linked artefacts and owns output writing.

This document is the single source of truth for accepted build-system, project graph, builder, tooling, link and output architecture. It describes the intended end state, including contracts that are not fully implemented yet. It is not an implementation-status report.

The typed direct and source `#Config` contracts, shared primitive command inputs and immutable
explicit `@project` interface are implemented. General non-template directives, `$config` prefixes,
closed `$project` / `$html_builder` configuration, root-only `$page` and explicit root purposes are
accepted queued design. Other accepted end-state contracts are labelled in their owning sections.

`docs/compiler-design-overview.md` is mandatory prerequisite reading. It owns semantic identities, public interfaces, compiler stages, module artefact contents, generated-function compilation, fingerprints and target-validation semantics. This document owns how projects and packages orchestrate those compiler contracts.

Companion authorities:

- `docs/compiler-design-overview.md` for core compiler architecture
- `docs/src/developer-docs/language/overview.mtf` and the canonical unsuffixed references it selects for source syntax and language semantics
- `docs/src/docs/directives/directives.mtf` for general directive syntax, ownership and argument rules
- `docs/src/docs/design-scope/` for design bias and scope boundaries
- `docs/src/developer-docs/memory-management/overview.mtf` for reference semantics, borrow validation, lifetime topology, retained-edge liveness, declared regions, affine ownership, Retained Edge Counting and backend memory lowering
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
| `config.moth`, project fields, `$config`, `@project`, builder directives, root metadata or bootstrap order | `Project bootstrap` and the exact relevant subsection | `docs/compiler-design-overview.md` > `Frontend stages > Stage 4: AST semantics > Constants, build configuration and const records` when compiler folding or handoff changes |
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
- Graph and input membership is conservative and comes from physical-target-bearing authored file references; output emission is exact and comes from entry or package reachability. Reachability never decides graph membership, and graph membership never forces emission. `SourceKindNoFileValue` and site-root `@/` have no physical target, source membership, input or watch record.
- Resource origin, byte source, emitted output path and rendered URL are four separate facts owned by the build system after the compiler has published semantic resource facts.
- Builders assign every reachable resource use one semantic URL context. No builder scans rendered HTML, CSS, Markdown or arbitrary strings to rediscover resources.
- Ordinary `$config` values and static `if` may specialise executable behaviour, but they cannot change source discovery, semantic source sets, dependency graphs, declaration or export existence, or package topology. Future `$feature` is a separate pre-graph structural-selection contract.
- Source semantics remain target and platform agnostic.
- Builders and backend capability metadata map stable source semantics to target-specific artefacts.
- Builders must not expose target identity through `$config`.
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

- owned compiler and builder directive signatures
- config-only contexts for `$project` and `$html_builder`
- source-backed Core and Builder packages
- binding-backed packages
- template directives
- external import providers
- builder runtime packages
- supported source file kinds
- builder-provided primitive build globals
- target-affinity and capability metadata

Frontend-owned directives and compiler-owned builtins are added to this surface. A builder cannot replace them. HTML remains the only implemented artefact builder for this work. Unknown or inactive unsupported directive forms are rejected rather than silently ignored.

Explicit command inputs are parsed into typed primitive values before config or source contracts are
matched. Programmatic command APIs construct the same typed carrier. Builder-provided primitive globals
must express stable semantic configuration; target or platform identity is not a valid builder-provided
`$config` global. Target intent remains build-system input and is not source-visible configuration.

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

That sequence is compiler-owned. The build system is a client of the config compilation service: it supplies the one authored source and consumes folded directive-invocation facts, explicit input-contract results, authored key locations and diagnostics. It does not compose the stages itself, and config bootstrap is not a second build-owned frontend pipeline. Application of validated project and HTML settings stays with the existing typed builder and config owners. See `docs/compiler-design-overview.md` > `Frontend stages > Stage 2: header syntax and interface binding > Project config compilation service`.

The compiler service returns the config source's live span builder with its outcome. Bootstrap
retains it through schema application and output validation on successful and diagnosed paths,
then installs the finished table once before sharing the source database. This handoff moves
source data and does not give build code access to the compiler's semantic stages. Config is one
known-source inventory lane: its identity is registered upfront and final before the service
tokenizes; only its table installation waits for the build-owned validation that can still
produce spans.

Allowed source includes:

- earlier private helper constants declared before their uses
- standalone `$config` contracts
- exactly one `$project` invocation
- exactly one `$html_builder` invocation for an authored HTML directory project
- scalar and optional constants
- anonymous const records used as ordinary helpers
- collections of supported folded values
- foldable templates represented by their folded string result

Rejected source includes:

- every source dependency clause, including relative, project, Core, Builder, dependency and binding-backed clauses
- authored file-value paths
- runtime declarations
- mutable bindings
- functions
- named support types
- traits and conformances
- standalone top-level templates
- page fragments
- `$page`
- `export:`
- nested config files or companion config sources
- inactive-builder sections retained for later use

Project config creates no source-visible declarations. Its folded outputs enter the project through specialised build-system interfaces only.

Short accepted shape:

```moth
$config
version #String = "0.1.0"

$project(
    name = "my_site",
    version = version,
    entry_root = "src",
)

$html_builder
```

A fixed literal version also works. Having a contract named `version` does not satisfy an omitted `$project` argument. `config.moth` does not select the builder. The command has already done so. Synthetic single-file operations without authored `config.moth` keep their explicit synthetic configuration path and do not invent `$project` / `$html_builder` source declarations.

### Project directive

`config.moth` contains exactly one `$project` invocation. It publishes validated project identity and settings through the existing config result and predefined `@project` interface. It does not create a local source symbol named `project`.

The positional parameter order is:

| Parameter | Type | Default | Configurable value permitted |
|---|---|---|---|
| `name` | `String` | Required | No. |
| `version` | `String` | Required | Yes. |
| `entry_root` | `String` | `"src"` | No. |
| `author` | `String?` | `none` | Yes. |
| `license` | `String?` | `none` | Yes. |
| `template_const_loop_iteration_limit` | `Int` | The existing compiler-owned default limit | No. |

`name` must be a valid Moth project identifier and is not inferred from the checkout directory. Directory-project `entry_root` remains a relative directory strictly beneath the project root. `version` has no declaration default; `"0.1.0"` belongs to the starter `$config` example, not to `$project`. Missing version is diagnosed even when an unused input contract named `version` exists.

The signature is closed. Arbitrary metadata fields, misspelled standard fields and `metadata = ...` are rejected. Ordinary helper records in config remain ordinary private constants. Project-wide reusable constants belong in ordinary source or support packages.

Retain existing field-domain and output/source-path validation. Project identity, source-discovery and compiler-control settings remain fixed-only: reject `$config` dependence in `$project`'s `name`, `entry_root` and `template_const_loop_iteration_limit` arguments, including dependence through earlier helper constants. An ordinary compile-time value without input dependence remains valid. A folded value equal to a literal is not evidence that it was fixed.

### Bootstrap `$config` declarations

Only explicit `$config` declarations create input contracts. `$config` is a prefix on one following explicitly typed top-level `#` binding. The binding name is the input name, the type is the contract and `=` is the fallback. Required non-optional inputs may omit the initializer. There is no `$config(default)` form and no type-position or value-position directive.

Accepted contract types are:

- `String`
- `Int`
- `Float`
- `Bool`
- `Char`
- optional forms of those types

`$config` is compiler-owned declaration metadata. It is not a type constructor, source dependency clause or wrapper type. The semantic type of a marked binding remains the declared `T`.

Project fields and ordinary helper constants never implicitly supply or block same-named inputs. Remove the fixed-project-field provider tier and its override-blocking semantics.

Bootstrap contracts resolve in this order:

1. explicit CLI or programmatic input for that name
2. a compatible builder-provided primitive global
3. the declaration's validated fallback or optional absence
4. a required-input diagnostic

Resolution of bootstrap contracts happens during config compilation before Stage 0 applies fields such as `entry_root`. Config fallbacks may use the existing allowed single-file compile-time surface and earlier visible helper values. They must finish as a supported primitive or optional primitive. Same-file forward references remain errors.

An explicit input without a bootstrap or selected-source contract is unknown, even if it matches a project metadata field.

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

The folded `$project` directive produces a specialised immutable `ProjectGlobalsInterface` under the permanently reserved `@project` dependency root.

The interface contains:

- stable field identities for the predefined project fields only
- folded backend-neutral values
- source locations
- field-level fingerprints
- project-context provenance
- no AST
- no HIR
- no runtime body

It is classified as project-local and Moth-source-backed for provenance and capability purposes, but it is not discovered as a normal source package.

`@project` exposes those predefined fields as namespace members. It does not expose another value named `project`. Config helper constants and `$config` input declarations are not implicitly public project metadata.

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

Project field dependencies are recorded at field granularity as semantic facts for future incremental
reuse. Current dev orchestration does not consume them for targeted invalidation; source locations and
resolution origins remain provenance, not semantic fingerprint inputs.

### Source contracts and static specialisation

Source `$config` is intentionally narrow so every project-wide contract can be validated before module AST compilation.

A source declaration may use only the accepted primitive or optional types listed for bootstrap contracts. Only a single explicitly typed top-level compile-time binding is eligible.

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

This restriction is deliberate. Source-contract preparation can normalise the literal without opening provider interfaces or evaluating module AST. Stage 0 does not run a second general constant evaluator before AST.

Header syntax preparation normalises each source contract into a small build-input shape:

```rust
pub struct SourceBuildConfigContract {
    pub name: BuildInputName,
    pub value_type: BuildInputType,
    pub required: bool,
    pub default: Option<PrimitiveBuildValue>,
    pub span: SourceSpan,
}
```

Exact names may change. `BuildInputType` is limited to the accepted primitive and optional domain. `PrimitiveBuildValue` stores a normalised literal value or `none`.

The barrier validates all contracts in the command's selected source graph before module AST compilation.

Matching declarations within one compilation boundary agree on name, primitive type, optionality, required or default state and normalised default value. Explicit overrides do not excuse conflicting defaults. Same-file duplicate declaration and no-shadowing rules still apply. Matching names in separate selected modules can refer to one input contract without merging their local declaration identities.

Selected-source contracts resolve in this order:

1. the already-resolved matching bootstrap `$config` contract, after compatibility validation
2. explicit input for a source-only contract
3. a compatible builder-provided primitive global
4. the common source fallback or optional absence
5. a required-input diagnostic

A project field named `version` does not satisfy a source input contract. `$project(version = "1.0")` creates project metadata, not a `version` input provider. Unknown explicit inputs are diagnosed only after every selected source contract is known. Each project or package boundary owns its own namespace; consumer input does not satisfy dependency contracts implicitly.

The resolved value enters module AST as an ordinary folded constant. It creates no runtime wrapper,
dependency symbol category, HIR node or new visibility rule.

`$config` of `Bool` uses ordinary `if`; no `$config if` syntax exists. Both branches complete Stage 4
frontend validation before a known Bool selects the executable branch. The selected branch keeps its
lexical scope. Only active executable work reaches HIR and downstream generated-function, borrow,
lifetime, link, target and backend systems. Static selection changes executable facts, not source
structure or graph topology. Ordinary static `if false` does not suppress frontend directive errors.
See the compiler authority for the exact Stage 4 ownership contract.

### `$html_builder` and builder settings

For an authored HTML directory project, `config.moth` contains exactly one `$html_builder` invocation. Bare `$html_builder` supplies all defaults. The command selects the builder first. This invocation never selects a backend, builder or runtime platform.

HTML settings remain a closed directive signature consumed by the existing typed HTML config owner. They use backend-neutral folded values rather than builder-specific nominal types. Unknown parameters are errors. Known unavailable directives error. Unknown directives error. Inactive-builder sections are not folded or ignored as a holding area. Multi-builder source and configuration await structural `$feature` selection.

These settings consume permitted folded input values and do not declare their own `$config` contracts. Config still rejects source dependencies and authored file-value paths. Tracked file resources belong in later root-local `$page` metadata.

Project and page signatures stay distinct. There is no generic cross-scope field inheritance or merge system. Builder output-path settings and defaults remain builder-owned.

### Root metadata and purpose directives

Root metadata and purpose directives are root-local builder facts. They are not an embedded `config.moth` source file.

`$page` is HTML-builder-owned normal-root metadata and an entry-purpose declaration. Placement rules:

- valid only at the top level of a normal module root, at most once
- valid on the explicitly selected source in synthetic single-file HTML mode, at most once
- invalid in ordinary non-root files, support roots, facades, `config.moth`, function bodies, ordinary branches, loops, declaration initializers, template heads and `export:` blocks

`$page` describes the whole root regardless of source position. It does not enable a mode for subsequent statements. Style places it early, after any earlier constants or dependencies needed by its arguments. Argument visibility remains ordinary file visibility. Same-file forward references are invalid even though the purpose applies to the whole root.

Header syntax retains the invocation and its ordinary dependency and reference facts. Module AST checks and folds it once through ordinary module visibility. Metadata is stored in that module's non-HIR lane. It creates no ordinary declaration, public export, runtime value or project global.

Typed metadata arguments may use earlier same-file constants, dependency-bound constants, explicit `@project` members, resolved source `$config` values and the existing foldable expression and template surface. Resource-bearing strings stay structural. Consumers never inherit or apply provider page metadata.

Authored root runtime or direct output requires an implemented purpose with a consumer. Only `$page` works initially. Comment and documentation semantics and API-only declarations are exempt. Future `$test` is documented but cannot satisfy validation until implemented. Unmarked executable candidates cannot silently vanish merely because entry filtering selects `$page`.

Every normal module selected into the current command's semantic graph has its root metadata and purpose validated whether or not an entry activates it. Modules reached through dependency clauses never apply their page metadata to the consumer.

### Fixed bootstrap order

The command and bootstrap flow is:

```text
select command, artefact builder, build profile and tooling overlays
-> parse explicit command inputs into typed primitives
-> construct the compiler and builder capability surface
-> compile config through the named compiler service
-> publish input-contract results and predefined @project fields separately
-> derive entry_root and construct the canonical source index and provider graphs
-> collect and resolve selected-source input contracts
-> compile dependency-ordered waves through the canonical module service
-> publish complete module results and their generated deltas
-> assemble a success-only ProjectCompilation
-> plan entry/package roots and exact reachable unions
-> perform link, target and memory planning
-> lower validated physical variants and emit owned outputs
```

Config remains one self-contained source with no runtime or dependency graph. Its compiler service resolves helpers, input contracts and directive arguments before build-owned settings application. Source contracts resolve only after graph discovery and before module AST semantics.

Future `$feature` selection is deferred. It must act after bootstrap facts are available and before excluded source publishes graph or declaration facts.

The canonical module service owns the local semantic sequence in `docs/compiler-design-overview.md`. `HTML project builder > Mixed-target planning and validation` owns the detailed link-to-output order for HTML. `check` performs the applicable planning and validation and stops before lowering and output emission.


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

The index inventories module roots. It does not scan arbitrary `.moth` files as pages. Root purpose selection consumes retained directive facts. Unmarked executable candidates cannot silently vanish.

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

These discovery rules apply to retained source. Ordinary static `if` excludes no source at this stage. Future `$feature` selection must exclude inactive regions before they publish provider, file-reference or declaration facts.

The compilation boundary keeps a `SourceDatabaseBuilder` beside immutable source lookup services.
Tokenization borrows a source's original live span builder; each `SourcePreparationDelta` returns
that builder outside its success or failure result. File workers and chunk merges return every
builder before fallible aggregation, including discarded speculative preparations. Repeated
canonical and check-only preparation appends to the same source-local table.

Source identity finalization and span-table finalization are separate boundaries. The live source
owner survives all canonical and check-only producers. Private AST lookup handles end before the
owner consumes its transient `Arc<SourceDatabase>` via unwrap into owned lookup storage and
installs each table once. Only then is the terminal publish `Arc` minted, and only then may
outcomes retain that source context for rendering. Package interfaces can publish
earlier; their source lookup context waits for the package's remaining check-only producers.

Prepared syntax may contain:

- tokens or source-kind payloads
- declaration shells
- dependency clause shells
- structural provider references
- structural file references
- local declaration-ordering hints
- retained `$config` input-contract shells
- retained root metadata and purpose directives
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
physical-source or watch record. Graph membership is never decided by output reachability, and `$config`
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

Retained source `$config` contract shells do not create structural provider edges, dependency symbol
bindings or topology changes. Their later resolved values are consumed only by ordinary module AST
semantics. Root purpose selection also consumes retained facts rather than rediscovering source.

When a provider interface is available, the compiler's interface-binding phase resolves retained dependency clauses into stable dependency symbol bindings and final visibility. Binding does not reparse source.

The four reference classes remain distinct:

- structural provider references for Stage 0
- structural file references for Stage 0 graph and input resolution
- dependency symbol bindings for compiler visibility and AST
- local declaration-ordering edges for compiler Stage 3

Stage 0 never implements a competing dependency grammar or lightweight scanner that later reparses the same syntax surface.

Provider-backed discovery remains serial while it mutates shared package identities, provider caches, resolution tables or diagnostic identity. Parallel provider discovery requires deterministic provider deltas and remapping first.

Stage 0 produces structure, resolved build-input contracts and compiler inputs. It does not type-check executable bodies, generate HIR or perform borrow validation.

Provider-independent source preparation is Stage 0's only reach into the compiler before a module is ready. Stage 0 decides which source candidate to prepare, when to prepare it and how to schedule preparation work across threads. Tokenization and header-preparation semantics stay compiler-owned behind one preparation call, and the exception ends at prepared syntax: Stage 0 consumes structural provider and file references and does not parse expressions, bind source symbols, order declarations or enter AST, HIR or borrow stages.

Direct-template discovery follows the same ownership split. A successful bundle moves its source
database and live span builders into the named compiler service, which finalizes the tables after
folding. A diagnosed discovery keeps the known source snapshots and finalizes at that terminal
preparation boundary instead. Request aggregation keeps earlier documents' warning source contexts
when a later document fails, without copying their warning vectors. Synthetic single-file traversal
and recursive direct-template discovery normalize private provisional identities once before
publishing success or diagnosis. See `docs/compiler-data-layout-design.md` > `Source identity and
database > Private discovery finalization`. Directory/package inventories retain upfront final IDs.

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

A normal module may own dormant top-level runtime work and page fragments. `$page` declares an HTML
artefact; the root file still defines module identity. Helpers remain non-entry files.

Support modules and the project package facade are API-only:

- no implicit `start`
- no top-level runtime statements
- no page fragments
- no route, builder artefact or page purpose
- ordinary runtime code inside functions remains valid

`export:` is the current public visibility marker. Accepted future direction replaces it with
`$export`; both forms will not remain permanent alternatives.

Every normal module in the command's semantic graph has dormant root work fully compiled, borrow-validated and locally lifetime-analysed. Entry assembly activates already compiled work only. Authored unpurposed root runtime or direct output is diagnosed rather than dropped from the inventory.

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

The facade package identity comes from the `$project` directive's `name` argument.

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

Build-configuration namespaces are scoped to one project or package compilation boundary. A consuming command's unqualified CLI or programmatic inputs do not implicitly satisfy a dependency's `$config` contracts.

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
-> Success / Diagnosed (plain CompilerDiagnostic values) / CompilerError
-> deterministic string-identity remap and atomic publication
```

Diagnosed outcomes carry the compact plain user-diagnostic boundary. Typed `CompilerError` values
remain the outer infrastructure/invariant failure lane and abort the owning project or package.
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

HTML application `build` requires a `$page` somewhere under `entry_root`. Single-file HTML output requires `$page` on the synthetic root. It performs target validation, backend lowering and output writing when compilation succeeds.

### `dev`

The first `dev` build compiles the complete graph required by its selected entries and package policy. HTML application `dev` has the same page-entry requirement as `build`.

Current dev rebuilds produce a fresh successful graph and recompute semantic fingerprint facts during
frontend compilation. Those facts are not retained after the build. Retention, comparison and
dependency-aware targeted in-memory reuse are deferred to `Incremental and persistent artefacts`,
alongside persistent compatibility work.

### `check`

`check` compiles:

- every discovered project module below `entry_root`
- check-only orphan source units
- the optional project package facade
- required Core and Builder source package graphs
- required dependency package surfaces
- reachable generated requests

Directory-project `check` permits a valid zero-page source tree. It still diagnoses unpurposed root runtime or direct output. A declaration-only root at `entry_root` is valid. Single-file `check` of a declaration-only synthetic root is valid; runtime or output work without purpose is invalid.

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

Tooling-only metadata never creates an artefact entry.

## Generated-function boundary

`docs/compiler-design-overview.md` > `Generated concrete functions` owns request identity, canonicalisation, deduplication, materialisation and semantic convergence.

The build system supplies an immutable view of published generated identities and summaries to each canonical module compilation. It owns boundary publication, completed sidecar storage, placement and reuse. The compiler returns the completed generated delta with the module transaction. Base module artefacts remain immutable.

Nested requests converge inside that transaction. They do not create extra module jobs or alter provider-wave ordering. Only requests from the specialised active AST are published, including the existing suppression of inactive assertion-message work. Build code neither filters those requests afterwards nor installs summaries or reruns HIR and borrow stages.

Cross-package instances belong to the consuming compilation. A diagnosed generated result exposes no partial sidecar and blocks dependent entry or package output. Internal failures abort the owning compilation boundary.


## Entry and package link planning

### Entry candidates and selection

`$page` declares HTML entries, including empty default roots and metadata-only pages. Runtime work or fragments alone are no longer the accepted trigger. A package facade is not an implicit HTML page.

Retained `$page` presence identifies an entry candidate before argument folding, including a descendant page not imported by the site root. The candidate becomes a page only after its ordinary module and metadata validation succeeds. Bare `$page` creates a page even with no runtime work or fragments. Use the existing normal-module root inventory, retaining unmarked executable roots for purpose diagnostics rather than scanning ordinary files as page entries.

One canonical normal module may produce several `EntryAssembly` values. The HTML builder initially produces at most one route entry per normal module.

### Entry assembly

An `EntryAssembly` selects one already compiled normal module and activates only that module's:

- compiler-synthesised `start`
- dormant top-level runtime work
- runtime page fragments
- compile-time page fragments
- resolved page metadata
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

1. selects the active normal module whose root declared `$page`
2. activates its already compiled dormant root work
3. merges compile-time fragments at their recorded runtime insertion indexes
4. creates runtime fragment slots
5. invokes active `start` once through the selected runtime path
6. hydrates runtime fragments in source order
7. assembles route HTML and companion artefacts from resolved metadata and existing directory routes

HIR carries runtime code only. Compile-time fragments and document structure live in compiler metadata and entry plans. Directory routes, fragment order and dependency dormancy are unchanged. Reachable `io.set_title` requires an advertised browser document-title capability and is rejected before lowering on unsupported hosts; there is no silent runtime no-op.

A module without `$page` produces no independent HTML entry. Its reachable declarations, runtime requirements and resources may still contribute to a consuming page or package assembly. Authored root runtime work or direct output without an implemented purpose is diagnosed.

### Mixed-target planning and validation

Source contains no target-selection annotations and cannot query whether a function will become
JavaScript or Wasm. `$config` cannot carry target or backend identity. Automatic partitioning and
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
-> ValidatedMemoryPlan validation
-> plan fingerprint and final variant key
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

Partitioning is entry-specific. Follow `Mixed-target planning and validation` through family/layout refinement, memory-plan validation and fingerprinting before constructing the final physical-variant key. Matching pre-plan layouts alone do not establish reuse. Deduplication applies only to completed validated variants.

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
- inferred-region and declared-region placement
- planned affine-responsibility transitions
- normalised retained-edge obligation transitions
- hidden-destination plans
- REC representation decisions
- cleanup plans
- destruction plans
- physical coalescing decisions

Donor-local or process-local indexes are never fingerprinted directly.

Physical coalescing is finalised before plan validation and fingerprinting, so a coalescing decision is part of the fingerprinted plan rather than a later lowering choice.

Entries with the same key reuse one variant. Different keys produce separate JavaScript companion or Wasm variants.

One source function may be JavaScript in one entry variant and Wasm in another.

Each selected module variant has a generated JavaScript companion facade. Wasm is emitted per selected module variant.

### Link planning and lifetime topology

Project and package link planning instantiates local lifetime summaries with builder lifecycle roots and validates the complete lifetime topology before target assignment. Linking does not reopen source or mutate HIR.

`ProjectCompilation` or the link plan conceptually carries project-level validated lifetime topology. That topology is shared and target-independent, and it does not carry one project-global physical memory plan. Exact Rust shape remains open.

The handoff is the compiler-owned backend-neutral memory requirements defined in `docs/compiler-design-overview.md` > `Lifetime-region and escape validation > Backend-neutral memory requirements`. These facts describe validated lifetimes and retention constraints, not selected physical strategies or concrete ownership/count transitions.

Builder-supplied page, mount, request, frame and arena roots are lifecycle inputs, not builder-specific source-law exceptions. Builder lifecycles cannot change language validity. Lifecycle-root instantiation is what lets reactive and mounted storage that outlives a lexical function still satisfy one lifetime owner and the retained-edge outlives rule.

Exported lifetime summaries participate in the public-interface fingerprint. They carry result provenance, retention, detached stored-result effects, cardinality, whole-domain kills, exit-specific success and error effects and frontier-enabling effects. They never carry donor-local allocation-family, region, counter or concrete-frontier indexes. Caller and link-level lifetime analysis derives concrete cleanup frontiers after combining these effects with local aliases, other retention domains, future edge creation and builder lifecycles. Topology-relevant implementation and link facts invalidate affected assemblies. Exact persistent encoding remains deferred.

External boundary profile and capability metadata belong on the builder surface conceptually so backends receive closed WIT-value or host-binding classifications rather than inventing retention graphs.

### Memory-strategy plans

After target-contract validation, the build system supplies the candidate physical variant, build profile, validated topology, backend-neutral requirements and capability metadata to the compiler-owned memory planner. The planner refines families and layouts, validates the result and returns one `ValidatedMemoryPlan` for that target/profile variant.

The build system orchestrates this call without selecting strategies or mutating semantic facts. Shared topology and backend-neutral requirements remain reusable across variants. Backends realise the returned plan rather than choosing ownership, cleanup or count transitions.

The complete plan contents, strategy outcomes and publication invariants are defined in `docs/src/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf` > `ValidatedMemoryPlan contract`. Compiler-stage ownership and conservative refinement fallback are defined in `docs/compiler-design-overview.md` > `Lifetime-region and escape validation`.

A plan is validated before its fingerprint enters the final physical-variant key. `HTML project builder > Physical variants` owns deduplication and reuse policy.

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

Wasm lowering consumes explicit selected-function, import, export, capability, layout, validated lifetime and `ValidatedMemoryPlan` inputs. It encodes the plan's normalised per-family retained-edge obligation transitions and planned discharge sites rather than deriving generic retain or release operations from borrow facts.

Full-control runtime memory support must eventually cover allocator, inferred region, declared-region, REC counter and destruction-plan behaviour. The current basic linear-memory page and heap-base planning is migration debt, not the accepted end state.

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
functions, runtime fragments, compile-time fragments, selected page metadata and reachable provider
runtime requirements. Authored metadata use locations and original spans stay with those uses.

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
- conflict diagnostics use semantic origins and authored use locations, never reconstructed strings

### Invalidation

- a stable origin change is a semantic change and follows the compiler's fingerprint owners
- a byte-only change invalidates the content fingerprint, re-emits affected outputs and may invalidate a provider transform cache, without recompiling semantic consumers
- a route, output root or URL context change replans URLs and outputs and invalidates affected output or physical variant keys, without reopening source legality

Reachable file bytes are hashed once per build state, emitted bytes are read once per physical
source, and one read may feed several output records when distinct origins deliberately map to
distinct paths.

## Output ownership

Artefact builders own output-path settings and defaults from their config-only directive contracts.

Builders that produce no artefacts register no output settings.

HTML defaults remain:

- development: `dev`
- release: `release`

The HTML builder accepts output-root overrides through `$html_builder`.

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
These are deferred build-system policies over compiler-produced semantic facts; computing a fingerprint
does not mean that the current dev server reuses an artefact.

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
