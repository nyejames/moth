# File values and resource linking implementation plan

## Purpose

Replace eager rendered-path asset tracking with one structural resource-linking model built on ordinary `String` values.

The target is:

- top-level dependency clauses keep binding declarations and namespaces, and remain compile-time syntax with no first-class value
- an explicit-extension file path in expression position evaluates to a `String`
- `.mtf` and `.md` file values reuse the compiler-owned synthetic `content` constant their source-kind adapter already produces
- any other accepted file evaluates to a resource-bearing `String` that stays structural until a builder knows the output artefact
- bare `@/` evaluates to a structural site-root `String` that names no file
- every authored file-value path is graph-active before AST reachability, constant folding or static branch specialisation
- filesystem resolution happens once, before AST, and AST never probes the filesystem again
- stable resource identity contains no absolute path, output path, route, URL or content hash
- resource anchors remain structural through `ConstValueStore`, TIR, public folded values, HIR and link facts
- only resource uses that reach a selected entry or package output are read, hashed and emitted
- builders choose output placement and the URL context for each reachable use
- no compiler stage or builder scans arbitrary strings, rendered HTML, CSS or Markdown to rediscover files
- the legacy `CompileTimePath`, `RenderedPathUsage` and HTML tracked-asset reconstruction lanes are deleted

There is no source-visible `Path` type, and no replacement wrapper type such as `File`, `Resource`, `AssetPath` or `ResourceString`. `String` is the only language-level value type involved.

This plan owns the file-value and resource model. It does not reopen dependency-clause grammar, module topology, package declaration syntax, TIR architecture or output ownership.

## Current state

```text
WORK_ID: path-values-resource-linking
WORK_SOURCE: docs/roadmap/plans/path-values-and-resource-linking-plan.md
BASE_REVISION: 72405be4d1a1ba2cc7fb596ed4ce953a66fbb47e
STATUS: active
CURRENT_SCOPE: Phase 4 resource link planning and atomic old-lane deletion - slices 4a and 4b have landed
COMPLETED: Phases 0, 1, 2 and 3, plus Phase 4 slices 4a and 4b; Path type/availability lane deleted; graph-active prepared references and shared Stage 0 resolution; AST value-position file paths interpret the resolved-reference table with content reuse, structural resource and site-root pieces, and content-source ordering edges across every pre-body declaration shell; template folds, public projection and the direct `.mtf` service carry one owned structural string vocabulary, and one `ConstStringRequirement` owner diagnoses const-required operations that need final text; TIR folds, const-template projection, AST finalization and the runtime handoff all carry pieces, with a resource or site-root piece acting as a hard text-coalescing boundary; HIR string constants, append composition and string-id remapping carry pieces; persistent generic bodies capture their resolved file-reference subset alongside the path-syntax subset and each materialised body carries its own Stage 0 facts, so generic parameter defaults and generic bodies both resolve file values through sidecar-local resource tables and portable content values, and the interim `MOTH-DEFERRED-0001` file-value guard is deleted; both resource-table lanes survive HIR lowering paired with the HIR that indexes them, per-function link facts carry ordered resource and site-root uses, and resource origins carry authored provenance that never reaches identity; top-level const fragments carry structural pieces from fold through collection and into builder-facing `OwnedFoldedString`, and the remaining final-text wall is `render_entry_fragments`
NEXT_ACTION: Phase 4 slice 4c - build exact entry and package resource unions, and extend the byte-source registry to the `ResourceByteSource`/`ResourceContentState` shape, registration only, no reads
VALIDATION: full `just validate` green at the slice 4b checkpoint - clippy `--all-features -D warnings`, 8 feature lanes, source audit, 4656 + 788 + 17 unit tests, 1911/1911 integration fixtures, docs build, bench-ci, bench-scaling 3/3, timers erasure
AUDITS: Phases 1, 2 and 3 audits accepted after corrections, as recorded in Git history. Slice 4a as previously recorded. Slice 4b ran two lanes: no production defect. Two test-strength findings were required and inverted - collection must keep all-text `Pieces` unflattened, and `render_entry_fragments` must emit byte-identical HTML for `Text` and all-text `Pieces`. Two findings were deferred until 4e replaces the wall: a real `#[...]` emitter/service pipeline fixture, and caller-level JS/Wasm `CompilerMessages` mapping tests
BLOCKERS: none
NOTES: template heads are cut over for content occurrences only - resource, bare `@/`, extensionless and `.moth` head forms deliberately remain on the eager rendered-path lane until Phase 4 deletes it; any runtime use of a resource-bearing string still reaches `MOTH-INFRA-0001` at JavaScript expression lowering, which is general to every runtime position rather than specific to one operation, and closes as items 3 and 4 land HIR and TIR carriage and Phase 4 supplies the output boundary
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
- source-kind adapters that own `.mtf` and `.md` meaning and publish one synthetic `content` constant per content file
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
- `docs/src/docs/resources/file-paths.mtf`
- `docs/src/docs/resources/file-values.mtf`
- `docs/src/docs/packages/dependency-paths.mtf`
- `docs/src/docs/packages/dependency-clauses.mtf`
- `docs/src/docs/moth-templates/content-dependencies.mtf`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`

## Current migration surface

- resource paths currently resolve through `CompileTimePath` and render eagerly through the configured origin
- the HTML builder reconstructs tracked assets from module-wide rendered-path metadata
- `RenderedPathUsage` and module-wide rendered-path metadata still exist
- stable resource identity, portable resource paths, dense module-local `ResourceId` and the `ModuleResourceTable` already exist and are kept
- file-only regular resource targets, directory-target diagnostics and module-root-relative ownership already exist and are kept
- bare `@/` is accepted as a site-root URL `String` but is still rendered eagerly in AST instead of staying structural
- Moth-aware `$md` single-slash link targets still render literally, so they lag the accepted site-root contract
- a builtin `Path` type, its source spelling, its canonical identity and the recursive compile-time-only availability classifier exist on the branch and are deleted by Phase 1
- no expression yet produces a structural value: expression-position paths still fold eagerly to text in the template head

## Scope boundaries

Owned by this plan:

- file-value expression semantics and their diagnostics
- graph-active file reference preparation and Stage 0 file resolution
- semantic resource identity, module-local resource tables and build-owned byte sources
- structural `String` representation through every semantic value owner
- exact entry and package resource unions
- builder output placement, URL contexts, conflicts and invalidation
- deletion of the eager rendered-path lane and the abandoned `Path` type lane

Not owned by this plan:

- dependency-clause grammar, module topology or package declaration syntax
- project dependency declarations or package-manager work
- TIR architecture beyond adding structural resource and site-root support
- output-root policy, manifest ownership or the dev server
- general asset transforms, image processing or content-addressed output names
- template body syntax modes and any new template directive. The superseded plan carried a `$literal` follow-up; it is not in the canonical directive surface, and a plan sequences work rather than introducing language semantics. The resource model needs only that the existing template-body tokenizer preserve opaque anchors, which Phase 5 covers. A separate template plan can own body modes if they are accepted and documented canonically.

## Locked language contract

### Two path roles

Moth keeps two distinct uses of `@` paths, and neither owner reinterprets the other's path family.

A **dependency clause** is file-wide compile-time binding syntax. It creates a structural dependency edge, binds declarations or namespaces, participates in visibility and re-export rules, and produces no value.

A **file path in expression position** is an ordinary expression that evaluates to a `String`.

```moth
@core/math sin, cos, PI
@docs/intro content as intro
@vendor/drawing.js as drawing

