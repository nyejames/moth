# Path values and resource linking implementation plan

## Purpose

Replace eager rendered-path asset tracking with one typed compile-time `Path` and structural resource model.

The target is:

- explicit-extension path expressions create compile-time `Path` values
- every authored resource path is resolved and validated once
- stable resource identity contains no absolute path, output path or content hash
- resource anchors remain structural through AST, TIR, folded values, HIR and link facts
- only reachable resource uses are emitted
- builders choose output placement and containing-artefact-relative URLs
- provider-managed resources use the same conflict and output authority
- unused resources are not read or hashed eagerly
- no compiler or builder scans arbitrary strings to rediscover assets
- the old `RenderedPathUsage` and HTML-specific reconstruction lane are deleted

This plan does not own dependency-clause grammar. It consumes the accepted result of:

- `docs/roadmap/plans/dependency-clauses-and-path-syntax-plan.md`

It also requires the accepted TIR cleanup result before adding resource nodes.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/path-values-and-resource-linking-plan.md
PLAN_ADOPTION_BASELINE: bfaacd54227811f9e2b279d5a24e3df84dc381c2
STATUS: queued - blocked by three prerequisites
CURRENT_SLICE: Phase 0A - plan adoption and design correction
PREREQUISITES:
1. canonical-module-compilation-and-scoped-packages-plan.md Gate D
2. dependency-clauses-and-path-syntax-plan.md completion
3. tir-corrections-and-simplification-plan.md completion
BLOCKERS:
- final retained path syntax owner does not exist yet
- TIR correction owners are not accepted yet
NEXT_RESUME_ACTION: after prerequisites, refresh all owners and execute Phase 0B
DEFERRED_FOLLOW_UP: resource-only dev fast path and watch-state optimisation
```

Keep this block concise. Git history is the implementation record.

## Required authorities

- `AGENTS.md`
- `docs/language-overview.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- canonical language references under `docs/src/docs/`
- TIR architecture and accepted cleanup plan
- style, testing and validation guides
- progress matrix and roadmap
- predecessor dependency-clause plan

## Locked language contract

### Resource path expressions

An explicit-extension path in expression position produces one compile-time resource value.

```moth
logo #= @images/logo.svg
font #Path = @fonts/cmunss.woff2

icons #{Path} = {
    @icons/add.svg,
    @icons/remove.svg,
}
```

A resource path:

- has one explicit non-source extension on its final component
- resolves to an existing regular file
- resolves from the owning module root, not the physical source-file directory
- is not grouped
- is not a directory
- is not an external URL
- does not use `@/`, `@./`, parent components or `@@`

Recognized source kinds such as `.moth`, `.mtf` and `.md` are dependencies, not resource values.

Extensionless generated outputs such as `CNAME` remain builder-owned output features. A future explicit escape syntax is deferred.

### Builtin `Path`

`Path` is a reserved builtin compile-time type.

Allowed in V1:

- `#Path` and inferred Path constants
- transparent aliases to `Path`
- compile-time records with Path fields
- compile-time collections containing Path or supported Path-containing records
- compile-time field access and collection iteration
- exported folded Path constants
- exported public nominal const-record shapes with public compile-time-only fields
- direct template insertion
- compile-time template control flow

Rejected in V1:

- mutable Path bindings
- ordinary runtime bindings
- function parameters or returns containing Path
- runtime struct or choice instances containing Path
- receiver methods on Path-containing compile-time-only records
- options, choices, maps or generic applications containing Path
- map keys
- equality, ordering and operators
- casts
- implicit `Path -> String` assignment
- runtime filesystem access
- config values
- provider namespace values

### One availability classifier

Use one recursive classifier with a reason-bearing unsupported-shape result.

Conceptual form:

```rust
pub enum TypeAvailability {
    RuntimeStorable,
    CompileTimeOnly,
    NonValue,
}

pub enum CompileTimeShapeError {
    OptionContainsCompileTimeOnly,
    ChoiceContainsCompileTimeOnly,
    MapContainsCompileTimeOnly,
    GenericArgumentContainsCompileTimeOnly,
    RuntimeNominalContainsCompileTimeOnly,
}

pub fn classify_type_availability(
    type_id: TypeId,
    env: &TypeEnvironment,
) -> Result<TypeAvailability, CompileTimeShapeError>;
```

Rules:

