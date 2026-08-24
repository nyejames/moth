# Path values and resource linking implementation plan

## Purpose

Replace eager rendered-path asset tracking with one typed compile-time `Path` value and one structural resource-linking model.

The target is:

- explicit-extension path expressions create compile-time `Path` values
- every valid authored resource path is resolved and validated once through its normal semantic owner
- stable resource identity contains no absolute path, output path, route, URL or content hash
- resource anchors remain structural through AST, TIR, `ConstValueStore`, public folded values, HIR and link facts
- only resource uses that reach a selected entry or package output are emitted
- builders choose output placement and the URL context for each reachable use
- provider-managed and generated resources use the same identity, conflict and output authority
- unused resources are not read, hashed or emitted
- no compiler stage or builder scans arbitrary strings, rendered HTML, CSS or Markdown to rediscover assets
- the legacy `CompileTimePath`, `RenderedPathUsage` and HTML tracked-asset reconstruction lanes are deleted

This plan owns the Path value and resource model. It does not reopen dependency-clause grammar, module topology, package declaration syntax, TIR architecture or output ownership.

## Current state

```text
STATUS: queued - ready for activation
CURRENT_SLICE: Phase 0 - activation and authority publication
BLOCKERS: none
NEXT_ACTION: move this plan to active, execute Phase 0 and stop for activation review
```

Keep this block small. Record activation baselines and implementation checkpoints in working notes and Git history, not in this file.

## Starting assumptions

This plan starts only from these delivered capabilities:

- compiler-owned canonical module compilation with atomic success publication
- stable module, package, declaration and generated-function identities
- scoped module and support-package filesystem ownership
- one file-owned `PathSyntaxTable` with dense `PathSyntaxId` handles
- extensionless source dependency clauses and retained explicit-extension provider classification
- provider-independent source preparation that stops at retained syntax
- exact-view TIR with one AST-local store, checked copy/remap and one owned runtime handoff
- one module-local `ConstValueStore` as the folded-value authority
- one owned public folded-value vocabulary
- Stage 4 validation of both ordinary `if` branches followed by known-Bool specialisation before HIR
- generated-function materialisation that publishes completed sidecars transactionally
- validated HIR with per-function and per-block link facts
- build-owned exact entry and package reachability
- central output-path validation, manifests, skip-unchanged writes and stale cleanup

These are capability prerequisites, not references to the plans that introduced them.

## Required authorities

Read the current versions of:

- `AGENTS.md`
- `docs/roadmap/roadmap.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/compiler-data-layout-design.md`
- `docs/src/docs/codebase/language/overview.mtf`
- the canonical dependency-path, dependency-clause, constant, template, Moth-template, module, package and project-structure references selected by the language overview
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.md`
- `docs/src/docs/progress/@page.moth`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `index.md`

At activation, refresh exact implementation paths from `index.md` and the owning module entry points. File paths in this plan are migration hints, not architecture authority.

## Current migration surface

The activation review should confirm the current equivalents of these facts:

- `PathSyntaxTable` already owns authored path syntax
- dependency clauses already retain source-versus-provider target classification
- `ProjectPathResolver` still mixes dependency resolution with the legacy compile-time path model
- `CompileTimePath`, `CompileTimePathBase` and `CompileTimePathKind` still carry filesystem and eager rendering state
- `PathTypeKind` still treats files and directories as distinct path types
- exact `@/` still has legacy path-literal handling
- `ExpressionKind::Path` is test-only and carries the legacy boxed path shape
- `ConstValueStore` has no Path or resource-bearing string payload
- public folded strings and HIR constants are flat strings
- top-level const fragments and the direct `.mtf` service return flat strings
- TIR and the owned runtime handoff have no resource node
- HIR link facts have no resource-use family
- `RenderedPathUsage` still records module-wide eager rendered paths
- the HTML builder still reconstructs tracked assets from module metadata
- provider runtime assets still use canonical `PathBuf` values as semantic identity

If one of these facts has moved or already changed, update the implementation map and preserve the ownership rules below.

## Scope boundaries

This plan does not implement:

- project dependency declarations, version solving or package-manager policy
- anonymous const-record syntax that is not already supported
- runtime Path values or filesystem access
- Path options, choices, maps or generic type arguments
- managed plain Markdown links or images
- configured resource roots outside the existing owned project or package roots
- an extensionless resource escape syntax
- content-addressed filenames, image transforms or a general asset pipeline
- persistent cross-build resource caches
- the deferred resource-only dev rebuild fast path
- HTTP caching, ranges, compression or serving policy

Do not pull a later language or package surface forward to make this implementation easier.

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
- cannot be followed by dependency selections
- is not a directory
- is not an external URL
- does not use `@/`, `@./`, parent components or `@@`

Recognised source kinds such as `.moth`, `.mtf` and `.md` are dependencies, not resource values.

Context decides the meaning of an explicit non-source extension:

- in a dependency clause it names an already registered provider target
- in expression position it creates a `Path` value

The dependency classifier remains header-owned. Resource-expression classification remains AST-owned. Neither owner reinterprets the other path family.

Extensionless generated outputs such as `CNAME` remain builder-owned output features. A future explicit resource escape syntax is deferred.

### Builtin `Path`

`Path` is one reserved builtin compile-time type.

Allowed in V1:

- `#Path` and inferred Path constants
- transparent aliases to `Path`
- currently supported compile-time record shapes with Path fields
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