docs #= @docs/intro.mtf
logo #= @assets/logo.svg
```

An explicit extension is required in value position. The extension is what tells a reader that the source refers to one file value rather than a declaration namespace. Dependency-clause source rules are unchanged: `@docs/intro.mtf content` stays a diagnostic because dependency clauses require extensionless source paths.

Context decides meaning. A registered provider extension names a provider target in a dependency clause and an ordinary resource file in expression position. Expression position never exposes a provider namespace and never invokes provider declaration semantics.

### File values in expression position

Every accepted value-position file path has natural type `String`. The file kind decides how that `String` is constructed.

Direct template use is valid:

```moth
#[@docs/intro.mtf]

[: <img src="[@assets/logo.svg]"> ]
```

### `.mtf` and `.md` content files

A `.mtf` or `.md` value path evaluates to that file's compiler-owned synthetic `content` `String`. It is the same folded value the source-kind adapter already publishes.

- the existing source-kind adapter remains the sole owner of `.mtf` and `.md` meaning
- no second `.mtf` parser, Markdown renderer or content extraction path is added
- no second `content` constant is created
- the source filename is not observable through the value
- structural resource or site-root pieces inside the content stay structural in the resulting `String`
- plain Markdown links and images inside a `.md` file keep their existing literal and untracked semantics

### Ordinary resource files

Any other accepted explicit-extension regular file evaluates to a resource-bearing `String`.

The value carries one structural `Resource` anchor rather than eager text, because the final URL is unknown until the builder knows the output artefact and its URL context.

### `.moth` files have no file value

```moth
source = @helpers.moth
```

is a diagnostic. A `.moth` file exposes declarations through dependency clauses. Source text is never exposed and no `String` representation is manufactured for it.

More generally, a compiler source kind may be used as a file value only where the language defines one canonical compiler-owned content `String` for that source kind. V1 has that property for `.mtf` and `.md` only. This is not a general source-file reflection system.

### Structural strings

A plain `String`, a resource-bearing `String` and a site-root-bearing `String` are one Moth type. The distinction is never observable through type identity, casts, reflection or overload resolution.

Conceptual compiler-local shape:

```rust
pub enum ConstStringValue {
    Text(StringId),
    Pieces(Vec<ConstStringPiece>),
}

pub enum ConstStringPiece {
    Text(StringId),
    Resource(ResourceId),
    SiteRoot,
}
```

Exact names and layout are implementation details. Keep a compact plain-text fast path; do not make every `String` allocate a piece vector.

A `String` containing `Resource` or `SiteRoot` pieces is fully folded structural compile-time data, so this is a valid constant:

```moth
logo #= @assets/logo.svg
image #= [: <img src="[@assets/logo.svg]"> ]
```

Because a resource-bearing `String` is an ordinary `String`, all of these are legal where they would otherwise be legal:

```moth
logo = @assets/logo.svg

logo_for |name String| -> String:
    if name is "main":
        return @assets/main.svg
    else
        return @assets/fallback.svg
    ;
;

logos {String} = { @assets/main.svg, @assets/fallback.svg }

maybe_logo String? = @assets/main.svg
```

Do not reject these merely because the `String` contains a structural piece. Any target-specific restriction must come from a real backend inability to preserve `String` semantics.

Whether an operation preserves structure or requires final characters is one owned question with one answer, not a decision each folding consumer makes independently. Give it a single policy owner:

```rust
pub enum ConstStringRequirement {
    PreserveStructure,
    RequireConcreteText,
}
```

`PreserveStructure` covers assignment and storage, template concatenation, interpolation, slot and wrapper composition, copying, collection and record storage, export and re-export, and passing or returning the value.

`RequireConcreteText` covers anything whose result depends on the characters: `String` equality, length, containment, prefix and suffix tests, parsing and casts from `String`, compile-time hashing, use as a compile-time map key, duplicate-key validation, a compiler or host call needing real characters, and a formatter that inspects characters rather than preserving an opaque anchor.

V1 takes the simplest consistent rule: while any `Resource` or `SiteRoot` piece remains unresolved, every `RequireConcreteText` operation is diagnosed. Do not attempt partial symbolic equality or partial hashing. The diagnostic is a structural-string diagnostic, not a path error. Runtime string operations are unaffected: by runtime lowering each selected physical variant has a concrete URL context, so `Resource` and `SiteRoot` pieces become ordinary text before the running program observes the value, and these become ordinary runtime string operations.

### Site-root URLs

Bare `@/` is a `String` naming the site root. It becomes one structural `SiteRoot` piece rather than eager text.

```moth
docs_url #= [@/, "docs/"]
```

A site root has no resource origin, no byte source, no filesystem target, no watch interest, no resource graph edge and no emitted resource. It is never checked, copied, hashed, rewritten, watched or included in a resource union.

The builder renders `SiteRoot` using the selected artefact's project-origin policy. Ordinary resource URLs never carry that origin; they are relative to the artefact that observes them.

A Moth-aware `$md` link target beginning with a single `/` uses the same `SiteRoot` piece. `//host/...` stays protocol-relative and literal, and `./`, `../`, `#`, `?` and explicit schemes stay literal under the existing Markdown URL contract. A `$md` link target is a URL, not a resource file: Markdown link parsing never becomes resource discovery, and rendered Markdown is never scanned.

Only the bare spelling is a site root. `@/logo.svg` remains a rejected absolute-root resource path.

### External URLs

Quoted strings remain ordinary untracked text and create no graph or input record:

```moth
external #= "https://example.com/logo.svg"
cdn      #= "//cdn.example.com/app.js"
favicon  #= "/favicon.svg"
```

### Config bootstrap

`config.moth` is self-contained and compiled before Stage 0 constructs the source graph, so file-value paths are not allowed there. This is an explicit semantic rule: now that file values have type `String`, no type rule rejects them.

```moth
project #= |
    icon = @assets/icon.svg,
|
```