- `Path` is `CompileTimeOnly`
- supported records and collections become compile-time-only transitively
- aliases preserve classification
- unsupported aggregate shapes fail through this one owner
- runtime receiving boundaries use the same classifier
- compile-time availability does not enter nominal identity
- no parallel Path-containing type hierarchy is introduced

### Resource-bearing strings

A template inserting a Path still has language type `String`, but its internal value remains structural until final output placement.

Rules:

- Path insertion creates a resource piece, not an eager URL string
- const folding may produce a builder-deferred resource string
- resource-bearing strings may be composed, stored in constants, exported, imported, passed and returned as ordinary String values
- resource pieces survive until physical entry/package lowering
- compile-time operations requiring final text reject unresolved placement
- runtime operations occur after entry-specific resource URL lowering
- output paths are never observable through a cast or compile-time formatter

### Moth templates and Markdown

`.mtf` remains declarationless and dependency-clause-free.

Resource use occurs through nested Moth template syntax inside the implicit Markdown body:

```moth
[$html:
    <img src="[@images/ownership.webp]" alt="Ownership graph">
]
```

Rules:

- plain `@images/ownership.webp` Markdown text remains ordinary text
- nested Path expressions resolve from the `.mtf` file's owning module root
- restricted same-directory root scope may expose exported Path constants and resource-bearing const templates
- `.mtf` gains no frontmatter or general declarations
- plain `.md` links and image targets remain literal and untracked
- no rendered Markdown or HTML scanning is allowed

### External and site-root URLs

These are ordinary untracked strings:

```moth
external #= "https://example.com/logo.svg"
cdn #= "//cdn.example.com/app.js"
site_root #= "/favicon.svg"
```

They are not checked, copied, rewritten, watched or included in resource unions.

## Resource ownership and identity

### Explicit resource owners

Do not duplicate package identity beside a module identity.

Use an explicit owner model equivalent to:

```rust
pub enum StableResourceOwnerId {
    Module(StableModuleOriginIdentity),
    Provider(StableProviderResourceOwnerId),
}

pub struct StableResourceOriginId {
    pub owner: StableResourceOwnerId,
    pub logical_path: PortableResourcePath,
}
```

Rules:

- module-owned resources derive package identity through `StableModuleOriginIdentity`
- provider-generated resources use a provider owner
- provider output reusing an unchanged module-owned source may deliberately use the same module-owned origin
- transformed or generated provider output gets provider-owned identity
- identity excludes absolute path, content hash, output path, alias, export binding, source location, route and builder prefix

### Filesystem ownership

A direct Path literal may address only a regular file inside the current module or package's private filesystem ownership.

- traversal starts at the owning module root
- ordinary unrooted directories owned by that module may be traversed
- child normal modules and support packages stop traversal
- another module's private resource cannot be addressed directly
- the project facade is not a global resource escape
- support visibility follows existing scoped-package rules
- cross-module resources travel only through exported Path values or resource-bearing exported templates
- no public visibility table exists for files themselves
- a physical resource root outside `entry_root` is deferred
- canonical containment rejects symlink escape
- strict case validation reuses existing source-path policy
- logical aliases to one canonical file remain distinct origins

## Dependency and emission liveness

### Dependency liveness

Every authored resource path is resolved and validated during the single preparation pipeline, including paths in:

- unused constants
- compile-time branches that later fold away
- unmaterialised generic templates
- helper templates that are never rendered

Preparation records:

- stable resource origin
- owning module or provider
- regular-file existence and containment
- canonical source path for build IO
- logical source path
- source location
- watch interest

Preparation does not read or hash unused resource bytes.

### Emission liveness

A resource is emitted only when a structural resource anchor reaches a selected output through:

- a compile-time page fragment
- dormant root runtime work
- a reachable source function
- a reachable generated function
- a reachable provider runtime requirement
- a selected package assembly

An unused Path constant creates no output.

Runtime branch liveness follows current syntactic CFG reachability. Do not add constant-condition tree shaking here.

## Data model

### Dense local tables

Conceptual compiler-local data:

```rust
pub struct ResourceId(u32);
pub struct ResourceUseId(u32);

pub struct ModuleResourceTable {
    records: Vec<ModuleResourceRecord>,
    by_origin: FxHashMap<StableResourceOriginId, ResourceId>,
}

pub struct ModuleResourceRecord {
    origin: StableResourceOriginId,
    source_location: SourceLocation,
}
```

Public interfaces carry stable origins. Local AST, TIR, HIR and sidecars use dense IDs.

### Build-owned source registry

