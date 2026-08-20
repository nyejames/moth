# Moth Compiler Design Overview

Moth is a high-level language with first-class string templates. Its compiler is a staged, backend-neutral library used by the project tool, development server, tooling overlays and backend builders.

This document is the single source of truth for accepted core compiler architecture, semantic ownership and cross-stage compiler contracts. It describes the intended end state, including contracts that are not fully implemented yet. It is not an implementation-status report.

`docs/build-system-design.md` owns project bootstrap, Stage 0 graph construction, config, module and package topology, command policy, project builders, linking and output ownership. Read both documents when a task crosses the compiler and build-system boundary.

`docs/src/docs/codebase/compiler-design/**` is an educational explanation layer for compiler concepts and their relationship to Moth. It does not override this architecture document, `docs/build-system-design.md`, the language authorities or the progress matrix.

Companion authorities:

- `docs/build-system-design.md` for project and build orchestration
- `docs/src/docs/codebase/language/overview.mtf` and the canonical unsuffixed references it selects for source syntax and language semantics
- `docs/src/docs/design-scope/` for design bias and scope boundaries
- `docs/src/docs/codebase/memory-management/overview.mtf` for reference semantics, borrow validation, lifetime topology, retained-edge liveness, declared groups, affine ownership, Retained Edge Counting and backend memory lowering
- `docs/src/docs/codebase/style-guide/style-guide.mtf` for implementation standards
- `docs/src/docs/progress/@page.moth` for current support and backend coverage
- `docs/roadmap/roadmap.md` and `docs/roadmap/plans/` for implementation order and genuinely deferred design

User-facing pages under `docs/src/docs/**` teach the language. They do not replace this architecture reference.

## Task-reading guide

For every compiler task, read the opening authority text above and
`Architectural invariants`. Heading paths use `>` to name nested sections. Read
the selected heading through the next heading of the same or higher level,
including nested subsections unless the route narrows further. Read the full
document for architecture plans, cross-stage ownership changes, broad refactors
or thorough reviews.

| Task | Read in this document | Also read when affected |
|---|---|---|
| Module compilation inputs, outcomes, root roles or artefact lanes | `Compiler input and result boundary` | `docs/build-system-design.md` > `Deterministic scheduling and graph outcomes` |
| Diagnostic lanes, render context or deterministic diagnostic identity | `Diagnostics and deterministic identity` | `docs/src/docs/codebase/style-guide/style-guide.mtf` > `Diagnostics` and `Returning errors` |
| Cross-module declaration, type, builtin or binding identity | `Stable semantic identities` | `Public semantic interfaces` |
| Public surfaces, exported effects, aliases, conformances or project provenance | `Public semantic interfaces` | `Stable semantic identities` and the relevant language reference |
| Fingerprints, invalidation inputs or compiler-owned reuse facts | `Fingerprints and reuse facts` | `docs/build-system-design.md` > `Incremental and persistent artefacts` |
| Concrete generic materialisation or generated sidecars | `Generated concrete functions`; `Frontend stages > Stage 4: AST semantics > Generics` | `docs/build-system-design.md` > `Generated-function worklist` |
| Tokenization, header syntax, interface binding, source-kind preparation or local declaration ordering | The relevant section under `Frontend stages > Stage 1: tokenization`, `Stage 2: header syntax and interface binding` or `Stage 3: local declaration ordering` | `docs/build-system-design.md` > `Prepared-source orchestration` when Stage 0 consumes or schedules the result |
| AST typing, constants, traits, casts, templates, reactivity or another language feature | `Frontend stages > Stage 4: AST semantics` and the exact relevant subsection | The feature's canonical unsuffixed language references and routed memory material when value flow is affected |
| HIR shape, lowering, validation, numeric ownership or call targets | `Frontend stages > Stage 5: HIR and validation` and the exact relevant subsection | The affected Stage 4 producer, Stage 6 consumer or backend handoff |
| Borrow validation, transfer facts or exported access summaries | `Frontend stages > Stage 6: borrow validation` | The task route in `docs/src/docs/codebase/memory-management/overview.mtf` |
| Lifetime regions, escapes, retention, cleanup frontiers or exported lifetime summaries | `Lifetime-region and escape validation` | The memory task route and `docs/build-system-design.md` > `HTML project builder > Link planning and lifetime topology` when project lifecycles are involved |
| Memory-strategy selection, REC, handle tags or collector-free lowering | `Lifetime-region and escape validation` > `Memory-strategy planning` | `docs/src/docs/codebase/memory-management/retained-edge-counting/` and the backend memory route |
| Reachability, link facts, target checks or backend inputs | `Per-function link facts`; `Target-contract validation`; `Backend-facing compiler handoff` | `docs/build-system-design.md` > `Entry and package link planning` and the relevant builder section |
| Current source locations | `Compiler implementation map` | Open the owning module entry point and adjacent producer or consumer before changing code |

## Architectural invariants

- One directory-scoped `@*.moth` or `+*.moth` module is the canonical semantic compilation unit.
- A physical module is compiled once per project or package compilation boundary and owns local type, HIR, borrow and lifetime-analysis identity/facts.
- Every normal module included in a command's semantic graph has its dormant root work parsed, type-checked, lowered, borrow-validated and lifetime-analysed before any entry can activate it.
- Tokenization and declaration-shell parsing happen once. Later phases bind and consume retained syntax rather than reparsing source.
- Each semantic fact has one source owner. A later stage does not reconstruct the same fact from source or an earlier IR.
- Module interfaces use stable semantic identities rather than donor-local indexes.
- AST resolves constants, generic call inference, traits, casts and template semantics, then emits concrete generic requests. Generated functions are materialised, HIR-validated, borrow-validated and lifetime-analysed before backend handoff.
- Stage 4 validates both branches of an ordinary `if` before static selection. A known compile-time Bool selects one branch before HIR, and the selected branch retains its lexical scope.
- Terminality and durable generated requests are derived from the specialised active AST. An inactive static branch contributes no HIR or downstream executable facts.
- HIR never receives an `if` whose condition is already a known compile-time Bool.
- Build configuration cannot change Stage 0 graphs or declaration structure.
- Source does not select or inspect physical targets.
- TIR is AST-local. HIR receives folded strings or neutral owned runtime handoff data only.
- HIR is the first backend-facing semantic IR. Borrow validation reads validated HIR and writes side tables without rewriting it.
- Public semantic facts, executable state, backend-neutral link facts and compiler metadata are separate artefact lanes.
- User-facing failures use `CompilerDiagnostic`. Internal invariants and infrastructure failures use `CompilerError`.
- Backend validation consumes explicit roots, target assignments, validated HIR and validated lifetime topology. Lowerers never rediscover source meaning or reconsider lifetime legality.
- Static proof is the semantic baseline, not collection. GC is one permitted physical representation of an already legal topology and preserves the same accepted programs and observable behaviour.
- Lifetime-region and escape validation is mandatory and backend-independent. GC cannot bypass topology legality.
- Backends declare whether they support collector-free release lowering. A capable full-control release backend must not fall back to a tracing or reachability collector.
- Imprecise memory planning retains conservatively; it must not reject legal source. A missing physical strategy after successful topology validation is `CompilerError`.
- Parallelism, reuse and caching preserve deterministic identities, diagnostics and output order.