There is no file-versus-directory Path subtype. Resource expressions always resolve to regular files. Delete the legacy file/directory type distinction instead of adapting it.

### One availability classifier

Use one recursive semantic classifier with reason-bearing unsupported-shape results.

Conceptual shape:

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
    type_environment: &TypeEnvironment,
) -> Result<TypeAvailability, CompileTimeShapeError>;
```

Rules:

- `Path` is `CompileTimeOnly`
- supported records and collections become compile-time-only transitively
- transparent aliases preserve classification
- unsupported aggregate shapes fail through this owner
- every runtime receiving boundary consumes this classifier
- compile-time availability does not enter nominal type identity
- no Path-specific parallel type hierarchy is introduced
- `TypeId` remains the semantic type authority

Use current supported record and collection surfaces. Do not implement later anonymous-record syntax as part of Path availability.

### Resource-bearing strings

A template that inserts a Path still has language type `String`.

```moth
logo #Path = @assets/logo.svg

image #= [$html:
    <img src="[logo]" alt="Moth">
]
```

Internally, the value remains structural until a builder supplies output placement:

```text
Text("<img src=\"")
Resource(logo)
Text("\" alt=\"Moth\">")
```

Rules:

- Path insertion creates a resource piece, not an eager URL string
- plain text strings retain a compact plain-text representation
- resource-bearing strings use one ordered Text-or-Resource piece model
- `ConstValueStore` owns the module-local folded form
- public projection converts local resource IDs to stable resource origins
- HIR constants and runtime string construction retain local structural resource pieces
- top-level const fragments retain structural resource pieces
- resource-bearing strings may be composed, stored in constants, dependency-bound, passed and returned as ordinary String values
- resource-bearing strings do not create another source-visible string type
- compile-time operations that require final text reject unresolved resource placement
- runtime text operations occur only after the selected physical variant has lowered each resource piece to text
- output paths and URLs are never observable through a cast, formatter or public interface

Do not add separate `DeferredString`, `AssetString` or builder-specific string semantics. Boundary payloads may use local IDs or stable origins, but they express the same ordered piece model.

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
- the established restricted root scope may expose visible Path constants and resource-bearing const templates
- `.mtf` gains no frontmatter or general declarations
- plain `.md` links and image targets remain literal and untracked
- no rendered Markdown or HTML scanning is allowed

### Direct Moth-template compilation

The direct `.mtf` compiler service has no route or containing output artefact, so it cannot render final resource URLs.

It must:

- use the same TIR, `ConstValueStore` and resource resolution owners as integrated compilation
- return an owned structural folded string with its resource source facts
- permit text extraction only when the folded content contains no resource pieces
- diagnose a request for final text when placement is unresolved
- never accept route policy or render builder URLs inside the frontend

A project builder may consume the structural result and apply its normal resource link plan. Tooling that needs plain text gets a clear unsupported-content diagnostic rather than a guessed path.

### External and site-root URLs

These are ordinary untracked strings:

```moth
external #= "https://example.com/logo.svg"
cdn #= "//cdn.example.com/app.js"
site_root #= "/favicon.svg"
```

They are not checked, copied, rewritten, watched or included in resource unions.

### Static control-flow specialisation

Resource validity and resource output liveness are separate.

- both branches of an ordinary `if` complete normal frontend validation
- resource paths in a known-Bool inactive branch are still resolved and validated
- the inactive branch contributes no durable resource anchor, generated request, HIR or link fact
- runtime or unknown branches retain ordinary CFG reachability
- this plan adds no general constant-condition tree shaking beyond the established Stage 4 known-Bool specialisation

## Resource identity and ownership

### Stable semantic origin

Do not duplicate package identity beside a module identity.

Conceptual shape:

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

- module-owned resources derive package identity through the stable module origin
- provider-generated resources use a stable provider owner
- provider output that reuses an unchanged module-owned source may deliberately use the same module-owned origin
- transformed or generated provider output gets provider-owned identity
- identity excludes absolute paths, content hashes, output paths, aliases, export bindings, source locations, routes and builder prefixes
- moving a declaration between ordinary files in one module does not change resource origin
- moving or renaming the resource within its owner changes resource origin
- consumer-local aliases and re-exports do not change origin

### Three separate facts

Keep these concepts separate:

1. **Resource origin** is semantic identity.
2. **Resource use** is one authored or generated anchor with a source location and executable or metadata owner.
3. **Byte source** is the build-owned file or generated payload from which output bytes come.

One origin may have many uses. One canonical file may back several distinct logical origins. A resource use never owns a `PathBuf`.

Conceptual compiler-local shape:

```rust
pub struct ResourceId(u32);
pub struct ResourceUseId(u32);

pub struct ModuleResourceTable {
    origins: Vec<ModuleResourceOrigin>,
    uses: Vec<ModuleResourceUse>,
    by_origin: FxHashMap<StableResourceOriginId, ResourceId>,
}

pub struct ModuleResourceOrigin {
    pub origin: StableResourceOriginId,
    pub declaration_location: SourceLocation,
}