is a diagnostic, as is a direct `.mtf` or `.md` file value in project config. Source `#Config` defaults stay limited to their existing primitive-literal forms and also cannot use file-value paths. Config defines build inputs; it does not observe a route or output URL context, so the site root does not belong there either.

## Graph activity and output liveness

### Graph-active file paths

This is the main architectural change from the superseded plan.

Every authored file-value path is graph-active before AST reachability, constant folding or static branch specialisation. Graph activity follows the authored path occurrence, not later expression liveness.

```moth
unused #= @docs/old.mtf

enabled #= false
if enabled:
    html = @docs/experimental.mtf
;

unused_resource = @assets/large.webp
```

All three still contribute structural input dependencies.

### The required invariant

```text
authored file path
    -> graph/input dependency always

semantic resource use
    -> retained only through normal semantic liveness

emitted resource
    -> retained only through entry/package reachability
```

Consequences:

- unused `.mtf` and `.md` value dependencies are still prepared and validated
- errors inside graph-active content sources are not hidden by later dead-code elimination
- a missing resource path in an inactive static branch is still invalid
- a resource file referenced only from dead code is still a known and watchable input
- a dead resource is not read, hashed or emitted
- a known-Bool inactive branch contributes no HIR resource use
- an unreachable private function contributes no output resource use
- an unmaterialised generic body contributes no output resource use
- none of those cases removes the earlier graph or input edge

Do not tree-shake paths before graph construction. Do not make graph structure depend on `#Config`, constant folding or reachability.

### Static control-flow specialisation

Graph and input validity is separate from executable and output liveness. Both branches of an ordinary `if` remain frontend-valid, and graph-active paths in both branches have already contributed input edges. After known-Bool specialisation an inactive branch publishes no HIR resource use and causes no emission. Do not add general CFG tree shaking.

## Discovery, resolution and publication

### Syntax remains syntax-only

`PathSyntaxTable` continues to own authored path spelling and location only. Do not add resource identity, filesystem resolution, output placement or content state to path syntax rows.

Ownership:

```text
tokenizer                  -> authored path syntax
header/source preparation  -> structural graph reference classification
Stage 0                    -> graph and physical input resolution
interface binding          -> passes the resolved table through, uninterpreted
AST                        -> value semantics
```

The handoff is explicit at every join, so two implementers cannot build incompatible halves of it:

1. preparation publishes file-reference syntax facts beside the prepared syntax
2. Stage 0 consumes those facts and resolves their physical targets
3. the resolved table is paired with the prepared syntax and supplied to module compilation
4. interface binding passes it through without interpreting it
5. AST resolves a path token through its file-owned identity and that token's resolved entry
6. the AST file-value owner is given no filesystem resolver

Conceptually:

```rust
pub struct PreparedHeaderSyntax {
    // existing fields
    pub structural_provider_references: Vec<StructuralProviderReference>,
    pub structural_file_references: PreparedFileReferenceTable,
}

pub struct ModuleCompilationInput {
    // existing fields
    pub resolved_file_references: ResolvedFileReferenceTable,
}
```

Exact Rust names stay open. Rules 1 to 6 do not.

### Preparation owns graph reference classification

Compiler-owned source preparation publishes the structural file references Stage 0 needs, from the dense path rows tokenization already produced.

Conceptual shape:

```rust
pub struct PreparedFileReference {
    pub source_file: FileId,
    pub path_syntax: PathSyntaxId,
    pub location: SourceLocation,
    pub class: PreparedFileReferenceClass,
}
```

Rules:

- no second tokenization, no second expression parser and no arbitrary source-text scan
- preparation knows which path rows a dependency clause consumed and excludes them from the value-reference family, so one authored path occurrence keeps one semantic role
- classification is shallow: bare `@/` is a site root and creates no file edge, explicit `.mtf` or `.md` is a content-source reference, explicit `.moth` is retained as a source-kind reference so AST can issue the precise diagnostic, another explicit extension is a resource-file reference, and an extensionless non-dependency path is left for AST to diagnose
- no type checking, folding or surrounding-expression parsing happens during this scan

Because classification never reads the surrounding expression, two edge cases need stated answers.

**Malformed surrounding code.** Every retained path token outside a dependency-clause-owned token range is structurally classified. A later AST syntax failure in the containing expression does not retract that graph fact. Order diagnostics so a speculative missing-file error cannot displace the primary syntax error at the same location.

**Explicit `.moth` value paths.** Stage 0 validates and identifies the target so AST can issue the precise diagnostic, and stops there. It does not add that file to the module's semantic source set, and its declarations never affect collisions or visibility. Invalid value syntax must not change the module before AST rejects it. The same rule applies to any future recognised source kind with no canonical file-value semantics: it is identified for its diagnostic, never silently treated as an ordinary resource.

### Stage 0 owns physical resolution

Stage 0 consumes prepared structural file references and owns physical graph and input resolution, not language value semantics.

For content-source references it ensures the referenced source enters the appropriate semantic source set before the consuming module reaches AST. The content source is prepared exactly once through the normal source-kind adapter. Content files are never opened recursively from AST.

A newly discovered `.mtf` may contain further file references, so discovery is a monotone worklist with an explicit fixed point rather than a single pass:

```text
seed the module root and the dependency-clause source set
    -> prepare each not-yet-prepared source once
    -> consume its structural file references
    -> add newly discovered .mtf and .md sources to the module source set
    -> repeat until no new source is added
    -> then run header aggregation and local declaration ordering
```

Rules:

- deduplicate by canonical `SourceFileId`, never by authored spelling
- retain every authored reference location separately
- prepare each physical content source exactly once
- a repeated reference is not a cycle
- a real dependency cycle through synthetic `content` declarations is diagnosed later by the local declaration ordering owner, not during discovery
- resource files discovered inside newly added sources enter the same build-input registry
- ordering and diagnostics never depend on worklist insertion order

For resource references it resolves the file as a build input and watch interest, validating the same ownership rules the resource model already uses: module-root-relative resolution, regular-file target, canonical containment, strict case, no parent traversal, no `@./`, no `@@`, no child-module or support-package private traversal and no symlink escape.

Stage 0 may create an unhashed build-owned resource source record. It must not read file contents, hash an unused file, choose an output path, render a URL, create a public semantic resource origin or decide whether the resource reaches an output.

What exists after Stage 0 and before AST is a build input, not a semantic identity. Keep the two records separate:

```text
Stage 0 build input
    canonical physical source
    owning root
    validated logical target
    watch interest
    no semantic origin

successful AST and module publication
    stable semantic origin
    association to the existing physical source record
    semantic resource facts
```

Three identities, three owners:

```rust
ResolvedResourceInputId
ResourceSourceId
StableResourceOriginId
```