## Compiler input and result boundary

The build system owns discovery, source ownership, graph construction, provider scheduling and command-specific module selection. The compiler owns source preparation, interface binding, local semantic compilation and target-contract validation.

Exact Rust names may change. The ownership boundaries may not.

### Module compilation input

A canonical module compilation receives:

- a stable module identity and root role
- the module's semantic source set selected by Stage 0
- retained token and header-syntax preparation for every semantic source
- graph-resolved provider identities and dependency-ordered provider interfaces
- the namespace and capability surface selected for the project or package build
- resolved build-configuration values and synthetic compile-time interfaces visible to the module
- deterministic source identities and a diagnostic identity context

Source preparation and provider binding are deliberately separate.

`PreparedHeaderSyntax` is produced before the provider graph has been compiled. It contains syntax that can be known without opening a provider interface:

- tokens or source-kind prepared payloads
- declaration shells
- dependency clause shells and aliases
- structural provider references
- local declaration-ordering hints
- root-activity and fragment-placement metadata
- source `#Config` contract shells
- source locations, diagnostics and remap information

`BoundModuleHeaders` is produced when the build system schedules the module after its required providers have compiled. The compiler binds retained dependency clauses against immutable provider interfaces and produces:

- stable bound declaration identities
- bound canonical type and folded-value facts
- final file-local visibility
- source and binding namespace records
- receiver-surface visibility
- completed collision results

Binding does not retokenize source or reparse declaration syntax.

Provider-created binding interfaces are available before source-module compilation and may be bound as soon as provider discovery has produced them. A source-module dependency cannot become a stable dependency symbol binding until the source provider's public interface exists.

### Module compilation outcomes

A diagnosed source module and an internal compiler failure are different result classes.

```rust
pub type CompileModuleResult =
    Result<ModuleCompilationOutcome, CompilerError>;

pub enum ModuleCompilationOutcome {
    Success(CompiledModuleArtifact),
    Diagnosed(ModuleDiagnostics),
}
```

Contracts:

- `Diagnosed` contains user-facing diagnostics and no partial public interface.
- A successful artefact never contains errors.
- Consumers blocked by a diagnosed required interface are not semantically compiled.
- Independent graph branches may continue under build-system orchestration.
- An internal `CompilerError` aborts the owning project or package compilation because later results cannot be trusted.
- Structured warnings may be retained only on a successful artefact.
- A shared module produces one canonical diagnostic set rather than one repeated failure per blocked dependant.

The build system may collect successful independent branches for `check` or future LSP use. A backend never receives a partial linkable project.

### Compiled module artefact

A successful module result has four explicit data lanes plus fingerprints.

```rust
pub struct CompiledModuleArtifact {
    pub interface: PublicSemanticInterface,
    pub executable: ModuleExecutable,
    pub link_facts: ModuleLinkFacts,
    pub metadata: ModuleCompilerMetadata,
    pub fingerprints: ModuleFingerprints,
}
```

`PublicSemanticInterface` contains consumer-visible semantic facts.

`ModuleExecutable` contains module-local semantic state:

- the local `TypeEnvironment`
- validated module-local HIR
- borrow-analysis facts
- lifetime-region and escape-validation facts

`PublicSemanticInterface` also exports lifetime and effect summaries conceptually, including:

- fresh result roots
- aliases of one or more parameters
- projection results
- extracted or detached results
- aliases of another result
- independent result graphs
- retained-parameter relationships
- retention cardinality
- persistent-edge creation and destruction effects
- extraction effects
- whole-domain kill effects
- cleanup-frontier facts
- outlives constraints
- external boundary classification

Donor-local region IDs do not cross module interfaces. Exported lifetime summaries use stable semantic relationships.

`ModuleLinkFacts` contains backend-neutral facts used by graph linking and target validation:

- per-function source call edges
- stable binding-backed call IDs
- helper and capability requirements
- reactive features
- numeric and cast operations
- map and other target-gated features
- runtime path and asset requirements
- per-function project-context provenance
- generated-function requests

`ModuleCompilerMetadata` contains non-HIR compiler and builder-facing metadata:

- dormant root activity
- folded top-level fragment values and runtime insertion indexes
- resolved root-local entry metadata
- documentation fragments and API-index metadata
- rendered path usages
- structured warnings

Compile-time page fragments never live in HIR. HIR validation checks executable fragment operations only. An artefact-level validator checks compile-time fragment values, insertion indexes and their relationship to dormant root metadata.

### Module root semantic roles

A normal module may define declarations, dormant top-level runtime work and page fragments. All dormant root work is semantically compiled before the artefact is available for entry activation.

Support modules and the project package facade are API-only semantic modules. They may define functions, types, constants, traits and other legal declarations. Ordinary runtime code inside functions remains valid. They have no implicit `start`, top-level runtime statements, page fragments, route or builder artefact. Invalid root activity is diagnosed before executable HIR leaves the compiler.

The project package facade compiles with project-facade visibility supplied by the build system. Its semantic result is an ordinary immutable module artefact and public interface. The separate `ProjectPackageAssembly` and its project-wide assembly privilege belong to the build system.

`export:` is the only public visibility marker for every module root role.

### Normal-root `start`

A normal root's implicit `start` is compiler-synthesised, non-exported and cannot be bound through a dependency clause.

It is infallible as a function contract. It has no `Error!` return channel. Runtime failures that are not handled in source follow the applicable trap or invariant behaviour rather than becoming builder-defined error fragments.

`start` owns the normal root's dormant top-level runtime work and produces runtime fragment strings in source order. Entry assembly may activate it once after the module has already compiled. Compilation itself never activates it.

Support roots and the project package facade have no implicit `start`.

## Diagnostics and deterministic identity

Diagnostics are durable compiler data rather than a final formatting step.

### Diagnostic lanes

- `CompilerDiagnostic` owns source, syntax, dependency, config, type, rule, borrow and target-contract failures.
- `CompilerError` owns impossible compiler states, transformation failures, filesystem failures and tooling or backend infrastructure failures.
- `DiagnosticBag` owns stage-local accumulation.
- `CompilerMessages` is used at build and rendering boundaries.
- Diagnostic payloads carry structured reasons, source locations, symbols and semantic identities rather than pre-rendered prose.
- Deferred-feature diagnostics remain distinct from outside-design-scope diagnostics.