pub struct ModuleResourceUse {
    pub resource: ResourceId,
    pub location: SourceLocation,
    pub owner: ResourceUseOwner,
}
```

The exact Rust shape may change. These rules do not:

- local AST, TIR, `ConstValueStore`, HIR and module link facts use dense IDs
- the dense ID is always paired with its owning resource table
- public interfaces use stable origins
- generated sidecars use generated-local dense IDs or an explicitly paired shared table
- donor-local resource IDs never cross module or generated-sidecar boundaries
- use locations do not participate in semantic identity
- a repeated origin preserves every relevant use location without cloning origin data

### Build-owned byte sources

Conceptual build state:

```rust
pub struct ResourceSourceRegistry {
    sources: Vec<ResourceByteSourceRecord>,
    source_by_origin: HashMap<StableResourceOriginId, ResourceSourceId>,
    source_by_canonical_path: HashMap<PathBuf, ResourceSourceId>,
}

pub enum ResourceByteSource {
    File {
        canonical_source_path: PathBuf,
        logical_source_path: PortableResourcePath,
        owner_root: PathBuf,
        content: ResourceContentState,
    },
    Generated {
        provider_owner: StableProviderResourceOwnerId,
        content_fingerprint: ResourceContentFingerprint,
        bytes: Vec<u8>,
    },
}

pub enum ResourceContentState {
    Unhashed,
    Fingerprinted(ResourceContentFingerprint),
}
```

The exact storage shape may use ranges, arenas or shared byte buffers. Preserve these rules:

- one semantic origin record per stable origin
- one physical byte-source record per canonical file or generated payload
- several origins may reference one byte source
- equal stable origins must agree on their byte-source facts
- canonical source paths are build IO facts only
- content hashes are output invalidation facts, not semantic identity
- preparation and semantic validation do not read or hash resource bytes
- conflict validation completes before resource bytes are read
- reachable file bytes are hashed once per build state
- emitted file bytes are read once per physical source
- one read may feed several output records when distinct origins deliberately map to distinct paths

### Filesystem ownership

A direct Path literal may address only a regular file inside the current module or package's private filesystem ownership.

- traversal starts at the owning module root
- ordinary unrooted directories owned by that module may be traversed
- child normal modules and support packages stop traversal
- another module's private resource cannot be addressed directly
- the project facade is not a global resource escape
- support visibility follows existing scoped-package rules
- cross-module resources travel only through exported Path values, resource-bearing exported constants or reachable function implementation
- no public visibility table exists for files themselves
- a physical resource root outside the accepted owned roots is deferred
- canonical containment rejects symlink escape
- strict case validation reuses the established source-path policy
- logical aliases to one canonical file remain distinct origins

## Resolution and publication

### Syntax remains syntax-only

`PathSyntaxTable` continues to own authored path spelling and location only.

Do not add resource identity, filesystem resolution, output placement or content state to path syntax rows.

Dependency clauses retain their own path role through header syntax. AST expression parsing consumes all other path tokens in expression contexts. No stage infers resource expressions by subtracting dependency rows from a token scan.

### AST owns resource-expression classification

A path becomes a resource only when the ordinary AST expression parser consumes it in a valid expression position.

The AST owner:

- reads the exact `PathSyntaxId` through its file-owned table
- confirms one explicit non-source extension on the final component
- rejects source extensions, missing extensions and dependency selections
- resolves from the owning module root
- validates regular-file existence, case and canonical containment
- interns the stable origin into the module resource table
- records the authored use location
- returns `ExpressionKind::Path(ResourceId)`

Resolution is memoised by the exact retained source identity and path row or by the resulting stable origin. The same authored path is never resolved twice.

Do not add a provider-independent body scanner to source preparation. Header preparation still stops at retained syntax. Valid bodies, constants, templates and generic templates are parsed through their normal semantic owners.

### Complete source validation

Every valid authored resource expression is resolved even when it appears in:

- an unused private constant
- a helper template that is never rendered
- either branch of an ordinary `if`
- a branch later removed by known-Bool specialisation
- a generic template that is never materialised
- a function that is unreachable from the selected entry

Syntax-invalid code follows normal parser recovery and diagnostic ordering. It does not need speculative filesystem resolution.

### Module result boundary

Resource resolution produces two result families:

- compiler-owned semantic resource facts used by the module, generated delta and public interface
- build-facing byte-source and watch-interest facts used for IO and invalidation

Successful publication is atomic with the module artefact, public interface and generated delta. The build system merges the source delta into the boundary-wide registry only when the semantic result is publishable.

A diagnosed module exposes no partial HIR, public interface or semantic resource table. It may retain build-only watch interests for missing resource targets so creating the file can trigger a later rebuild. Those observations carry no public semantic value and cannot be used by another module.

The direct `.mtf` service returns the same owned source facts with its structural folded content.

## Structural value flow

### `ConstValueStore` is the folded-value authority

Extend the existing module-local folded-value graph. Do not create a resource-specific constant arena or restart recursive AST-expression interpretation.

The store needs equivalent support for:

- Path values
- plain text strings
- resource-bearing strings
- collections and supported records containing Path
- const templates with Resource pieces

Conceptual payload additions:

```rust
pub enum ConstValuePayload {
    // existing payloads
    Path(ResourceId),
    String(ConstStringValue),
}