- Stage 0 creates or reuses a `ResourceSourceId`
- AST creates a `StableResourceOriginId`
- successful module publication associates one or more origins with one source
- a diagnosed module publishes no origin association
- a watch interest for a missing target is build-only and carries no manufactured resource identity
- one canonical file may back several distinct logical origins
- equal origins must resolve to compatible source facts

This is what keeps a diagnosed module from publishing semantic identity early, and what removes any reason for a second filesystem lookup.

Filesystem resolution happens once. Stage 0 publishes the resolved target so AST can consume it:

```rust
pub enum ResolvedFileReferenceTarget {
    ContentSource {
        source: SourceFileId,
        content_declaration: LocalContentDeclarationIdentity,
    },
    ResourceSource {
        source: ResourceSourceId,
        owner_relative_path: PortableResourcePath,
    },
}
```

Exact IDs may differ. The rule does not: AST interprets an already-resolved target and never rediscovers it.

### AST owns value semantics

One focused file-value resolver consumes the exact `PathSyntaxId`, the prepared source identity, the Stage 0 resolved target, ordinary AST receiving context, the module resource table and the folded constant environment. It is not given a filesystem resolver, so rediscovery is unavailable rather than merely discouraged.

- `.mtf` and `.md` resolve to the existing folded synthetic `content` `String`; no path string is manufactured
- an ordinary resource interns the stable semantic origin into the module resource table and produces a `String` expression carrying one `Resource` piece, typed with the ordinary builtin `String` `TypeId`
- bare `@/` produces a `String` carrying one `SiteRoot` piece and creates no resource identity
- a source kind with no file-value semantics gets a typed diagnostic explaining that source declarations are consumed through dependency clauses

Precise diagnostics are kept for an extensionless value path, a directory target, a missing target, an explicit `.moth` file value, an absolute-root spelling such as `@/logo.svg`, `@./`, parent traversal, `@@`, a module-boundary escape, a strict-case mismatch and a symlink escape. An invalid file-value expression is never silently reinterpreted as a dependency namespace.

### Content-source declaration ordering

A direct `.mtf` or `.md` value path reuses the synthetic `content` constant, which creates a local compile-time dependency that declaration ordering must understand.

```moth
intro #= @docs/intro.mtf
```

requires `docs/intro.content` to be available before `intro` folds. Extend header preparation's retained local dependency facts so this creates the ordering relationship. Do not defer this to AST and reorder declarations there, and do not create a second content constant.

The rule is not limited to constant initializers. Every top-level declaration-shell expression that must be resolved or folded before ordinary body emission records a content-file declaration edge when it contains a `.mtf` or `.md` file value:

- ordinary constant initializers
- const-template bodies
- function parameter defaults
- struct field defaults
- entry metadata and `config:` values
- other compiler-owned declaration defaults
- top-level compile-time fragments

```moth
render |header String = @docs/header.mtf| -> String:
    return header
;

Card = |
    icon String = @assets/card.svg,
|
```

The parameter default needs the edge. The field default is a resource-only `String`, so it needs no content ordering edge, but it still needs its Stage 0 resolved reference like any other file value.

The same mechanism covers content files that depend on other content files. A real local content dependency cycle is diagnosed through the existing local declaration and source ordering authority rather than an AST recursion guard.

A runtime function body needs no local ordering edge, because the module's synthetic content constants are already complete before body semantics consume them.

### Module result boundary

Resolution produces two result families:

- compiler-owned semantic resource facts used by the module, generated delta and public interface
- build-facing byte-source and watch-interest facts used for IO and invalidation

Successful publication is atomic with the module artefact, public interface and generated delta. The build system merges the source delta into the boundary-wide registry only when the semantic result is publishable.

A diagnosed module exposes no partial HIR, public interface or semantic resource table. It may retain build-only watch interests for missing targets so creating the file can trigger a later rebuild. Those observations carry no public semantic value.

## Resource identity and ownership

### Stable semantic origin

Do not duplicate package identity beside a module identity.

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

1. **Resource origin** is semantic identity.
2. **Resource use** is one authored or generated anchor with a source location and an executable or metadata owner.
3. **Byte source** is the build-owned file or generated payload from which output bytes come.

One origin may have many uses. One canonical file may back several distinct logical origins. A resource use never owns a `PathBuf`.

The module table interns origins and nothing else. Uses live with the owners that already have exact liveness for them:

```rust
pub struct ResourceId(u32);

pub struct ModuleResourceTable {
    origins: Vec<ModuleResourceOrigin>,
    by_origin: FxHashMap<StableResourceOriginId, ResourceId>,
}
```

```text
ModuleResourceTable   origins only
FunctionLinkFacts     ordered executable resource uses
FragmentMetadata      ordered non-HIR resource uses
PublicFoldedValue     stable origins embedded in structural String pieces
```

This is the shape already on the branch. Do not reintroduce a module-wide use list beside it.

The exact Rust shape may change. These rules do not:

- local AST, TIR, `ConstValueStore`, HIR and module link facts use dense IDs
- the dense ID is always paired with its owning resource table
- public interfaces use stable origins
- generated sidecars use generated-local dense IDs or an explicitly paired shared table
- donor-local resource IDs never cross module or generated-sidecar boundaries
- use locations do not participate in semantic identity
- a repeated origin preserves every relevant use location without cloning origin data
- there is no single module-wide "used resources" list that destroys exact liveness ownership; executable uses belong to HIR and link facts, and compile-time fragment and exported-value uses belong to their existing metadata owners

### Build-owned byte sources

One registry spans project, source-package and provider results for a compilation boundary.

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
```

Preserve these rules:

- one semantic origin record per stable origin
- one physical byte-source record per canonical file or generated payload
- several origins may reference one byte source
- equal stable origins must agree on their byte-source facts
- canonical source paths are build IO facts only
- content hashes are output invalidation facts, not semantic identity
- graph activation may register an unhashed source or watch interest but must not force a byte read
- preparation and semantic validation do not read or hash resource bytes
- conflict validation completes before resource bytes are read
- reachable file bytes are hashed once per build state
- emitted file bytes are read once per physical source
- one read may feed several output records when distinct origins deliberately map to distinct paths

### Filesystem ownership

A file value may address only a file inside the current module or package's private filesystem ownership.

- traversal starts at the owning module root, not the physical source-file directory
- ordinary unrooted directories owned by that module may be traversed
- child normal modules and support packages stop traversal
- another module's private file cannot be addressed directly, including a private `.mtf`
- the project facade is not a global resource escape
- support visibility follows existing scoped-package rules
- canonical containment rejects symlink escape
- strict case validation reuses the established source-path policy
- logical aliases to one canonical file remain distinct origins
- a physical resource root outside the accepted owned roots is deferred

Cross-boundary content and resources travel through ordinary exported values:

```moth
export:
    logo  #= @assets/logo.svg
    intro #= @docs/intro.mtf