Type diagnostics carry semantic type identities plus context. Rendering resolves user-facing names through `DiagnosticRenderContext` and the relevant local type environment.

Every user-facing diagnostic has a stable code and descriptor independent of its rendered wording. A stable code is not repurposed for a different semantic diagnostic family. Renderers may improve wording and presentation without changing the payload identity or code contract.

### Build-lifetime render context

One project or package compilation boundary owns a diagnostic identity context from bootstrap through final rendering.

- `SourceLocation` stores interned path and scope identity rather than owned display paths.
- Parallel workers may return deterministic string-table deltas.
- File deltas merge in original source order.
- Module deltas merge in canonical module order.
- Diagnostics and warnings never merge in worker-completion order.
- Tokens, headers, visibility records, type-rendering contexts and artefacts are remapped before a later consumer uses them.
- A success or failure result that outlives the active compilation call carries the merged `StringTable` or an equivalent self-contained render context.

Full table cloning remains valid for genuinely independent identity boundaries. It is not the ordinary module-compilation strategy.

Process-local `StringId` values and absolute filesystem paths are not persistent semantic identities. A serialised artefact stores canonical logical identities plus self-contained strings or a remappable string table.

## Stable semantic identities

### Origin identities and export bindings

A declaration has a stable semantic origin identity rooted in its defining package, module and declaration.

Conceptual forms include:

```rust
OriginDeclarationId
OriginFunctionId
OriginTypeId
OriginConstantId
```

The exact names may change. These rules do not:

- donor-local AST indexes, HIR indexes and `TypeId` values do not cross module boundaries
- private declarations never receive a consumer-visible identity
- source aliases do not change origin identity
- identity assignment is deterministic across thread scheduling

A public re-export adds a separate stable export binding:

```rust
pub struct ExportBinding {
    pub exporting_module: ModuleId,
    pub public_name: PublicName,
    pub origin: OriginDeclarationId,
}
```

The origin identity remains stable when another module re-exports it under a different name. The export binding belongs to the exporting module and its public API name. Changing a re-export alias changes the exporting module's public-interface fingerprint but does not change the origin declaration identity.

### Cross-build stability

A public origin identity derives from:

- stable project or dependency package identity
- canonical module path
- module root role
- defining declaration name
- declaration category
- receiver identity where relevant

It does not depend on:

- the cosmetic suffix of a root filename
- the ordinary source file that contains the declaration
- source position
- declaration order
- thread scheduling

Moving an exported declaration between ordinary files in the same module preserves origin identity. Renaming it or moving it to another module changes identity.

### Type identity

Each compiled module owns one local `TypeEnvironment`. `TypeId` equality in that environment is the only valid comparison for module-local semantic decisions.

Cross-module interfaces use canonical type identities rather than donor-local `TypeId` values. Canonical identity covers:

- builtins
- module-owned nominal structs and choices
- transparent aliases
- options, collections, maps and fallible carriers
- concrete generic nominal instances
- generic parameters inside exported generic templates
- binding-backed external package types

A consumer may intern compact local `TypeId` handles for dependency-bound canonical types. The local environment retains an origin map to canonical identity. Cross-module equality compares canonical identity, never rendered names or unrelated local handles.

`DataType` is parse-only or diagnostic-only after semantic resolution. It must not drive executable AST, HIR or backend semantic decisions.

Access classification remains separate from type identity. Mutability, shared access and exclusive access do not create manufactured type shapes.

Collection and map identity remain canonical constructed shapes:

- growable `{T}` and fixed `{N T}` collections are distinct
- fixed capacity is semantic identity rather than an allocation hint
- `{K = V}` maps store key and value identities directly
- later stages query semantic shapes rather than parse syntax or private side tables

AST builds the local type environment. Early nominal registration creates identity and generic parameter metadata. Canonical fields and variants are written only after AST resolves their type shells.

Member queries expose borrowed field or variant views and direct lookup helpers. Later stages do not clone member lists for semantic lookup.

AST body emission uses a narrow interner over `TypeEnvironment`. It may intern derived types and dependency-bound canonical types but cannot mutate completed nominal declarations.

External parameters with no frontend mapping use an explicit unknown-external state. They never use sentinel `TypeId` values.

### Compiler-owned and binding-backed symbols

Compiler-owned builtins are neither source declarations nor builder-provided bindings. They own language-defined operations, builtin type policies, runtime error identities and compiler-defined cast evidence.

Binding-backed packages are typed semantic interfaces rather than Moth modules. They:

- use stable package and symbol identities
- may expose opaque types, constants and free functions
- may expose recursive package-local namespace paths
- do not expose source-defined receiver methods
- map to target helpers, imports, glue or native operations only after HIR

Source-owned wrapper types provide method-style APIs over external handles when needed.

Source-module namespace records remain shallow and field-access-only. They do not silently acquire the recursive namespace behaviour of binding-backed packages.

The bare `io` namespace is prelude policy for the Core IO package rather than a separate package category.

## Public semantic interfaces

A public interface contains only facts a semantic consumer may observe:

- exported origin identities and export bindings
- canonical exported type shapes
- folded exported constants and const-template values
- generic templates, bounds and required evidence
- exported traits and reusable conformance evidence
- receiver surfaces and visible methods
- function parameter access modes
- mutation, optional transfer eligibility and effect categories
- complete result provenance: fresh roots, parameter aliases, projections, extracted or detached results, result-to-result aliases and independent result graphs
- retained-parameter and outlives summaries
- retention cardinality and persistent-edge creation or destruction effects
- extraction effects and whole-domain kill effects
- cleanup-frontier facts
- external-boundary classifications
- relevant reactive effect summaries
- project-context provenance for every exported fact

Backend planning facts do not belong in this interface. Per-function calls, helper requirements, runtime assets and target-gated features live in `ModuleLinkFacts`.

Aliases affect source spelling. They do not replace semantic origin identity.

Receiver methods remain attached to their receiver type's exported source surface. They are not independent free namespace entries and cannot be bound, aliased or re-exported separately.

### Public-surface and package-export validation

AST rejects every exported semantic surface that transitively exposes an unavailable identity or prohibited project context.

Semantic surface validation covers:

- function parameters and returns
- struct and choice fields
- type aliases
- exported constants and const records
- generic bounds and templates
- trait requirements
- receiver methods
- reusable conformance evidence
- access, optional-transfer, complete result provenance, retention, cardinality, extraction, whole-domain kill, cleanup-frontier, outlives, external-boundary and reactive summaries

An exported semantic surface cannot leak:

- a private nominal type
- a private trait or evidence identity
- a private receiver surface
- a runtime anonymous-record type
- a project-context fact prohibited by the active package facade policy

A runtime anonymous record uses a hidden nominal type local to its source site. It cannot escape through an exported signature, field, alias, return, receiver method or trait evidence.