```rust
pub enum ResourceContentState {
    Unhashed,
    Fingerprinted(ResourceContentFingerprint),
}

pub struct ResolvedResourceSource {
    pub origin: StableResourceOriginId,
    pub canonical_source_path: PathBuf,
    pub logical_source_path: PortableResourcePath,
    pub owner_root: PathBuf,
    pub content: ResourceContentState,
}
```

Rules:

- equal stable origins must agree on source facts
- preparation does not hash bytes
- conflict validation runs before output reads
- reachable opaque resources are hashed once per build state
- emitted bytes are read once per deduplicated source
- content hash is not part of public semantic identity
- byte-only changes are output invalidation, not semantic identity changes

### Successful resource state

The core resource plan must expose enough data for a later dev fast path.

```rust
pub struct ResourceBuildState {
    pub sources_by_origin: HashMap<StableResourceOriginId, ResourceSourceState>,
    pub origins_by_canonical_path: HashMap<PathBuf, Vec<StableResourceOriginId>>,
    pub outputs_by_origin: HashMap<StableResourceOriginId, ResourceOutputState>,
    pub emitted_uses: Vec<ResourceUseOutput>,
}
```

One canonical source may back several logical origins. The reverse path index updates all aliases without collapsing output ownership.

The watcher and resource-only update implementation are deferred to a separate follow-up plan.

## Output placement and URLs

The resource origin and emitted output path are separate facts.

Default placement:

- project-local resource: preserve path relative to `entry_root`
- source or dependency package resource: prefix canonical package output identity, then preserve package-relative path
- provider-managed resource: use the provider's declared stable output path
- generated provider resource: use its declared path and bytes

The package output-prefix encoder is one build-system owner and must be injective over:

- package origin
- canonical package name
- future version or package-instance identity where applicable

Consumer-local aliases do not change output identity.

URL rendering:

1. choose the containing output artefact
2. choose the validated resource output path
3. compute a lexical relative path from the containing artefact's parent
4. use `/` separators
5. percent-encode each UTF-8 segment
6. prefix same-or-descendant paths with `./`
7. retain parent-relative `../...`
8. never prepend HTML origin

Examples:

```text
resource: assets/logo.svg
container: index.html
URL: ./assets/logo.svg

resource: assets/logo.svg
container: docs/getting-started/index.html
URL: ../../assets/logo.svg

resource: styles/fonts/site.woff2
container: styles/site.css
URL: ./fonts/site.woff2
```

Inline CSS uses the HTML artefact as container. Standalone CSS uses its own output path. A builder without a containing-artefact policy rejects reachable resource anchors.

## Conflict rules

- one origin used by many entries emits once
- same output path and same origin deduplicate
- same output path and different origins fail with both locations
- resource conflicts with HTML, JS, Wasm, manifest or provider output fail
- ordinary Path and unchanged provider runtime use deduplicate only when origin and output path agree
- transformed or generated provider output is distinct identity
- validate every output path and conflict before reading bytes

## Template body syntax follow-up inside this plan

Replace mutable last-writer body modes with order-independent requirements:

```rust
pub enum TemplateBodySyntax {
    Template,
    Literal,
    Discard,
}
```

Rules:

- normal directives request `Template`
- `$literal` and `$code` request `Literal`
- `$note` and `$todo` request `Discard`
- repeated equal requests are idempotent
- incompatible requests diagnose both directive locations
- order does not change body tokenization
- `$raw` remains whitespace policy
- `$css` and `$css("inline")` use normal Template syntax
- `$literal` disables nested templates, expressions and resource anchors in its body
- `$css("raw")` is not added

## Data-oriented and performance requirements

- dense local resource IDs
- one source record per stable origin
- one path validation per authored path
- no `PathBuf` per resource use
- no URL string in semantic identity
- no full resource table clone per entry
- no scan over every compiled module when exact reachability exists
- no formatter access to resource internals
- no arbitrary HTML, CSS, Markdown or String scanning
- no output read before conflict validation
- one byte hash per reachable source per build state
- one output read per deduplicated source
- sorted contiguous final resource plans

Add counters for:

- authored resource paths
- resolved source records
- resource anchors
- reachable entry uses
- emitted unique resources
- deduplicated uses
- bytes hashed
- bytes read
- output conflicts

## Work protocol

Each code-bearing phase ends with:

```bash
cargo fmt --all
just validate
```

Run architecture audits whenever resource facts cross AST, TIR, public interfaces, HIR, link or output boundaries.