pub enum ConstStringValue {
    Text(StringId),
    Pieces(Vec<ConstStringPiece>),
}

pub enum ConstStringPiece {
    Text(StringId),
    Resource(ResourceId),
}
```

Exact layout is benchmark-selectable. Ownership is not:

- `ConstValueStore` owns the local folded graph
- its postorder visitor is the only recursive conversion route
- public projection consumes the visitor
- HIR constant projection consumes the visitor
- direct `.mtf` extraction consumes the visitor
- no consumer reconstructs resource strings from AST expressions or TIR

Keep the plain text fast path compact. Do not force every ordinary string into a one-element vector.

### Public folded values

The owned cross-module vocabulary needs equivalent stable forms:

- `PublicFoldedValue::Path(StableResourceOriginId)`
- an owned structural String payload whose Resource pieces carry stable origins
- `PublicConstTemplatePiece::Resource(StableResourceOriginId)`

A resource-bearing String remains `PublicFoldedValue::String(...)`. Do not add a second public language-level string category.

Public projection must:

- preserve piece order
- preserve stable origins beneath aliases and re-exports
- exclude source paths, output paths and URLs
- include stable resource facts in public-interface validation and fingerprints
- reject unavailable private origins
- project dependency-bound stable origins into consumer-local Resource IDs

### TIR

Direct Path insertion emits `TemplateIrNodeKind::Resource(ResourceId)` or an equivalent dense local reference.

Every exact-view TIR owner must handle Resource nodes:

- construction
- summaries
- preparation
- folding
- formatting
- slot composition
- wrapper application
- branch and loop traversal
- subtree copy and identity remap
- const-template projection
- runtime handoff materialisation
- reactive metadata reduction

Rules:

- Resource nodes are output-producing
- Resource nodes are non-reactive
- formatters see opaque Resource anchors
- formatters cannot inspect filesystem or output details
- copy/remap preserves stable resource identity while allocating any required local ID
- a missing Resource case in an exhaustive TIR walk is an internal error

### AST-to-HIR runtime handoff

Add an owned Resource node to the neutral runtime-template handoff.

The immutable and mutable handoff walkers must cover it. HIR still receives no TIR IDs, store references, overlays or formatter state.

### HIR and module constants

HIR needs one backend-neutral structural operation such as ResourceAppend. The exact statement name may change.

It must support:

- runtime template construction
- ordinary resource-bearing String constants
- values passed through locals, calls and returns
- resource-bearing top-level runtime fragments
- generated functions
- deterministic source locations for target diagnostics

`HirConstValue::String` must retain the Text-or-Resource shape. Do not flatten module constants before physical variant planning.

HIR never contains:

- absolute source paths
- output paths
- route-relative URLs
- content hashes
- builder names

### Top-level const fragments

Compile-time page fragments are compiler metadata rather than HIR, but their content must use the same structural string model.

The fragment metadata retains:

- its ordered Text-or-Resource content
- its existing runtime insertion index
- resource use locations needed by entry planning

The HTML builder renders Resource pieces only after the entry route and output path are known.

## Validation and emission liveness

### Semantic validation

Every valid authored resource expression resolves and validates during module AST semantics.

This is independent from output selection.

### Executable liveness

A Resource anchor contributes executable link facts only when it survives Stage 4 specialisation and reaches HIR.

Per-block and per-function link facts record resource uses in deterministic source order. Exact entry reachability unions them without rescanning HIR.

### Metadata liveness

Compile-time fragments and selected package folded values are not HIR. Their resource uses travel with their existing metadata or public-interface owners.

Entry planning combines:

- the selected start function
- reachable source and generated functions
- runtime fragments
- compile-time fragments
- selected entry settings
- reachable provider runtime requirements

Package planning combines:

- externally selected exports
- resource-bearing exported folded values
- reachable source and generated implementations
- provider runtime requirements permitted by the package target

An unused private Path constant creates no output.

### Runtime branches

Known-Bool inactive branches produce no link facts. Runtime branches retain ordinary CFG liveness, so any branch reachable in the retained CFG contributes its possible resource uses.

Do not add runtime value profiling or path-sensitive execution prediction.

## Output placement and URL contexts

Resource origin, byte source, emitted output path and rendered URL are four separate facts.

### Default output placement

- project-local resources preserve their path relative to `entry_root`
- source, Core, Builder and dependency package resources use one injective package output prefix followed by their package-relative path
- provider-managed resources use the provider's declared stable output path
- generated provider resources use their declared path and generated bytes

The package-prefix encoder is one build-system owner. It must be injective over the stable package origin, canonical package name and any future package-instance identity available to the build.

Consumer aliases do not change output identity.

### URL context

The builder assigns one semantic URL context to every reachable resource use.

The URL context is the artefact whose URL resolution rules observe the emitted string. It is not automatically the JS or Wasm file that contains generated code.

Examples:

- ordinary page HTML uses the page document
- inline CSS uses the page document
- standalone CSS uses the stylesheet
- page runtime code uses the active page document unless the builder defines a different sink
- another builder supplies its own explicit context policy

A builder that cannot assign a context to a reachable resource use rejects it before lowering.

URL rendering:

1. choose the validated resource output path
2. choose the use's semantic URL context
3. compute a lexical relative path from the context artefact's parent
4. use `/` separators
5. percent-encode each UTF-8 segment
6. prefix same-or-descendant paths with `./`
7. retain parent-relative `../...`
8. never prepend a project HTML origin

Examples:

```text
resource: assets/logo.svg
context: index.html
URL: ./assets/logo.svg