The compiler also records project-context provenance for executable source and generated functions in per-function link facts. Provenance follows direct value use, compile-time-derived implementation facts and source or generated call edges.

For external package eligibility, the build system rejects any declaration whose public semantic facts or reachable executable implementation directly or transitively depend on private `@project`. This includes an exported function that calls a private project-dependent helper. The validator does not treat implementation-only dependence as a reusable package specialisation mechanism.

### Synthetic compile-time interfaces

The compiler may consume specialised immutable interfaces produced outside ordinary module discovery, including the build-system-owned project-global interface.

A synthetic compile-time interface contains:

- stable member identities
- folded backend-neutral values
- source locations
- member-level fingerprints
- provenance
- no AST
- no HIR
- no runtime body

It enters visibility through the same dependency binding boundary as other interfaces. AST consumes its values and provenance but does not own its bootstrap or namespace policy.

## Fingerprints and reuse facts

Each successful base module records five separate fingerprints.

### Public-interface fingerprint

Covers the canonical semantic contents of `PublicSemanticInterface`:

- exported origin identities and export bindings
- canonical exported type shapes
- folded exported values
- generic template semantics and bounds
- trait and conformance evidence
- receiver surfaces
- access, optional-transfer, complete result provenance, retention, cardinality, extraction, whole-domain kill, cleanup-frontier, outlives and external-boundary summaries
- relevant reactive effect summaries
- project-context provenance

It excludes private bodies, source locations, warnings, formatting-only metadata and dormant root activity that is not public API.

### Implementation fingerprint

Covers executable body semantics and non-interface implementation facts that can change generated code. It includes private function bodies and bodies of exported functions when their public semantic facts remain unchanged.

Generated requests and link facts come from active specialised executable control flow. Configuration
dependencies use the existing fingerprint owners; they do not create a separate fingerprint family.
Exported folded values or effect summaries may vary when they depend on configuration, with provenance
retained, while structural public identity remains stable.
Configuration conditions cannot create or remove declarations or exports. They may change active
executable effects and derived link facts only.

It excludes dormant root activity and generated sidecar bodies.

### Dormant root-activity fingerprint

Covers compiler-synthesised `start`, top-level runtime work, page fragments and resolved entry metadata owned by a normal root.

### Runtime-dependency fingerprint

Covers backend-neutral link facts derived from callable functions and dormant root activity:

- helper and capability families
- source and binding-backed calls
- target-gated features
- runtime glue requirements
- rendered runtime path and asset facts

Generated-function requests are worklist dependencies carried with module link data, but they are not runtime-dependency fingerprint contents. A change to the emitted request set is covered by implementation and worklist invalidation, updates generated sidecars and relinks affected assemblies.

### Documentation fingerprint

Covers public documentation, editor metadata and API-index data.

### Invalidation meaning

- A private or exported body change does not recompile semantic consumers unless a public semantic fact or exported effect changes.
- An implementation change may require relinking or code regeneration without semantic consumer recompilation.
- A root-activity change relinks entries that activate the module.
- A runtime-dependency change updates capability, glue and asset planning.
- A documentation-only change regenerates documentation or editor indexes without invalidating semantic consumers or executable instances.

The build system owns invalidation, relinking and persistent cache compatibility over these compiler-defined facts.

## Generated concrete functions

Base module artefacts remain immutable. Concrete generic functions live in generated sidecars owned by the consuming project or package compilation.

A generated request is keyed by:

- stable generic declaration identity
- canonical concrete type identities
- required evidence identities

The declaring module owns and validates the immutable generic template. AST in a consumer emits requests
from the active specialised executable AST. Calls in inactive static branches may be frontend-validated
but do not enter the generated-function worklist. The build system deduplicates and schedules requests.
The compiler materialises each accepted request.

Each generated function artefact owns:

- its stable request identity
- a generated-local type environment or immutable canonical-to-local type delta
- concrete validated HIR
- generated borrow facts
- generated lifetime-region and escape facts
- generated exported lifetime and effect summaries
- generated link facts
- implementation, runtime and compatibility fingerprints

Generated HIR does not borrow the mutable local type environment of the requesting module and does not extend the declaring dependency artefact. Cross-module calls use stable targets.

A generated function may request further instances. Materialisation continues until the build-system worklist reaches a fixed point.

A diagnosed generated request exposes no partial generated artefact. It blocks only entries or package surfaces that require it. An internal generated-function `CompilerError` aborts the owning project or package compilation.

## Frontend stages

Stage 0 belongs to the build system. It selects the project and package graph, semantic source sets, provider order and command roots. See `docs/build-system-design.md`.

### Stage 1: tokenization

Tokenization converts source text into located tokens.

It owns:

- lexical recognition
- source location tracking
- string and template delimiter context
- numeric literal scanning and source diagnostics
- symbolic operator, assignment and mutable-declaration spacing diagnostics
- style directive token recognition through the supplied merged registry
- syntax-level rejection of unsupported or unknown directive forms

`numeric_text` owns shared numeric grammar, normalisation, separator and exponent validation and materialisation helpers used by later semantic consumers.

Frontend-owned directives are always present. Builder directives may extend the registry but cannot override frontend names. Tokenization and template parsing use the same merged registry.

`TokenizerEntryMode` selects the initial lexical state:

- ordinary `.moth` starts in code mode
- Moth template `.mtf` starts in an implicit template body while preserving original source locations
- plain Markdown `.md` is prepared before tokenization and has no tokenizer entry mode

The tokenizer does not resolve dependencies, types or declarations.

### Stage 2: header syntax and interface binding

Header work has two explicit phases so syntax is parsed once without pretending provider interfaces already exist.

#### Header syntax preparation

Syntax preparation is the only phase that discovers module-wide top-level declaration syntax.

It owns:

- dependency clause and public re-export syntax
- root-role-aware `export:` parsing
- dependency clause shells, flat direct selections and aliases
- declaration shells for constants, functions, structs, choices, aliases, traits and conformances
- dormant normal-root start-body separation
- compile-time fragment placement metadata
- source-kind adapters that synthesise ordinary declarations
- structural provider references
- conservative local declaration-ordering hints
- source `#Config` contract shells

Support roots and project package facades reject root runtime activity before executable HIR can be produced. Normal roots retain dormant start and fragment metadata.

Syntax preparation does not type-check executable bodies, fold expressions or open source provider interfaces.

`#Config` contract shells are not structural provider references and cannot affect Stage 0 edges.

#### Interface binding

After required source providers have compiled, interface binding resolves retained dependency clauses against immutable interfaces.

It owns:

- stable bound origin identities and export bindings
- bound canonical types and folded values
- final file-local visibility
- source namespace records
- binding-backed package namespace records
- receiver-surface visibility
- prelude and builtin reservations
- completed name and alias collision checks

Binding-backed provider interfaces may already exist before source graph compilation. Source-module bindings wait for provider interfaces.