Phases 2A and 2B are internal worktree checkpoints. Do not merge them until Phase 2C deletes the old rendered-path lane.

Stop when:

- a second durable resource representation appears necessary
- identity would need an absolute path, output path or content hash
- formatter code needs filesystem or output details
- builder code needs to parse source or arbitrary text
- Path becomes runtime-storable to solve an implementation problem
- a route-specific URL would enter a public interface
- compatibility or fallback resource pipelines appear necessary
- an accepted TIR owner would be bypassed
- unused resources would be read or hashed eagerly
- a phase crosses more than two unlisted stage boundaries

## Phase 0 - activation and authority refresh

### Phase 0A - queued adoption

- add this plan and split index
- correct ownership and sequencing decisions
- do not update the roadmap in this slice
- do not change implementation

### Phase 0B - activate after prerequisites

- verify canonical Phase 5 Gate D
- verify dependency-clause plan completion
- verify TIR correction-plan acceptance
- refresh `main`, code owners, tests and benchmark baseline
- reconcile moved TIR and preparation owners
- update language, compiler and build authorities with final Path/resource semantics
- add or refresh the deferred progress-matrix row
- stop for activation review

## Phase 1 - resource identity, Path type foundation and early resolution

### Goal

Establish stable resource ownership and compile-time type legality before introducing typed Path AST or TIR nodes.

### Work

- add portable resource paths
- add `StableResourceOwnerId` and `StableResourceOriginId`
- add module-local `ResourceId` and table
- add build-owned resource source registry
- add builtin Path syntax, TypeId and canonical identity
- add one recursive availability classifier and error reasons
- reject Path at runtime receiving boundaries
- classify explicit-extension expression paths during preparation
- reject grouped resource expressions
- reject source extensions, missing extensions, directories and missing files
- resolve from owning module root
- enforce module/package boundaries and canonical containment
- retain watch interests for missing targets
- register generic-body resource references during declaration preparation
- keep content state `Unhashed`
- add production `ExpressionKind::Path(ResourceId)` only after Path type registration and availability checks exist

### Tests

- stable identity ignores source location and output path
- module owner identity does not duplicate package state
- provider-generated owner identity is representable
- moving declaration files inside one module preserves origin
- moving or renaming resource changes origin
- missing, directory, source-extension and extensionless diagnostics
- cross-module private resource diagnostic
- symlink escape and strict case diagnostics
- dead const branch still validates path
- unmaterialised generic body still validates path
- Path ordinary, mutable, parameter and return diagnostics
- unsupported option, choice, map and generic shapes
- unused resource bytes remain unread and unhashed

### Exit gate

- resource origins and source records are stable and path-free across semantic boundaries
- Path legality is owned before typed AST use
- preparation performs no output placement or byte hashing
- stop for review

## Phase 2 - structural resource vertical cutover

### Phase 2A - AST, TIR and folded values

- make direct template Path insertion emit `TemplateIrNodeKind::Resource`
- add opaque formatter resource anchors
- update accepted TIR walkers exhaustively
- preserve resources through slots, wrappers, branches, loops and subtree copy
- add `PublicFoldedValue::Path`
- add `PublicConstTemplatePiece::Resource`
- preserve resource-bearing strings without flattening
- add compile-time-content diagnostics for unresolved placement
- make resource nodes output-producing and non-reactive

### Phase 2B - runtime handoff, HIR and link facts

- add owned runtime resource nodes
- update immutable and mutable handoff walkers
- add structured HIR resource append or equivalent
- record resource uses per source and generated function
- retain dormant-root and fragment resource pieces
- keep URL strings out of HIR
- add containing-artefact mapping input to physical lowering
- include resource URL mapping in physical variant identity

### Phase 2C - builder placement and old-lane deletion

- add one `ResourceLinkPlan` owner
- build exact entry and package resource unions
- plan validated output paths
- compute containing-artefact-relative URLs
- return resource bytes as ordinary output records
- integrate central output validation, manifests and stale cleanup
- deduplicate origins and diagnose conflicts
- emit large-resource warning once per reachable origin
- delete `RenderedPathUsage`
- delete module-wide rendered-path metadata
- delete eager path formatting from template heads
- delete directory resource values and relative-to-file rendering
- delete HTML per-module tracked-asset reconstruction
- remove or repurpose `tracked_assets.rs`

### Required tests