resource: assets/logo.svg
context: docs/getting-started/index.html
URL: ../../assets/logo.svg

resource: styles/fonts/site.woff2
context: styles/site.css
URL: ./fonts/site.woff2
```

### Physical variants

A source or generated function that constructs a resource-bearing runtime String may lower differently for different entry URL contexts.

The relevant normalised resource URL map or its fingerprint participates in physical variant identity. It does not enter:

- source legality
- canonical HIR identity
- public interfaces
- semantic module identity

Compile-time fragment URL rendering participates in the containing output artefact plan, not an unrelated runtime code variant.

## Conflict rules

- one origin used by many entries emits once when output placement is identical
- the same output path and same origin deduplicate
- the same output path and different origins fail with both useful locations
- resource output conflicts with HTML, CSS, JS, Wasm, manifests or provider output
- unchanged provider use and ordinary Path use deduplicate only when origin and output path agree
- transformed or generated provider output has distinct identity
- all output paths and conflicts validate before hashing, metadata reads or byte reads
- warnings such as large-resource warnings are emitted once per reachable physical source
- conflict diagnostics use semantic origins and authored use locations, not reconstructed strings

## Fingerprints and invalidation

Resource invalidation follows existing fingerprint owners.

### Semantic changes

These affect semantic or public facts as appropriate:

- stable resource origin changes
- ordered Resource pieces in an exported folded value change
- a public Path constant changes origin
- a reachable function or fragment gains or loses a resource use
- provider resource effect metadata changes

### Byte-only changes

Changing file bytes without changing stable origin:

- does not change type identity
- does not change public semantic identity
- does not recompile semantic consumers
- invalidates the resource content fingerprint
- re-emits affected outputs
- may invalidate a provider transform cache when the provider consumes those bytes

### Placement-only changes

Changing a route, output root or containing URL context:

- does not change public interface identity
- replans URLs and outputs
- invalidates affected output or physical variant keys
- does not reopen source legality

Public-interface fingerprints include stable resource origins and ordered structural String pieces. Runtime-dependency fingerprints include exact reachable resource uses. Resource build state includes content and output fingerprints.

The resource-only dev fast path remains deferred, but the successful state must expose enough separation to implement it later without changing semantic identity.

## Template body syntax follow-up

Replace mutable last-writer body modes with order-independent requirements:

```rust
pub enum TemplateBodySyntax {
    Template,
    Literal,
    Discard,
}
```

Rules:

- ordinary directives request `Template`
- `$literal` and `$code` request `Literal`
- `$note` and `$todo` request `Discard`
- repeated equal requirements are idempotent
- incompatible requirements diagnose both directive locations
- directive order does not change body tokenisation
- `$raw` remains whitespace policy
- `$css` and `$css("inline")` use normal Template syntax
- `$literal` disables nested templates, expressions and resource anchors in its body
- `$css("raw")` is not added

This work remains inside the resource plan because composable CSS must preserve opaque Resource anchors without turning CSS validation into text scanning.

## Data-oriented and performance requirements

- dense local resource and use IDs
- one origin record per stable origin
- one byte-source record per canonical file or generated payload
- one semantic resolution per authored resource expression
- no `PathBuf` per resource use
- no URL string in semantic identity
- no full resource-table clone per entry
- no scan over every compiled module when exact reachability exists
- no formatter access to resource internals
- no arbitrary HTML, CSS, Markdown or String scanning
- no output read before global conflict validation
- one content hash per reachable physical source per build state
- one byte read per emitted physical source
- sorted contiguous final plans
- compact plain-string fast path
- resource piece storage measured before introducing broad generic containers or per-piece heap allocation

Add counters for:

- authored resource expressions
- resolved origins
- physical byte sources
- resource anchors
- resource-bearing strings
- reachable entry and package uses
- emitted unique resources
- deduplicated origins and byte sources
- bytes hashed
- bytes read
- output conflicts
- URL maps built
- resource table remaps or projections

Use existing benchmark and scaling infrastructure. Add a scaling series only when the implementation introduces a size-sensitive path whose complexity is not already protected.

## Work protocol

### Validation

A documentation-only activation slice ends with:

```bash
cargo run --quiet -- build docs --release
```

Each code-bearing slice ends with:

```bash
cargo fmt --all
just validate
```

Use focused tests during iteration. Report only commands actually run.

### Review

Every non-trivial slice ends with the repository Slice review.

Run a structured boundary audit:

- before the atomic core cutover merges
- after provider resources join the common model
- before final completion

The structured audit is read-only. Implement accepted findings in a separate correction slice.

### Atomic core cutover

Phase 1A through Phase 1C are internal worktree checkpoints. Do not merge them to the accepted main baseline until Phase 1C has:

- a complete source-to-output vertical path
- deleted the old eager rendered-path lane
- deleted legacy compile-time path identity
- passed the Phase 1D review and validation gate

Do not keep old and new production paths behind a flag, fallback or adapter.

### Stop conditions

Stop and return for design review when:

- a second durable resource semantic representation appears necessary
- identity would need an absolute path, output path, route, URL or content hash
- provider-independent preparation would need to scan body tokens
- formatter code would need filesystem or output details
- builder code would need to parse source or arbitrary text
- Path would need to become runtime-storable
- a route-specific URL would enter a public interface
- direct `.mtf` compilation would need to guess output placement
- a compatibility or fallback resource pipeline appears necessary
- an accepted TIR, `ConstValueStore`, public folded-value or link-fact owner would be bypassed
- unused resources would be read or hashed eagerly
- one phase crosses more than two unlisted stage boundaries

## Phase 0 - activation and authority publication

### Goal

Make the accepted Path and resource contract canonical, establish the current baseline and leave an implementation-ready owner map.

### Work

- move the roadmap entry from queued to active
- set this plan's status to active and current slice to Phase 0
- record the activation revision and baseline results in working notes, not this file
- refresh current source owners from module entry points and `index.md`
- create the canonical unsuffixed Path and resource references
- route those references from the compiler-facing language overview
- update compiler architecture for:
  - AST-owned resource-expression classification
  - `ConstValueStore` structural values
  - public stable resource facts
  - HIR resource operations and link facts
  - module publication of resource source deltas and diagnosed watch interests
- update build architecture for:
  - the boundary-wide resource source registry
  - exact entry and package resource unions
  - URL contexts
  - output planning, conflicts and invalidation
- update the data-layout authority for any durable Path syntax or compact ID requirements
- add or refresh a truthful deferred progress-matrix row
- inventory current path, template, HIR, provider, HTML output and stale-cleanup tests
- record baseline counters and benchmark results for representative docs and focused resource fixtures
- add no production code

### Exit gate

- the plan no longer acts as the sole language or architecture authority
- every implementation phase has a current owner and adjacent consumer
- baseline validation and benchmark evidence are recorded
- documentation release build passes
- stop for activation review

## Phase 1 - atomic core Path and resource cutover

Phase 1 establishes the minimal complete source-to-output route. Its internal slices do not merge independently.

### Phase 1A - identity, type legality and semantic resolution

#### Work

- decouple dependency resolution from `CompileTimePath`
- make dependency APIs return dependency-owned resolved targets only
- add portable resource paths
- add stable module/provider resource owner and origin identities
- add the module-local origin and use tables
- add the build-owned byte-source registry and module source delta
- add builtin Path syntax, `TypeId` and canonical identity
- add the recursive availability classifier and reason payloads
- delete the file/directory Path type distinction
- reject Path at every runtime receiving boundary
- remove legacy exact-`@/` resource-path behaviour
- keep site-root URLs as ordinary strings
- implement AST expression-path classification
- resolve from the owning module root
- enforce module/package boundaries, strict case and canonical containment
- reject source extensions, missing extensions, directories and missing files
- preserve build-only watch interests for missing targets
- replace the test-only boxed Path expression with production `ExpressionKind::Path(ResourceId)`
- add Path and structural String payload support to `ConstValueStore`
- keep Path values HIR-invisible except when consumed by structural template or string construction
- resolve resource paths in all validated bodies and generic templates without a preparation scanner
- add typed diagnostics with exact source locations

#### Tests

- expression path versus dependency provider context
- `@/`, `@./`, parent and `@@` rejection
- missing extension, source extension, directory and missing-target diagnostics
- module-root-relative resolution from nested source files
- cross-module private resource rejection
- support-package ownership boundaries
- symlink escape and strict-case diagnostics
- stable origin ignores declaration location and output placement
- moving a declaration inside one module preserves origin
- moving or renaming the resource changes origin
- several use sites share one origin and keep distinct locations
- one canonical file may back distinct logical origins
- ordinary, mutable, parameter and return Path diagnostics
- option, choice, map, generic argument and runtime nominal diagnostics
- inactive static branch still validates its resource path
- unmaterialised generic template still validates its resource path
- unused resource bytes remain unread and unhashed

### Phase 1B - structural values, TIR, HIR and link facts

#### Work

- add the ordered Text-or-Resource string model to local folded values
- keep pure text strings on the compact fast path
- update the `ConstValueStore` visitor and every current consumer
- add `TemplateIrNodeKind::Resource`
- update every exact-view TIR walker and summary
- preserve Resource through slots, wrappers, branches, loops and subtree copy
- add opaque formatter Resource anchors
- make Resource output-producing and non-reactive
- add Resource to const-template projection
- add Resource to owned runtime-template handoff and both walkers
- add a backend-neutral HIR resource append operation
- make HIR String constants structural
- make top-level const fragments structural
- record resource uses per HIR block and function
- retain deterministic use order and diagnostic locations
- collect base-module resource facts through the existing link-fact owners
- retain stable generic-template resource origins for the Phase 2 sidecar projection
- make known-Bool inactive branches contribute no HIR or link facts
- update direct `.mtf` compilation to return structural folded content
- reject direct final-text extraction when resources remain unresolved
- add target validation for builders without a resource URL-context policy

#### Tests

- direct const and runtime template insertion
- plain-string fast path remains flat
- resource-bearing String stored in a constant
- resource-bearing String passed and returned within one module
- const operation requiring final text is rejected
- top-level const fragment preserves Resource pieces
- direct `.mtf` structural result and text-only extraction
- slot, wrapper, branch and loop preservation
- non-reactive Resource node
- TIR subtree copy preserves origin and remaps local identity correctly
- runtime handoff walkers visit Resource once in document order
- HIR constants retain structural pieces
- HIR link facts retain deterministic resource-use order
- known-Bool inactive branch emits no use
- runtime branch retains possible uses
- unsupported builder context fails before lowering

### Phase 1C - resource link plan, placement and old-lane deletion

#### Work

- add one build-owned `ResourceLinkPlan`
- merge successful module source deltas transactionally
- build exact entry resource unions from function facts and fragment metadata
- build exact package unions from selected exports and reachable implementations
- plan validated output paths
- assign one semantic URL context to every reachable use
- build normalised URL maps once per context
- lower HIR Resource appends through the selected physical variant map
- render compile-time fragments through their containing output plan
- return resource bytes as ordinary central output records
- include resources in manifest ownership and stale cleanup
- deduplicate origins, physical sources, reads and warnings
- validate all output conflicts before metadata or byte reads
- emit large-resource warnings once per reachable physical source
- include relevant URL-map fingerprints in physical variant keys
- delete `RenderedPathUsage`
- delete module-wide rendered-path metadata
- delete eager path formatting from template heads and String coercion
- delete legacy relative-to-file and entry-root compile-time path rendering
- delete directory resource values
- delete `CompileTimePath`, `CompileTimePathBase`, `CompileTimePathKind` and `PathTypeKind`
- delete HTML per-module tracked-asset reconstruction
- delete or repurpose `tracked_assets.rs`
- remove path-format configuration that existed only for eager resource rendering
- update `index.md` and module-level ownership comments

#### Tests

- root and nested route URLs
- inline CSS uses document context
- standalone CSS uses stylesheet context
- runtime page code uses document context rather than JS bundle path
- one resource used by several pages emits once when placement matches
- distinct origins backed by one file hash and read once
- unused validated resource emits nothing and remains unhashed
- resource conflicts with HTML, CSS, JS, Wasm, manifest and provider outputs
- conflict validation happens before resource reads
- large-resource warning deduplicates by physical source
- stale resource output cleanup
- skip-unchanged resource writes
- URL-map physical variant separation
- no absolute path or URL in semantic artefacts

### Phase 1D - core merge gate

Before merging the core cutover:

- run the full focused test matrix
- run the compiler integration suite audit
- run `just validate`
- run `just bench-check`
- inspect counters against the activation baseline
- run the structured compiler/build/resource boundary audit
- resolve every accepted finding
- perform the Slice review
- verify no old production path, fallback, feature flag or compatibility adapter remains

## Phase 2 - public Path values, aggregates and generated functions

### Goal

Extend the completed core route across public interfaces, supported compile-time aggregates and generated sidecars without changing the resource model.

### Work

- add stable Path values to `PublicFoldedValue`
- replace flat public folded String payloads with the owned Text-or-Resource form
- add Resource pieces to public const templates
- support allowed Path constants, aliases, records and collections
- support exported Path constants
- support exported public nominal compile-time record shapes
- support exported folded records and collections
- preserve resource origin beneath aliases and re-exports
- validate that public resource origins are available to consumers
- project dependency-bound stable origins into consumer-local Resource IDs
- include stable resource facts in public-interface fingerprints
- keep content fingerprints outside public interfaces
- freeze generic-template resource origins through the existing materialisation context
- project them into generated-local resource tables
- include generated-function resource uses in generated link facts
- emit generated resources only when the generated function is materialised and reachable
- keep Path illegal as a generic type argument
- avoid implementing later anonymous const-record syntax
- update progress status for each accepted sub-surface

### Tests

- exported Path used by a consumer template
- exported resource-bearing String constant
- exported CSS template with a font Path
- support package exports Path
- alias and re-export preserve origin
- private resource origin cannot leak through a public value
- supported public record and collection shapes
- two consumers share one output
- source and Builder package output prefixes
- package-prefix encoding is injective over current stable identities
- generic declaration validates a resource before materialisation
- generated sidecar remaps to its own local Resource ID
- unreachable generated resource emits nothing
- interface agreement catches differing stable resource refs
- byte-only changes do not change public semantic identity
- no absolute paths in public or generated payloads

### Exit gate

- public and generated boundaries use stable origins only
- no donor-local Resource ID crosses a boundary
- focused tests, `just validate` and Slice review pass
- stop for review

## Phase 3 - `$literal` and composable CSS

### Goal

Make template body syntax order-independent and allow CSS validation to preserve structural Resource anchors.

### Work

- add frontend-owned `$literal`
- represent body syntax as Template, Literal or Discard requirements
- merge requirements order-independently
- keep repeated equal requirements idempotent
- diagnose incompatible requirements at both locations
- make ordinary `$css` and `$css("inline")` use Template body syntax
- preserve opaque Resource anchors through CSS parsing and validation
- keep `$code` literal
- keep `$note` and `$todo` discarded
- keep `$raw` whitespace-only
- do not add `$css("raw")`
- update canonical directive documentation
- update compiler-owned highlighting and the external editor grammar through their normal owners

### Tests

- resource Path in inline and standalone CSS
- nested CSS templates
- literal attribute selectors
- grid line names
- directive order independence
- repeated equal requirements
- incompatible requirements report both locations
- `$literal` suppresses nested templates, expressions and resources
- `$raw` does not change syntax mode

### Exit gate

- CSS retains structural Resource anchors without text scanning
- body syntax has one order-independent owner
- validation and Slice review pass

## Phase 4 - provider-backed resources

### Goal

Move provider runtime assets and generated bytes into the same resource identity, reachability and output model.

### Work

- add stable provider resource-owner identity
- pass stable source-resource origins into provider requests where appropriate
- replace `PathBuf` runtime-asset semantic identity with stable resource declarations
- keep canonical provider source paths as IO facts only
- support provider declarations for unchanged source files
- support provider-generated bytes
- require stable provider output paths
- keep provider source files un-emitted by default
- attach runtime resources to exact exported-symbol and callable reachability
- deduplicate unchanged source Path and provider uses only through shared origin
- keep transformed and generated output distinct
- include source content fingerprint in provider caches that consume source bytes
- keep content fingerprint outside semantic resource identity
- publish provider resource results transactionally
- migrate external JavaScript runtime assets to the common model
- route provider conflicts through the central output planner
- remove provider-specific output-path hashing based on canonical source paths
- remove provider resource fallbacks and duplicate conflict logic

### Tests

- direct-selection and namespace provider clauses
- provider source not emitted by default
- explicit unchanged-source resource declaration
- transformed and generated provider resources
- stable output-path requirement
- exact exported-symbol reachability
- unreachable symbol suppresses its resource
- ordinary Path and unchanged provider resource deduplicate
- transformed provider output remains distinct
- deterministic disagreement for equal origins
- provider output conflicts with project outputs
- canonical provider paths do not enter semantic artefacts

### Exit gate

- one resource identity and conflict model covers project and provider resources
- structured boundary audit is clean after accepted fixes
- validation and Slice review pass
- stop for review

## Phase 5 - global state, invalidation and completion

### Goal

Complete one project/package-wide resource state, harden invalidation and remove every remaining fallback or duplicated owner.

### Work

- build one boundary-wide resource source registry over project, source-package and provider results
- use one injective package output-prefix encoder
- consolidate entry, package and provider resource plans
- sort final plans by output path then stable origin
- validate all conflicts before reading bytes
- hash each reachable file source once
- read each emitted physical source once
- share bytes across outputs without collapsing semantic origins
- build each URL-context map once
- include normalised URL-map identity in affected physical variants
- deduplicate warnings, hashes and reads
- finalise `ResourceBuildState` for later dev fast-path work
- prove byte-only, semantic and placement invalidation separately
- remove every per-module planning fallback
- remove obsolete path-format, rendered-path, tracked-asset and provider-asset tests
- replace implementation-shaped tests with canonical behaviour and invariant owners
- update benchmark workloads and scaling coverage where evidence requires it
- update language, template, Moth-template, package, project, HTML, compiler and build documentation
- update scaffolds and examples
- update the progress matrix truthfully
- update `index.md`
- update external editor grammar
- rebuild generated documentation
- mark affected audit-log rows stale under the audit rules
- delete this plan and remove its roadmap entry in the completion commit

### Final audit

Confirm:

- one resource origin vocabulary
- one module-local resource table model
- one boundary-wide byte-source registry
- one structural String piece model
- `ConstValueStore` remains the folded-value authority
- no arbitrary string or rendered-output scanning
- no eager URL or output path in semantic facts
- no eager hash or read for unused resources
- no old rendered-path lane
- no per-entry full resource-table clone
- no provider `PathBuf` semantic identity
- no route-specific URL in a public interface
- no duplicate package-prefix owner
- no compatibility resource pipeline
- no production test-only Path variant
- no missing Resource case in TIR, handoff, HIR, remap or validation walkers

### Final validation

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-check
cargo run --quiet -- build docs --release
```