Interface binding never copies provider declarations into the consumer. It never bypasses a facade to inspect private source.

#### Three reference classes

Header processing keeps three classes distinct:

- Structural provider references belong to Stage 0 graph construction.
- Dependency symbol bindings belong to visibility and AST semantics.
- Local declaration-ordering edges belong to Stage 3.

A dependency-bound declaration is never a node in the consumer's local declaration graph.

Local ordering edges include same-module facts needed before AST can consume declarations linearly:

- local type alias targets
- local struct and choice field type references
- local function parameter and return type references
- local explicit constant type references
- fixed collection capacities that use local compile-time constants
- local constant initializer references
- structurally visible local const-template control references
- local trait requirement and conformance references where ordering requires them

A reference to a dependency-bound declaration may support a structural provider edge and later become a dependency symbol binding. It is not a local ordering edge.

Declaration-shell parsers are shared with AST body-local declaration parsing so equivalent syntax remains on one parser path.

#### Source-kind adapters

Moth template `.mtf` preparation contributes one private synthetic `content #String` declaration. Its initializer is a structurally built `$md` template over the original body tokens. Nested templates without an explicit directive inherit the Moth template Markdown formatter. An explicit directive overrides that default.

Plain Markdown `.md` preparation renders raw Markdown to HTML and contributes the same private `content #String` declaration shape with a synthetic string-literal initializer.

Later ordering and AST folding treat both as ordinary compile-time constants. There is no Moth-template-specific or Markdown-specific AST, HIR, borrow or backend path.

A recognised source kind unsupported by the active builder is rejected with a typed dependency diagnostic. Resolution does not silently fall through to another extension candidate.

#### Direct Moth template service

The direct Moth template compiler service uses the same tokenizer, synthetic-header preparation, local declaration ordering and AST folding owners as integrated `.mtf` dependencies. It extracts the folded `content` constant and stops before HIR generation, borrow validation, target validation, backend lowering and output writing.

This service is a narrow compiler entry point, not a second Moth template parser or compiler mode.

### Stage 3: local declaration ordering

Stage 3 orders top-level declarations inside one canonical module using retained local edges. Stage 0 has already ordered provider modules and packages.

Stage 3 owns:

- topological sorting of local declaration shells
- cycle detection in the local declaration graph
- source-order stability among independent declarations
- local constant initializer ordering
- finalising the module's declaration order
- appending builtin declarations
- appending dormant normal-root `start` after declarations

It does not:

- order project or package modules
- copy dependency-bound declarations into the local graph
- inspect executable function or start-body references
- order body-local declarations
- rediscover dependencies

Same-file constants retain source-order semantics and same-file forward references are rejected. Cross-file constants in one module use header-provided local edges. Cross-module constants are already folded owned facts in provider interfaces.

A concrete required local edge that names no local declaration is a Stage 3 graph diagnostic. A conservative symbol-shaped hint that cannot be proven to denote a local declaration may be deferred to AST so type or expression resolution can issue the precise semantic diagnostic. Stage 3 does not convert every unresolved hint into a missing-header error.

After ordering:

- AST consumes declarations linearly
- AST does not rebuild visibility
- nominal identities may be registered before their members are resolved
- missing local edges are fixed in header syntax preparation
- missing providers are fixed in the Stage 0 graph
- dormant `start` is never a dependency participant

### Stage 4: AST semantics

AST consumes sorted declaration shells and bound visibility. It resolves declarations, folds constants and templates, parses executable bodies, type-checks expressions, validates terminality and emits typed AST nodes.

AST owns:

- module-local semantic declaration resolution
- dependency-bound canonical type projection into local `TypeId` handles
- public-interface validation and canonical export projection
- executable body parsing and type checking
- body-local declarations
- function terminality validation
- contextual coercion at explicit receiving boundaries
- generic template validation and module-local request emission
- trait, conformance and generic-bound evidence validation
- explicit cast evidence resolution and builtin folding
- constant, anonymous const-record and const-template folding
- template composition, slot routing, folding and runtime handoff preparation
- reactive source and subscription metadata
- module-local TIR from direct parser emission through finalisation
- root-local entry metadata folding through ordinary module visibility
- common frontend value-to-string behaviour for Float, Number, templates and runtime lowering

When accepted deferred `group` / `into` syntax is implemented, AST also owns parser, scope, placement and freshness validation for declared memory groups. Group identity must not enter `TypeId`. Implementation is deferred.

AST is defined by ownership and data flow rather than a fixed number of internal passes.

The Stage 4 semantic sequence is:

```text
parse and type-check complete authored bodies
-> fold constants and final compile-time expressions
-> specialise known Bool `if`
-> commit active generated requests and executable summaries
-> validate terminality over active control flow
-> hand the active AST to HIR
```

Both authored branches are frontend-valid before static selection. Inactive branches are not skipped
during name, visibility, type, generic-evidence, cast or constant-expression validation.

#### Dependencies and visibility

AST consumes bound file visibility. It may validate semantic use of visible symbols but does not rebuild dependencies or discover top-level visibility.

All user-visible names use one collision policy. Same-file declarations, source dependency clauses, binding dependency clauses, aliases, prelude symbols and builtins cannot silently shadow one another.

If AST cannot resolve a top-level declaration by walking sorted declarations and bound visibility, the missing fact belongs in syntax preparation, interface binding, local ordering or the Stage 0 graph. It does not justify another discovery pass.

#### Type checking and coercion

Expression evaluation determines an expression's natural type and remains strict. Contextual coercion is applied only by the frontend owner of an explicit receiving boundary.

Boundary owners include:

- declarations and assignments
- returns
- concrete function parameters
- struct and choice fields
- default values
- typed collection and map entries
- template and string content
- explicit `cast` targets
- `then` arms whose enclosing producer has an explicit receiver
- compiler and binding-backed call contracts

AST carries semantic `TypeId` values through fields, receiver lookup, calls, operators and compatibility checks.

#### Value-producing blocks and terminality

Value-producing `if`, match and block-form `catch` are closed receiving constructs rather than general expressions.

They are valid only where the receiver is explicit. Every producing path must satisfy the receiving arity.

AST owns receiving-context, arity and terminality diagnostics. Non-unit success returns must be terminal before HIR lowering.

If HIR receives a non-unit function that can fall through, AST violated its contract and HIR reports an internal transformation error.

Both value-producing `if` branches must first satisfy normal completeness, receiving-arity and type
rules. A known Bool then selects one validated branch, so no runtime branch or hidden merge value is
needed. Runtime conditions retain the existing all-path rules.

#### Constants, build configuration and const records

Constants are compile-time declarations and metadata rather than runtime top-level statements.

Header preparation owns local dependency discovery. AST owns semantic checking and folding.