;
```

A consumer binds those as ordinary exported `String` constants, and the public interface retains any structural resource origins inside the `String`. There is no public visibility table for files themselves.

## Structural value flow

### `ConstValueStore` is the folded-value authority

Extend the existing store rather than adding a parallel resource constant arena. It must represent plain text, resource-bearing strings, site-root-bearing strings and composed strings mixing those pieces in any order. Content-file values use the same representation, so a `.mtf` whose content contains a resource naturally exposes a structural `String`.

Its recursive visitor stays the one conversion path used by public interface projection, HIR constant projection, direct `.mtf` extraction and metadata projection. No consumer reconstructs a structural `String` from AST syntax or TIR.

### Public folded values

There is no `PublicFoldedValue::Path`. The public `String` representation retains structural pieces instead:

```rust
pub enum PublicStringPiece {
    Text(...),
    Resource(StableResourceOriginId),
    SiteRoot,
}
```

An exported file-value constant is an ordinary exported `String`. Consumer aliases and re-exports preserve the resource origin. The public-interface fingerprint includes piece order, stable resource origins and site-root pieces, and excludes final URLs, output paths, absolute source paths and content hashes.

### TIR

`Resource` and `SiteRoot` pieces stay structural through TIR. A direct resource file value in a template becomes an opaque resource anchor without first turning into text, and a direct content-file value inserts that content `String` including any structural pieces already inside it.

Every exact-view TIR owner handles structural content: construction, summary, preparation, folding, formatting, slot composition, wrappers, branches, loops, subtree copy, const-template projection, runtime handoff and reactive metadata reduction.

`Resource` pieces are output-producing and non-reactive. `SiteRoot` pieces are output-producing and non-reactive with no resource identity. Formatters treat both as opaque anchors and never inspect filesystem paths, output paths, resource bytes, route layout or the configured origin. A missing case in an exhaustive TIR walk is an internal error.

### Persistent generic bodies

`PathSyntaxId` is file-table-local, and persistent generic capture already copies the path rows a frozen body references into a compact generic-owned table. Once Stage 0 file resolution is keyed to those path IDs, copying the syntax rows alone is not enough: at materialisation the generated AST needs the matching resolved target without using stale donor `PathSyntaxId` values, reopening the filesystem or reading back into mutable prepared-source state.

Capture and rewrite the resolved-reference subset in the same pass as the syntax subset:

```text
source PathSyntaxId
    -> source ResolvedFileReferenceId

persistent capture
    -> compact generic PathSyntaxId
    -> compact generic ResolvedFileReferenceId

materialisation
    -> generated-local resource or content target
```

Required coverage:

- a resource file value inside a generic body
- a `.mtf` content value inside a generic body
- an exported generic using its declaring module's private content
- repeated materialisations reusing one stable origin
- generated-local resource IDs, never donor-local ones
- no consumer-project filesystem resolution of a dependency's private source path

### AST-to-HIR runtime handoff

The owned runtime-template handoff carries `Resource` and `SiteRoot` nodes and no TIR IDs or store references. Both the immutable and mutable handoff walkers cover the new pieces exhaustively, and a missing case is a compiler invariant failure.

### HIR and module constants

HIR strings retain the ordered `Text`, `Resource` and `SiteRoot` shape until physical variant planning can assign concrete text. The structural string pool needs a site-root operation beside its resource append; a site root is not representable as text or as a resource. Required behaviour:

- ordinary resource-bearing `String` constants remain structural
- runtime template construction remains structural
- strings may pass through locals, calls and returns
- generated functions and top-level runtime fragments preserve them
- known-Bool inactive branches publish no HIR resource use

A `SiteRoot` piece carries no `ResourceId`, so it cannot be reached through the resource union. Give it one exact owner: each selected function and each metadata owner records its own site-root use, collected by the same structural walk that collects resource uses. It must not fall through the resource-union system merely because it has no dense ID.

HIR never contains absolute source paths, output paths, frontend-chosen route-relative URLs, content hashes or builder names.

### Runtime string behaviour

Resource-bearing and site-root-bearing strings are not compile-time-only values and flow through ordinary runtime string positions. The selected physical variant lowers its structural map before runtime observation, so a function may accept, store, compare, return and process the resulting `String` under ordinary rules once lowered.

## Validation and emission liveness

Semantic validation covers every graph-active file path, including one in an unused private constant, an unrendered helper template, either branch of an ordinary `if`, a branch removed by known-Bool specialisation, an unmaterialised generic template or a function unreachable from the selected entry.

Executable liveness is exact and separate. Entry planning unions resource uses from the selected `start`, reachable source and generated functions, runtime fragments, compile-time fragments, selected entry settings and reachable provider runtime requirements. Package planning unions externally selected exports, structural resource-bearing exported strings, reachable source and generated implementations and provider runtime requirements permitted by the package target.

Unions come from per-function link facts and metadata owners without rescanning HIR or scanning all compiled modules. An unused private string containing a resource produces no emitted resource.

A file-value path inside a generic template is graph-active when the authored source is prepared, even if no instance is materialised. Its resource and output anchor becomes durable only when the generated body is materialised and survives normal specialisation and reachability. Generated sidecars use generated-local resource IDs or an explicitly paired shared table; donor-local `ResourceId` values are never copied into generated HIR.

## Output placement and URL contexts

- project-local resources preserve their path relative to `entry_root`
- source, Core, Builder and dependency package resources use one injective package output prefix followed by their package-relative path
- provider-managed resources use the provider's declared stable output path
- generated provider resources use their declared path and generated bytes

The URL context is the artefact whose URL resolution rules observe the emitted string, not necessarily the file containing the generated code:

- ordinary page HTML uses the page document
- inline CSS uses the page document
- standalone CSS uses the stylesheet
- page runtime code uses the active page document unless the builder defines a different sink
- another builder supplies its own explicit context policy

A builder that cannot assign a context to a reachable use rejects it before lowering.

The site root is a separate builder capability. A builder either supplies a site-root policy or rejects a reachable site-root use during target contract validation. A backend never guesses `/`, and a target with no meaningful site root is a legitimate rejection rather than a reason to invent a default.

A `SiteRoot` piece inside a dependency's exported constant renders from the final consuming artefact's project-origin policy, not from the dependency package's own config or build origin. Public folded values carry these pieces across package boundaries, so this needs stating rather than implying:

```moth
export:
    docs_url #= [@/, "docs/"]