Perform the final structured audit, resolve accepted findings and repeat every affected gate.

## Exit state

The work is complete when:

- every valid authored resource expression is resolved once through AST semantics
- Path is compile-time-only and enforced transitively
- resource origin, use and byte source are separate facts
- stable resource identity is owner-explicit and path-portable
- `ConstValueStore` owns Path and structural String values
- public interfaces contain stable resource origins only
- TIR, runtime handoff, HIR constants and HIR operations retain structural Resource anchors
- static inactive branches validate paths but emit no resource facts
- exact function, fragment and package reachability drives resource unions
- builders own output placement and URL contexts
- direct `.mtf` compilation never guesses placement
- unused resources are not read, hashed or emitted
- byte-only changes avoid semantic recompilation
- provider resources use the same identity, reachability and conflict authority
- all legacy rendered-string asset reconstruction is deleted
- current documentation and progress status match the implementation

## Deliberately deferred

- resource-only dev/watch rebuild fast path
- broader HTTP caching, ranges and compression
- configured resource roots outside `entry_root` or the owning package root
- extensionless resource escape syntax
- managed plain Markdown resource links and images
- Path options, choices, maps and generic arguments
- runtime Path values or filesystem access
- ordinary asset transforms and content-addressed output names
- cross-build resource caches and persistent serialisation