The build system resolves source `#Config` contracts before module AST compilation. Source defaults are deliberately restricted to self-contained primitive literals or `none`, as defined in `docs/build-system-design.md`. AST consumes the resolved primitive value and treats the declaration as an ordinary folded constant.

A source `#Config` declaration creates:

- no runtime wrapper type
- no HIR node category
- no source dependency clause
- no new visibility rule

A module folds each ordinary constant and const template once. Exported folded facts are copied into the immutable interface as owned backend-neutral values. Consumers never parse or fold provider templates again.

Private inferred const facts are advisory optimisation metadata. They do not affect semantics, declaration ordering or visibility.

#### Static Bool control-flow specialisation

Static specialisation applies to ordinary statement and value-producing `if` forms and uses the normal
folded Bool authority. It has no `#Config` special case and no config-specific branch node.

- both branches are fully frontend-valid before selection
- a known `true` selects the `then` branch
- a known `false` selects the `else` branch or an empty scoped result when no `else` exists
- the selected lexical scope is preserved
- runtime or unknown conditions remain ordinary `if`
- inactive calls do not publish generated requests
- inactive code contributes no effect, project-context, link or target facts
- no match folding, loop unrolling or general CFG partial evaluator is implied

Terminality and durable executable summaries observe the specialised active AST. HIR receives no
statically decided ordinary `if`; every `if` remaining in HIR has a runtime condition.

Fully folded struct and anonymous-record constants may become const records. Const records are compile-time field-access-only groups. They are not runtime values and cannot be passed, returned, stored or used through runtime methods.

Compile-time and runtime semantics agree on checked numeric failures, cast range checks, non-finite Float rejection and value-to-string formatting.

#### Generics

The declaring module owns and validates each immutable generic template.

At a call site, AST:

- infers concrete arguments from immediate call arguments and immediate expected result context
- resolves required visible trait evidence
- emits a module-local request keyed by stable generic identity, canonical concrete types and evidence identities
- diagnoses inference failures and missing evidence at the requesting call site

The build system owns project-wide or package-wide aggregation, deduplication and scheduling. The compiler materialises each selected request into the generated sidecar model defined earlier.

HIR and backends never infer generic arguments or consume unresolved generic template state.

#### Traits, conformances and casts

Trait declarations and conformances are compile-time frontend metadata.

Header syntax records trait and conformance shells. AST owns:

- stable trait identity
- requirement type resolution
- explicit conformance validation
- evidence visibility
- generic-bound checks
- bound-provided receiver-call resolution
- conflict and incompatibility diagnostics

Exported traits and reusable evidence use stable semantic identities. Consumers do not reconstruct conformance structurally.

Traits are not value types. Static bound calls resolve to concrete executable targets before HIR. HIR carries no trait objects, erased dispatch or runtime trait evidence.

Explicit `cast` is AST-owned. It resolves compiler-defined cast policy and evidence, performs foldable conversions and emits explicit runtime cast operations where needed. Contextual coercion and explicit casting remain separate paths.

User-defined cast evidence becomes an ordinary direct source-function call before or during HIR lowering. Cast evidence metadata itself does not cross into HIR.

#### Templates and TIR

AST owns all template semantics.

One module AST build owns one `TemplateIrStore`. Parser emission writes text, expressions, child templates, slots, inserts, wrappers and control-flow roots directly into that store. All TIR IDs are module-local typed IDs.

`Template` is a thin handle carrying a durable TIR reference and source location while AST construction is active. The reference contains a module-local root, the root phase and a value-carried `TemplateViewContext`; it is not a registry handle.

The phase sequence is:

```text
Parsed -> Composed -> Formatted -> Finalized
```

Folding requires `Composed` or later. AST-to-HIR handoff requires `Finalized`.

An exact `TirView` is the structural read authority after parser emission. Its `TirViewIdentity` contains the module-local root, phase and complete value context. The context contains only the optional expression, slot-resolution and wrapper-context overlay IDs, whose payloads remain owned by the module store. This identity determines effective reads, preparation and fold-cache keys and cycle detection. Consumers do not carry parallel authority tokens, store identities, context tables or overlay stacks outside the view.

Recursive consumers use two explicit view transitions:

- Structural child and wrapper transitions preserve the current complete expression overlay. Parsed references ignore their referenced slot-resolution and wrapper-context overlays; Composed or later references supply those two structural dimensions. Resolved slot sources and structural helpers retain the complete current context.
- Nested-value transitions enter an independently owned nested `Template` through that value's complete context rather than inheriting the containing structural root's expression overlay.

A composed or finalised root overlay contains effective overrides for every structural descendant reachable through children, wrappers, resolved slots, branches, fallbacks, loops and helper roots. Expression lookup uses that complete overlay followed by structural fallback.

One semantic preparation owner:

- validates every required reachable TIR root, node, overlay, wrapper and slot plan
- follows structural and nested-value transitions
- detects cycles by exact view identity
- classifies the value as foldable, runtime or helper
- preserves lazy runtime semantics
- returns `CompilerError` for missing authority

Preparation validates and classifies. It does not perform final folding or HIR handoff.

Preparation has two semantic modes. `Value` permits either a folded or runtime result while preserving lazy runtime behaviour. `ConstRequired` validates every required reachable branch, loop and helper before the owning caller rejects a runtime result through the established const diagnostic.

Discovering runtime dependence does not end authority validation. Preparation still validates every required reachable TIR structure so a valid runtime classification cannot conceal malformed internal state.

Folding and runtime handoff consume the same exact `TirViewIdentity` accepted by preparation and use the same structural and nested-value transitions. `fold_prepared_template` is the sole constant-fold entry. Runtime materialisation accepts `PreparedRuntime` plus the exact view and produces only neutral owned runtime-handoff vocabulary. Neither path classifies again, reconstructs overlays or applies a second interpretation of template structure.

AST finalisation:

- folds fully constant templates into strings
- preserves runtime `if` and loop bodies for lazy lowering
- prepares runtime slot source and site plans
- removes helper-only artefacts
- emits folded top-level fragment metadata
- replaces runtime templates with neutral owned handoff payloads

The TIR store is dropped before the completed AST leaves the stage.

No TIR store, ID, view, overlay, preparation type or registry crosses into a completed module, public interface, HIR or backend.

Missing roots, phases, overlays or exact-view authority are internal errors. Template meaning is never reconstructed from a second representation.

Number formatting uses the common value-to-string path. It does not add Number-specific TIR nodes.

#### Reactivity boundary

Reactivity is a constrained template and UI source-and-sink model rather than a general closure or function-value system.

The durable compiler ownership is:

- declaration parsing recognises reactive markers as syntax
- AST resolves ordinary `TypeId` values, stable source identity and subscriptions
- HIR carries backend-neutral source, sink and reachability metadata
- borrow validation treats subscriptions as read-only dependencies rather than active borrow lifetimes
- target validation rejects unsupported reachable runtime forms before lowering
- runtime update strategy remains backend-owned artefact policy