- const and runtime template resource anchors
- slot, wrapper, branch and loop preservation
- non-reactive resource anchor
- resource-bearing string passed and returned
- const operation requiring final text is rejected
- TIR subtree copy preserves resource identity
- HIR link facts retain deterministic use order
- root and nested route URLs
- one asset used by several pages emits once
- unused validated resource emits nothing and remains unhashed
- stale resource output cleanup
- resource/output collision diagnostics

### Merge gate

Do not merge 2A or 2B to the main accepted baseline before 2C deletes the old lane and the full architecture audit is clean.

## Phase 3 - public Path values, aggregates and generics

### Work

- support allowed Path constants, records and collections
- project imported stable refs into consumer-local ResourceId values
- support exported Path constants
- support exported public compile-time record shapes
- support exported const records and collections
- preserve re-export origin beneath aliases
- retain resource-bearing const templates
- include stable resource refs in public-interface fingerprints
- keep content fingerprints outside interfaces
- freeze generic-body resource refs
- project them into generated sidecars
- keep Path illegal as a generic type argument
- emit generated resources only when materialised and reachable

### Tests

- exported font Path used in exported CSS template
- support package exports Path
- alias and re-export preserve origin
- private resource cannot be addressed directly
- public const record and collection
- two consumers share one output
- dependency package output prefix
- generic declaration validates resource before materialisation
- unreachable generated resource emits nothing
- interface agreement catches differing resource refs
- no absolute paths in interface payloads

## Phase 4 - `$literal` and composable CSS

- add frontend-owned `$literal`
- merge body syntax requirements order-independently
- make ordinary `$css` template syntax composable
- preserve opaque resource anchors through CSS validation
- keep `$code` literal and `$note`/`$todo` discarded
- keep `$raw` whitespace-only
- diagnose incompatible requirements at both locations
- update compiler and external highlighting

Tests cover resource Path in CSS, nested CSS templates, literal attribute selectors, grid line names, directive order independence and incompatible requirements.

## Phase 5 - provider-backed resources

### Work

- rename source-provider APIs coherently where import terminology no longer applies
- pass stable provider-source resource origins into provider requests
- include content fingerprint in provider semantic cache keys
- keep canonical source paths IO-only
- replace `PathBuf` runtime-asset semantic identity
- support stable provider runtime resource declarations
- support provider-generated bytes
- require stable output paths
- keep provider source un-emitted by default
- attach runtime resources to exact exported-symbol reachability
- deduplicate unchanged source Path/provider uses by shared origin
- keep transformed/generated output distinct
- publish transactionally

Tests cover grouped and namespace provider clauses, provider source not copied by default, explicit provider resource declarations, generated resources, unreachable symbol suppression and deterministic disagreement.

## Phase 6 - exact global resource planning and completion

### Work

- build one project-level source registry over project and package boundaries
- use injective package output-prefix encoding
- consolidate project, package and provider resource plans
- sort final plans by output path then stable origin
- validate conflicts before reading bytes
- hash reachable sources once
- read emitted bytes once
- build each containing-artefact URL map once
- include URL map in physical variant reuse keys
- deduplicate warnings and resource reads
- remove per-module planning fallback
- update benchmark workloads where needed

### Documentation and tooling

- update language, package, project, template, HTML and compiler docs
- add focused Path/resource reference pages
- update scaffolds and examples
- update progress matrix truthfully
- update external editor grammar
- rebuild generated docs

### Final audit

- one resource identity vocabulary
- one source registry
- no arbitrary string scanning
- no eager URL or output path in semantic facts
- no eager hash for unused resources
- no old rendered-path lane
- no per-entry full resource-table clone
- no provider `PathBuf` semantic identity
- no route-specific URL in public interface
- no duplicate package output prefix
- no compatibility resource pipeline

### Final validation

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-check
```

## Exit state

The plan is complete when:

- every authored resource path is validated once
- Path is compile-time-only and enforced transitively
- resource identity is stable and owner-explicit
- public interfaces contain stable resource refs only
- TIR and HIR retain structural resource anchors
- exact reachability drives entry/package resource unions
- builders own placement and URL rendering
- unused resources are not read or emitted
- provider resources use the same conflict authority
- all old rendered-string asset reconstruction is deleted

## Deliberately deferred

- resource-only dev/watch fast path
- broader HTTP caching, ranges and compression
- configured resource roots outside `entry_root`
- extensionless resource escape syntax
- managed plain Markdown resource links
- Path options, choices, maps and generic arguments
- runtime Path values or filesystem access
- cross-build resource caches and persistent serialization