;
```

URL rendering picks the validated resource output path, computes a lexical relative path from the context artefact's parent, uses `/` separators, percent-encodes each UTF-8 segment, prefixes same-or-descendant paths with `./`, retains parent-relative `../` prefixes and never prepends a project HTML origin. `SiteRoot` is the separate piece and uses the project-origin policy.

A source or generated function that constructs a structural runtime `String` may lower differently for different entry URL contexts. The relevant normalised URL map or its fingerprint participates in physical variant identity. It never enters source legality, canonical HIR identity, public interfaces, type identity or semantic module identity. Do not clone canonical HIR per route before physical variant planning.

## Conflict rules

- one origin used by many entries emits once when output placement is identical
- the same output path and the same origin deduplicate
- the same output path and different origins fail with both useful locations
- resource output conflicts with HTML, CSS, JavaScript, Wasm, manifest and provider output
- unchanged provider use and ordinary resource use deduplicate only when origin and output path agree
- transformed or generated provider output has distinct identity
- all output paths and conflicts validate before hashing, metadata reads or byte reads
- warnings such as large-resource warnings are emitted once per reachable physical source
- conflict diagnostics use semantic origins and authored use locations, never reconstructed strings
- stale output cleanup uses ordinary manifest ownership, and skip-unchanged writes apply to resource outputs

## Fingerprints and invalidation

Semantic changes follow the compiler's existing fingerprint owners. Changing a resource's bytes without changing its stable origin does not change the `String` type or public semantic identity and does not recompile semantic consumers; it invalidates content and output fingerprints, re-emits affected outputs and may invalidate provider transforms that consume the bytes.

Changing a `.mtf` or `.md` content source is different: its synthetic `content` constant changes and follows ordinary source constant and public-interface fingerprint rules.

Changing route or output placement does not change source legality or public semantic identity. It replans `Resource` and `SiteRoot` text and invalidates affected physical and output variants.

Changing the project origin behaves the same way. It invalidates the physical and output variants of functions, metadata and static fragments that use the site root, and rerenders them. It does not change public semantic interfaces and does not recompile semantic consumers.

## Data-oriented and performance requirements

- one file-owned path syntax table, no second tokenization and no second expression parse
- at most one simple prepared-source walk over path tokens or dense path rows
- dense file-reference IDs where repeated cross-stage lookup benefits
- one filesystem resolution per authored file reference
- one stable origin record per unique semantic origin
- one physical source record per canonical file
- dense local resource and use IDs, and no `PathBuf` per resource use
- no URL string in semantic identity
- no full resource-table clone per entry
- no scan over every compiled module when exact reachability exists
- no formatter access to resource internals
- no arbitrary HTML, CSS, Markdown or string scanning
- no eager resource bytes, no eager content hashes and no output read before global conflict validation
- one content hash per reachable physical source per build state, one byte read per emitted physical source
- sorted contiguous final plans
- compact plain-string fast path, with piece storage measured before introducing broad generic containers or per-piece heap allocation

Counters:

- authored path rows
- graph-active file references
- deduplicated file targets
- content-source references and resource-source references
- resolved resource origins
- structural resource pieces and site-root pieces
- reachable resource uses
- emitted unique resources
- deduplicated origins and byte sources
- bytes hashed and bytes read
- output conflicts
- URL maps built
- resource table remaps or projections

Delete counters that exist only to measure the abandoned `Path` type machinery. Use existing benchmark and scaling infrastructure, and add a scaling series only when the implementation introduces a size-sensitive path that is not already protected.

## Work protocol

### Validation

A documentation-only slice ends with:

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

Run a structured boundary audit before the atomic core cutover merges, after provider resources join the common model, and before final completion. The structured audit is read-only; implement accepted findings in a separate correction slice.

### Atomic core cutover

Phases 1 through 5 are one atomic feature cutover for merge purposes. Internal branch checkpoints are fine. Do not merge a half-migrated semantic model, and do not keep old and new resource systems behind a feature flag, fallback or adapter.

The superseded plan drew this boundary after Phase 4. It moved because Phase 5 is not polish: exported and re-exported structural strings, generated sidecars, package assembly liveness, provider-managed resources, dependency artefact compatibility and direct `.mtf` service parity are existing production boundaries the language contract changes. In particular, merging while the direct `.mtf` service still returns flat text would leave two different `.mtf` semantics in one release, which the service contract forbids. Phase 4 remains a real internal review checkpoint; it is not a merge point.

### Stop conditions

Stop and return for design review when implementation appears to require:

- restoring a source-visible `Path` type, or adding another string type under any name
- parsing expressions during Stage 0, or reparsing source after preparation
- scanning rendered HTML, CSS or Markdown for resources
- reading resource bytes during graph construction
- making output reachability decide graph membership
- letting `#Config` alter file dependency topology
- letting a file value bypass a module or package facade
- placing final URL text in public semantic identity, or absolute filesystem paths in semantic resource identity
- flattening structural strings before a URL context exists
- a second `.mtf` or Markdown compiler path
- requiring resource strings to be compile-time-only
- a compatibility or fallback resource pipeline
- one phase crossing more than two unlisted stage boundaries

## Phase 0 - authority and branch reconciliation

### Goal

Make this design canonical before further implementation.

### Work

- rewrite this plan
- update compiler, build and data-layout architecture
- update the canonical resource language references
- update dependency-path references where the context split needs explanation
- update `.mtf`, `.md`, template and Markdown documentation where file values are now reachable
- update constants and string documentation where structural strings matter
- update the cheatsheet, progress matrix wording, design-scope wording and `index.md`
- inventory current branch code into preserve, adapt and delete
- record the branch tip and validation evidence in working notes
- rebuild generated documentation
- add no new `Path` type code

A design review of the first pass found the language model correct but the cross-stage joins incomplete. One correction slice closed them: the prepared-reference and resolved-target handoff, the transitive content-source worklist, the split between provisional resource inputs and published semantic origins, persistent generic resolved-reference capture, declaration ordering across every pre-body declaration shell, one concrete-text requirement policy, the site-root HIR, capability, cross-package and invalidation contracts, the atomic cutover boundary, the direct service wording, the conceptual resource table, the malformed-path and `.moth` classification rules, the `Path` identifier rule and the user-facing documentation drifts.

Two corrections deliberately depart from the redirect design document, both to resolve a tension inside it. The atomic cutover moved from Phases 1-4 to Phases 1-5, because merging with a flat-text direct `.mtf` service would ship two `.mtf` semantics in one release, which the service contract forbids. And the direct service no longer permits plain extraction when the caller supplies builder placement context; a builder-owned consumer renders structural content instead, so route and placement policy never enter the compiler service.

Phase 0 is closed. The design review is complete and implementation is cleared to start.

### Exit gate

- no canonical document describes `Path` as a source-visible type
- graph-active file-value paths are canonical
- the file-value result type is `String` everywhere
- dependency clauses remain canonical
- every cross-stage join has one named owner: preparation publishes, Stage 0 resolves, binding passes through, AST interprets
- the branch migration inventory is explicit
- the documentation release build passes
- design review complete