Reactivity does not become a second type system, implicit reflection mechanism or general higher-order function model.

### Stage 5: HIR and validation

HIR lowers fully typed AST and generated concrete functions into the first backend-facing semantic IR.

Each module retains local HIR IDs and its paired local `TypeEnvironment`. Cross-module executable references use stable targets. The callee body is never copied into the caller.

HIR owns:

- explicit local control flow
- locals, places, regions and terminators
- stable local and cross-module call targets
- concrete generated-function targets
- expression side-effect linearisation
- runtime template string construction
- template control flow as ordinary CFG
- runtime slot accumulators and appends
- map operations
- checked numeric operations
- runtime casts
- Float and Number formatting operations
- explicit external Float validation
- reactive metadata
- module constants and advisory private const facts
- function-origin metadata
- stable binding-backed external call IDs
- backend-neutral per-function link facts

Calls, checked operations, casts, map operations and other effectful expression work are linearised into statement preludes and temporary locals before the final value is used.

HIR does not:

- merge provider bodies into consumers
- carry donor-local identities across modules
- fold constants or templates
- reconstruct slot or render plans
- carry TIR
- carry compile-time page fragments
- receive a statically decided ordinary `if`
- evaluate `#Config`
- choose target- or platform-specific source branches
- solve generic arguments
- decide trait conformance
- carry runtime trait evidence
- decide final runtime ownership
- model exact lifetimes
- assemble routes or project artefacts

Every `if` remaining in HIR has a runtime condition. Static Bool branch selection is complete before
HIR and is never redone by HIR or a backend.

Plain binary operations remain valid for booleans and comparisons. Runtime template string construction lowers through explicit string append operations. Runtime scalar arithmetic and unary negation lower through explicit checked numeric statements. HIR validation rejects arithmetic that survives in the wrong representation.

#### HIR validation

HIR validation completes before borrow validation or target validation.

It checks:

- definition identities
- frontend type links
- region and CFG shape
- block ownership and terminators
- local and place references
- start-function and function-origin metadata
- module constants
- reactive metadata
- side-table mappings
- pattern and expression invariants
- finite Float values

Compile-time fragment values and insertion indexes are not HIR. Their validation belongs to the module artefact validator.

`NaN` and infinity in HIR are internal invariant failures.

A backend-neutral structured HIR view may be derived and validated when a structured lowerer needs it. It is not a second semantic authority and may be cached only as derived data.

#### Numeric ownership

Numeric behaviour has one owner at each layer:

- `numeric_text` owns lexical grammar, normalisation, separators, exponent rules and text materialisation.
- AST owns semantic numeric typing, constant evaluation, checked failure rules and cast evidence.
- HIR records numeric domain, operator and failure mode rather than backend helper names or one duplicated statement family per target.
- Compile-time and runtime operations round and fail at the same language-defined boundaries. `Number` rounds after every language-level operation result.
- Numeric optimisation facts remain side tables and do not mutate HIR.
- Target validation rejects unsupported reachable numeric domains before lowering.
- JS-only check elision remains in the JavaScript path until another backend needs a shared analysis owner.
- Float and Number formatting use the common value-to-string boundary consumed by templates and runtime lowering.

#### Call targets

Source calls use three explicit target classes:

- module-local function identity
- stable cross-module function identity
- stable binding-backed external function identity

HIR stores no dependency aliases, package source spelling or backend runtime names. Borrow validation resolves source targets to exported access and effect summaries. Target validation and lowerers resolve executable targets through explicit graph and link-plan inputs.

### Stage 6: borrow validation

Borrow validation runs once for each canonical module and once for each generated concrete function.

It enforces:

- shared and exclusive access rules
- optional transfer eligibility and no-later-use proof
- conservative aliasing for collections and maps
- legal mutable call access
- control-flow joins
- reactive invalidation facts

Borrow validation reads validated HIR and writes read-only side tables. It does not rewrite HIR, decide lifetime topology or decide final runtime ownership.

Optional inferred transfer is an optimisation path. When proof is unavailable on every relevant path, the operation remains a borrow. Failure to prove transfer must not reject an otherwise valid program. Immutable and mutable parameters may both receive inferred destruction responsibility at a proven final-use call site.

Borrow validation may emit preliminary return-root alias evidence for the later lifetime analysis. It does not own final result provenance, retained-edge summaries or topology constraints.

Closed external boundary profiles override this general rule. WIT value-only calls and restricted host-value crossings are non-consuming. Mutable opaque-handle access does not transfer Moth storage through the ordinary Moth ownership ABI.

Interfaces and fingerprints carry stable semantic identities and summaries only. Donor-local `TypeId`, HIR, allocation-family, region and counter indexes never cross module boundaries, and REC is never exposed as source semantics.

Cross-module call transfer consumes these summaries. It never opens the callee's HIR as local control flow.

Borrow validation resolves binding-backed function IDs through semantic package metadata to recover parameter access, mutation and return-alias contracts. It does not use source dependency clause syntax or backend runtime names.

Missing or inconsistent summaries are `CompilerError` invariant failures.

GC-native backends may ignore affine cleanup facts but cannot skip borrow validation or lifetime-region validation. Collected, debug and collector-free lowering accept and reject exactly the same programs.

Reactive subscriptions are read-only source dependencies rather than active borrow lifetimes.

Fresh rvalues passed to mutable call slots are materialised into compiler-introduced hidden locals before borrow validation. The checker then sees ordinary local access. Fresh-rvalue materialisation does not make temporaries valid mutable receivers.

## Lifetime-region and escape validation

Lifetime-region and escape validation is a distinct backend-neutral analysis after Stage 6 borrow validation and before target planning. It is not a numbered Stage 7.

Local per-function and module work:

- reads validated HIR and read-only borrow/effect facts
- owns allocation-family identity, complete result provenance, retention, escape and outlives constraints
- owns retention cardinality, extraction, whole-domain kill and cleanup-frontier candidate facts
- produces compiler-generated non-lexical lifetime intervals
- writes immutable side-table facts and exported lifetime summaries
- does not rewrite HIR
- does not choose target partition or physical allocation representation

Project and link work instantiates those summaries over the reachable call graph and builder-supplied lifecycle roots. Local module compilation cannot validate every cross-module or builder-lifecycle relationship by itself.

The analysis decides semantic lifetime ownership and topology legality. Diagnostics distinguish topology proven invalid from topology not proven legal by conservative analysis. Backends receive a validated topology and may not reconsider source legality.

### Retained-edge analysis

Retained-edge liveness belongs to this analysis, not to a separate runtime ownership system. It owns:

- retention domains and the edges within them
- edge creation and whole-domain kill effects
- final cleanup frontiers, which may be path-sensitive
- compiler-generated region epochs for repeated population and teardown