## Phase 1 - delete the Path type lane and add graph-active file reference preparation

### Goal

Establish the new early structural input boundary.

Start from a clean `just validate` baseline. The redirect landed as documentation-only commits, so the last recorded full-validation evidence predates it.

### Work

- delete the builtin `Path` type ID, canonical identity, source spelling, parsed annotation and `DataType::Path`
- return `Path` to the ordinary user identifier namespace; the final language reserves no `Path` spelling, and the design-scope page defers first-class filesystem paths without reserving the name
- delete the recursive compile-time-only availability classification and its diagnostics where no independent consumer remains, including the parameter, return, runtime-binding and carrier rejections and their tests
- delete the planned `PublicFoldedValue::Path`
- add prepared structural file-reference records
- mark dependency-clause path rows so they are not reclassified as file values
- collect non-dependency graph-active file paths without parsing expressions
- integrate them into Stage 0 source and resource input resolution
- include referenced `.mtf` and `.md` files in semantic source preparation
- create resource watch and input records without reading bytes
- publish resolved file-reference facts back to module compilation
- reject file-value paths from config bootstrap

### Tests

- graph activity for an unused private `.mtf`, `.md` and resource path
- a syntax error inside an otherwise unused `.mtf` dependency is reported
- a missing resource or content file in a known-Bool inactive branch is reported
- a path inside an unmaterialised generic template is graph-active
- repeated authored paths deduplicate the target without losing useful locations
- a graph-active unused resource is neither read nor hashed
- `@/` and ordinary quoted URL strings create no graph or input record
- a file-value path in `config.moth` is rejected
- transitive discovery reaches a fixed point: a `.mtf` referencing a `.mtf` referencing a resource prepares each source once, whatever the insertion order
- `Path` is usable as an ordinary user type name, for example `Path = |text String,|`
- a path token inside a syntactically broken expression is still graph-active, and the primary syntax error is not displaced by a speculative missing-file error
- an explicit `.moth` value path resolves for its diagnostic without entering the semantic source set or affecting collisions and visibility

### Exit gate

- graph construction no longer depends solely on top-level dependency clauses
- every authored file-value path is graph-active
- Stage 0 parses no expressions
- no `Path` type survives

## Phase 2 - AST String file values and content-source reuse

### Goal

Give value-position paths their final language-level `String` semantics.

### Work

- add one AST file-value resolver
- resolve `.mtf` and `.md` to the existing synthetic content `String`
- resolve an ordinary resource to a structural `Resource` `String`
- resolve bare `@/` to a structural `SiteRoot` `String`
- give `.moth` a precise no-file-value diagnostic
- add content-source local declaration ordering edges across every pre-body declaration-shell expression, not only constant initializers
- enforce module and package filesystem boundaries
- ensure AST consumes Stage 0 resolution rather than probing the filesystem
- replace the test-only boxed `ExpressionKind::Path` with the production string expression

### Tests

- the syntax distinction pairs: accepted `intro #= @docs/intro.mtf` and `@docs/intro content as intro`; rejected `intro = @docs/intro` and `@docs/intro.mtf content`; accepted `@vendor/drawing.js as drawing` and `drawing_url = @vendor/drawing.js`; rejected `helpers = @helpers.moth`
- direct `.mtf` and `.md` values, in a constant and in `#[@docs/intro.mtf]`
- content file depending on another content file, plus a real content dependency cycle
- a file value from a nested source file still resolves from the module root
- a file value cannot bypass a child module or support package facade
- a content-file value in a function parameter default and in a struct field default orders correctly
- a resource-only field default needs no content ordering edge but still resolves through Stage 0
- the rejected value-path shapes keep precise diagnostics

### Exit gate

- every accepted value path has natural type `String`
- no eager resource URL is produced for a value-position path; template-head resource, bare `@/`, extensionless and `.moth` occurrences stay on the eager rendered-path lane that Phase 4 deletes
- direct content-file values work in ordinary code and in templates

## Phase 3 - structural String vertical propagation

### Goal

Carry `Resource` and `SiteRoot` through every semantic value owner.

### Work

- extend `ConstValueStore` and preserve the plain-text fast path
- add structural public string values and update public interface projection and fingerprints
- update TIR, formatters, const-template projection and the runtime-template handoff
- update HIR string constants and append operations, top-level const fragments and generated-function remapping
- add the one `ConstStringRequirement` policy owner and diagnose compile-time operations that require unresolved final text through it
- capture and remap the resolved file-reference subset alongside the path-syntax subset in persistent generic bodies, and delete the interim Phase 2 rejection this replaces: `DeferredFeatureReason::FileValueInsideGenericBody` (MOTH-DEFERRED-0001). The generic parameter-default half of that rejection is already deleted; what remains refuses a file value written in a generic body at its definition site. Installing resource services on the generated lane also moves template-head paths inside a generic body off the eager rendered-path lane, so the captured subset must reach the template-head reader and not only `resolve_file_value`
- update direct `.mtf` structural output

### Tests

- ordinary string behaviour with resource-bearing values across runtime bindings, mutable bindings, parameters, returns, optionals, collections, map values, template composition, local flow, multi-function flow, exported constants, re-exported constants and generated function flow
- a compile-time operation requiring final text is diagnosed while pieces remain unresolved, covering equality, length, containment, parsing, compile-time map key use and duplicate-key validation
- resource and `.mtf` content values inside generic bodies, an exported generic using its declaring module's private content, and repeated materialisations reusing one stable origin
- a content file whose own value carries resource pieces, and one carrying site-root pieces, stay structural through const-template projection; moved here from Phase 2 because observing those pieces requires this phase's structural const-template projection

### Exit gate

- structural strings survive every required compiler boundary
- no consumer reconstructs them from source
- no `Path` payload exists

## Phase 4 - resource link planning and atomic old-lane deletion

### Goal

Complete one source-to-output route and delete eager resource reconstruction.

### Work

- build exact entry and package resource unions
- assign URL contexts and plan output paths
- lower `Resource` and `SiteRoot` pieces
- render structural values at the top-level const-template emission boundary, and carry pieces through top-level const fragments and page metadata; both are `StringId`-only today and refuse pieces explicitly, so Phase 3 item 4 could not reach them - `FoldedConstTemplateResult`, `AstConstTopLevelFragment` and `ResolvedConstFragment` gain piece-bearing forms here rather than earlier, where they would have been a dead parallel representation
- delete the interim page-metadata refusal `invalid_page_metadata.not_yet_renderable` outright - variant, reason key, message and its `html_builder_structural_page_metadata` fixture - once page metadata carries pieces; its wording claims the value has no final text yet, so once URL contexts are assigned that claim is false and rewording it would leave a refusal where the feature now works
- integrate resource outputs into central manifests, which requires retaining the module resource table past HIR lowering: neither an ordinary module result nor a generated sidecar keeps its table today, so a `ResourceId` in validated HIR currently has no reader after lowering. Retain both lanes together rather than one, so a sidecar resource is not silently unmanifested
- give resource diagnostics real provenance before emitting any: `ModuleResourceOrigin::first_authored_location` has no production reader today, and generated generic materialisation interns origins with a defaulted location because `intern_resource_origin` takes no location. The authored location is available at the parameter and struct-field materialisation sites, so thread it when the first diagnostic needs it rather than leaving a defaulted location to point at nothing
- validate conflicts before reads, and deduplicate sources, hashes, reads and warnings
- delete `RenderedPathUsage`, module-wide eager rendered-path metadata and `CompileTimePath`
- delete eager route-relative path formatting in frontend and template parsing; the residual callers left by Phase 2 are the template-head resource, bare `@/`, extensionless and `.moth` occurrence forms, which move onto the resolved table here
- delete HTML tracked-asset reconstruction from rendered path strings
- delete path-format configuration that exists only for eager rendering
- delete duplicate path filesystem resolution and every fallback that scans strings or generated HTML

### Tests

- nested route URL rendering, inline CSS context, standalone CSS context and runtime page string context
- several pages share one emitted resource
- static inactive branch validates input but emits no use, and a runtime branch retains possible uses
- an unreachable function emits no resource
- output conflicts, stale cleanup and skip-unchanged writes
- byte-only invalidation and physical variant separation by URL map
- site root: bare `@/` as `String`, default and configured origin, structural survival through folding, no resource origin or union membership, `$md` single-root route uses the same piece, `//` stays literal, `@/logo.svg` stays invalid
- a builder with no site-root policy rejects a reachable site-root use during target contract validation rather than defaulting to `/`
- a site-root piece in a dependency's exported constant renders from the consuming artefact's origin, and changing the project origin invalidates physical and output variants without recompiling semantic consumers

### Exit gate

- a complete file-value to structural string to output route exists
- the old path resource lane is gone
- unused resources remain unread and unhashed
- no rendered-text scanning exists

## Phase 5 - public, generated and service completion

### Goal

Close every non-mainline boundary.

### Work

- exported and re-exported resource-bearing string constants, and package assembly resource liveness
- generated sidecars and public fingerprints
- content-source file values across all legal same-owner cases
- direct `.mtf` service parity: consume the normal compiler-owned prepared dependency bundle or one shared compiler-owned mini orchestration over the same preparation owners, rather than a second path scanner or content graph
- keep the service contract exact: it always returns owned structural content plus required source facts; its text-only convenience accessor succeeds only when no `Resource` or `SiteRoot` pieces remain; a separate builder-owned consumer renders a structural result through the normal link plan and placement context. Route policy and output placement never enter the compiler service.
- provider-managed and generated resources on the common identity, reachability and conflict authority, replacing `PathBuf` runtime-asset semantic identity with stable resource declarations
- project and package dependency artefact compatibility
- CSS validation that preserves opaque resource anchors without scanning rendered text

### Exit gate

- no special boundary flattens or loses `Resource` or `SiteRoot` pieces
- one resource identity and conflict model covers project and provider resources
- CSS retains structural anchors without text scanning

## Phase 6 - hardening, measurements and documentation closeout

### Work

- complete the focused test matrix and audit integration coverage
- scaling and counter checks, and benchmark sanity
- remove obsolete path-format, rendered-path, tracked-asset and provider-asset tests
- replace implementation-shaped tests with canonical behaviour and invariant owners
- update language, template, Moth-template, package, project, HTML, compiler and build documentation; known drift carried from Phase 2 is `docs/src/docs/resources/file-values.mtf` line 12, which calls a `.md` value a "rendered content string" when it is the file's content, and line 68, whose unconditional structural claim only becomes true once Phase 4 removes the residual eager template-head forms
- update scaffolds, examples and the external editor grammar
- update the progress matrix truthfully and `index.md`
- rebuild generated documentation
- mark affected audit-log rows stale under the audit rules
- run the structured boundary audit and resolve accepted findings
- delete this plan and remove its roadmap entry in the completion commit

### Final validation

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-check
cargo run --quiet -- build docs --release
```

## Exit state

The work is complete when:

- top-level dependency clauses remain the declaration-binding mechanism
- value-position explicit file paths produce `String`, `.mtf` and `.md` produce their synthetic content, ordinary files produce structural resource-bearing strings, bare `@/` produces a structural site-root string, `.moth` has no file value, and no source-visible `Path` type exists
- every authored file-value path is graph-active independently of AST reachability
- Stage 0 parses no expressions, referenced content sources are prepared before consumers, and resource inputs are known and watchable without eager bytes
- filesystem target resolution occurs once, the resolved table reaches AST through prepared syntax and module compilation input, and AST owns value interpretation with no filesystem resolver of its own
- `Resource` and `SiteRoot` stay structural through `ConstValueStore`, TIR, public projection and HIR, with a compact plain-string fast path, and one policy owner decides which operations may keep structure
- generated sidecars preserve resource identity through captured and remapped resolved references, and compile-time operations cannot observe guessed final URLs
- `Path` is an ordinary user identifier again
- exact resource output liveness comes from existing entry and package reachability, and unused resources are not hashed, read or emitted
- URL contexts are builder-owned and site-root and resource URL semantics stay distinct
- output conflicts validate before resource reads, and byte-only changes do not recompile semantic consumers
- the `Path` type implementation, its availability diagnostics, eager `CompileTimePath` resource semantics and `RenderedPathUsage` reconstruction are gone, with no rendered-output scanner and no compatibility fallback
- plan, compiler architecture, build architecture and data-layout authority agree, user docs teach string file values, the cheatsheet contains no `Path` type, the progress matrix is truthful, generated docs are rebuilt and repository search finds no stale language-level `Path` contract

## Deliberately deferred

- resource-only dev and watch rebuild fast path
- broader HTTP caching, ranges and compression
- configured resource roots outside `entry_root` or the owning package root
- an extensionless resource escape syntax
- managed plain Markdown resource links and images
- first-class filesystem paths, directory values and runtime filesystem access
- source-file reflection and reading `.moth` source as text
- dynamic `String` to resource conversion and path construction from string pieces
- package namespace values, new declaration alias syntax and wildcard imports
- project dependency declarations and package-manager version solving
- ordinary asset transforms, image processing and content-addressed output names
- external HTTP resource fetching and runtime resource discovery
- cross-build resource caches and persistent serialisation