A final cleanup frontier lets an inferred region end before the aggregate that once held its values. Individual `remove` or `set` does not establish a frontier. Uniqueness scans and alias registries are rejected.

### Memory-strategy planning

After topology validation succeeds, a distinct memory-strategy planner selects one physical strategy per allocation family: stack or inline placement, static affine cleanup, inferred region allocation, explicit-group bulk reclamation, Retained Edge Counting, or host collection.

The planner is the sole owner of strategy selection. Borrow validation and lifetime validation supply facts and never choose a representation. Selection is deterministic for one compile and backend configuration, and never changes source legality.

Canonical REC design lives under `docs/src/docs/codebase/memory-management/retained-edge-counting/`, with detailed sequencing in `docs/roadmap/plans/retained-edge-counting-design-and-implementation-plan.md`.

### Backend handoff

Backends receive validated HIR, borrow facts, validated affine cleanup decisions, validated lifetime topology and a complete `ValidatedMemoryPlan`. That plan carries allocation-family layout, selected physical strategies, region and group placement, cleanup and destruction plans, REC decisions and physical coalescing decisions. Backends realise the plan and never reconsider source legality, recompute topology or select their own strategy. A backend that advertises full memory control must lower every accepted topology in a release build without a tracing collector; a missing strategy at that point is `CompilerError`.

Canonical design lives under `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/`. Declared `group` / `into` is accepted end-state syntax with implementation deferred; see `docs/src/docs/codebase/memory-management/declared-memory-groups/` for the canonical semantic contract and `docs/roadmap/plans/final-memory-management-redesign-and-implementation-plan.md` for implementation sequencing.

When group syntax is implemented:

- AST owns parser, scope, placement and freshness validation
- group identity must not enter `TypeId`
- HIR records explicit group metadata and exits
- recoverable checked failure paths remain explicit HIR control flow before memory analyses
- HIR still does not decide exact lifetime topology

## Per-function link facts

The compiler records backend-neutral facts for each executable source or generated function.

Facts include:

- module-local and cross-module source calls
- binding-backed external calls
- runtime helper and capability families
- reactive features
- numeric and cast operations
- maps and other target-gated features
- runtime path and asset usages

These facts are the compiler's linking authority. Module-wide summaries may exist as derived indexes but do not replace per-function facts.

Reachability operates on already-specialised HIR. It does not fold constants or choose branches itself;
Stage 4 has already removed ordinary `if` branches selected by known Bool conditions. Reachability
does not inspect borrow facts, decide target partitioning or perform tree shaking.

Some target checks require semantic type inspection in addition to raw reachability. Those checks use the paired type environment rather than syntax guesses or backend-owned type reconstruction.

## Target-contract validation

The build system supplies explicit validation roots and target assignments from entry or package link planning. The compiler owns validation semantics over those inputs.

Target validation:

- runs after HIR validation, borrow validation and complete project/link lifetime-topology validation
- traverses functions reachable from supplied roots
- includes reachable generated functions
- checks target-gated HIR features
- checks reachable binding-backed calls against target metadata
- may inspect semantic types where reachability facts are insufficient
- returns structured `CompilerDiagnostic` values for user-visible target failures
- returns `CompilerError` only for inconsistent compiler or builder metadata

Unsupported features in unreachable private functions do not fail validation.

Unsupported target features in an inactive static branch do not fail target validation because that
branch is absent from HIR and link facts. Frontend source errors in that branch were still diagnosed
earlier. Target assignment remains build-owned and source-neutral.

For mixed-target artefacts, validation receives the completed deterministic partition. It validates each function against its assigned target and verifies every permitted cross-target edge.

Root selection, command policy and partition strategy belong to the build system.

## Backend-facing compiler handoff

Backend lowerers receive only explicit validated inputs:

- module and generated-function HIR
- paired local or generated-local type environments
- stable local, cross-module and binding-backed call targets
- borrow facts
- validated lifetime-region facts and exported lifetime summaries
- validated affine cleanup decisions
- `ValidatedMemoryPlan` with allocation-family layouts, selected strategies, cleanup plans, destruction plans, REC decisions and physical coalescing decisions
- external boundary classifications
- per-function link facts
- selected-function, import and capability plans
- semantic layout identities required by the target
- builder lifecycle and runtime plans where relevant

Generated function sidecars carry the same conceptual lifetime summaries and facts as ordinary functions.

Binding-backed symbols carry a closed boundary classification such as WIT value-only or restricted host-binding. Exact Rust enum names are implementation detail. Missing compiler-owned boundary classification is `CompilerError`. Unsupported source-selected interface features are structured diagnostics.

Backend lowerers do not:

- load or parse source
- rebuild dependencies or visibility
- infer generic arguments
- reconstruct traits or conformance
- fold constants or templates
- interpret TIR
- rediscover project topology
- choose command, entry or route policy
- reconsider source legality, borrow facts or lifetime topology
- reinterpret `#Config` or static branch selection
- write final project outputs directly

A lowerer may implement a language-owned HIR operation with a target-native instruction or runtime helper only when the result preserves the full Moth contract.

Numeric checks, cast failure, finite-Float validation, map behaviour, error propagation and reactive semantics are not weakened because a target provides a more permissive primitive.

Concrete HTML assembly, JavaScript and Wasm partitioning, external JavaScript glue, tracked assets, output manifests and incremental scheduling belong in `docs/build-system-design.md`.

## Compiler implementation map

Current locations are navigation aids rather than permanent architecture.

- Frontend orchestration: `src/compiler_frontend/pipeline.rs`
- Tokenization and numeric text: `src/compiler_frontend/tokenizer/`, `src/compiler_frontend/numeric_text/`
- Header syntax, binding and declaration shells: `src/compiler_frontend/headers/`, `src/compiler_frontend/declaration_syntax/`
- Path syntax tables and general path resolution: `src/compiler_frontend/paths/`
- Dependency clause syntax, retained shells, target classification and interface binding: `src/compiler_frontend/headers/`, `src/compiler_frontend/headers/dependency_target.rs`
- Local declaration ordering: `src/compiler_frontend/module_dependencies.rs`
- Type identity, access, coercion, traits and builtins: `src/compiler_frontend/datatypes/`, `src/compiler_frontend/value_mode.rs`, `src/compiler_frontend/type_coercion/`, `src/compiler_frontend/traits/`, `src/compiler_frontend/builtins/`
- Binding-backed interfaces: `src/compiler_frontend/external_packages/`
- AST, constants, generics, templates and TIR: `src/compiler_frontend/ast/`
- HIR, validation and reachability: `src/compiler_frontend/hir/`
- Borrow validation: `src/compiler_frontend/analysis/borrow_checker/`
- Target-contract validation: backend feature and external package validation owners under `src/backends/`
- Integration cases and validation: `tests/cases/`, `src/compiler_tests/`, `justfile`
