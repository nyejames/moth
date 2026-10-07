# Moth Compiler Design Overview

> **Final design only.** This document specifies accepted compiler design, regardless of implementation progress. Status, migration steps and backend coverage belong in the roadmap and progress matrices. `V1` identifies a deliberately bounded feature contract, not unfinished implementation. Mark genuinely unsettled design explicitly.

Moth is a high-level language with first-class string templates. Its compiler is a staged, backend-neutral library used by project tools, development servers, tooling overlays and builders.

This document owns core compiler architecture, semantic ownership and cross-stage contracts. `docs/build-system-design.md` owns bootstrap, Stage 0 graphs, project and package topology, command policy, linking and output orchestration. Read both for cross-boundary work.

Accepted roadmap plans and their approved design decisions resolve gaps or conflicts in these references. Apply explicit supersession within its stated scope, not an assumption that any newer file overrides every older authority. Before retiring a plan, preserve its unique accepted contracts in their permanent owners and update references. Implementation behaviour and progress labels do not override design.

Companion authorities:

- `docs/src/developer-docs/language/overview.mtf` routes to canonical unsuffixed language references. They own source syntax and observable semantics. Paired Basic pages teach subsets.
- `docs/compiler-data-layout-design.md` owns source, token and diagnostic representation, identity lifetimes and the three failure lanes.
- `docs/src/developer-docs/memory-management/overview.mtf` routes to reference semantics, access validation, lifetime topology and physical memory contracts. Its permanent contracts take precedence over temporary memory implementation plans.
- `docs/src/docs/directives/directives.mtf` owns directive grammar and extension limits. `docs/src/docs/mon/mon-format.mtf` owns MON literal-data semantics.
- `docs/src/docs/design-scope/` owns accepted and excluded language boundaries. `docs/src/developer-docs/style-guide/style-guide.mtf` owns implementation standards.
- `docs/roadmap/roadmap.md` and its plans record accepted work, sequencing and open design. `docs/src/docs/progress/@page.moth` and the packages-and-builders matrix record support.

Educational compiler pages explain these contracts without overriding them. Routine internal API details belong in owning modules and doc comments. Use `index.md` to locate code rather than treating a source-path inventory as architecture.

## Task-reading guide

For every compiler task, read the authority text and `Architectural invariants`. A heading path uses `>` for nesting. Read a selected section through the next heading of the same or higher level, including subsections unless the route narrows further. Read the full document for architecture changes, cross-stage refactors and thorough reviews.

| Task | Read here | Also read when affected |
|---|---|---|
| Module inputs, outcomes, root roles and artefact lanes | `Compiler input and result boundary` | Build design > `Deterministic scheduling and graph outcomes` |
| Source identities, diagnostics and failure handling | `Diagnostics and deterministic identity` | Data-layout design and the style guide's diagnostic rules |
| Dense IR, expression ownership and capacity estimates | `Compiler storage and capacity`, Stage 4 > `Typed semantic expression ownership` and Stage 5 > `Dense HIR ownership` | Data-layout design for its separately owned records |
| Declaration, type, builtin or binding identity | `Stable semantic identities` | `Public semantic interfaces` |
| Public surfaces, effects, evidence and provenance | `Public semantic interfaces` | Relevant canonical language and memory references |
| Fingerprints and compiler reuse facts | `Fingerprints and reuse facts` | Build design > `Incremental and persistent artefacts` |
| Generics and generated sidecars | `Generated concrete functions` and Stage 4 > `Generics` | Build design > `Generated-function boundary` |
| Tokens, headers, source kinds and local ordering | Relevant Stage 1, 2 or 3 section | Build design > `Prepared-source orchestration` |
| Typing, calls, directives, constants or casts | Relevant Stage 4 subsection | The feature's accepted language contract |
| Multiple results and Core constant evaluation | Stage 4 > `Result slots and receiving boundaries` and `Core external constant evaluation` | Stage 5 > `Result channels` and routed memory references |
| Wiring | Stage 4 > `Wiring boundary` | The accepted Wiring source contract named there |
| Numeric identities, literals and profiles | `Numeric model and profile` and Stage 4 > `Numeric typing and literal materialisation` | Stage 5 > `Numeric ownership` and build bootstrap |
| Templates and structural strings | Stage 4 > `Templates and TIR` and `File values and resources` | Canonical template/resource references and build resource linking |
| HIR lowering and validation | Stage 5 | The producing Stage 4 owner and consuming analysis or lowerer |
| Borrow validation and transfer facts | Stage 6 | The memory overview's borrow-validation route |
| Lifetimes, retention and cleanup frontiers | `Lifetime-region and escape validation` | Build design > `HTML project builder > Link planning and lifetime topology` |
| Physical memory strategies and REC | `Lifetime-region and escape validation > Memory-strategy planning` | The memory overview's runtime/backend route |
| Reachability, target checks and backend inputs | `Per-function link facts`, `Target-contract validation` and `Backend-facing compiler handoff` | Build entry/package and builder sections |
| MON codec boundaries | `Rust-only MON service` | MON format and data-layout design > `MON data handoff` |

`Stage 4`, `Stage 5` and `Stage 6` above are under `Frontend stages`. `Build design` means `docs/build-system-design.md`.

## Architectural invariants

- One directory-scoped `@*.moth` or `+*.moth` module is the canonical semantic compilation unit. A boundary compiles it once, including dormant root work, before entry activation.
- Each semantic fact has one producing owner. Later stages consume retained syntax and validated facts rather than reconstructing meaning from source or another representation.
- AST semantics builds compact typed expression nodes addressed by owner-local IDs. Parser scratch, declarations, callable contracts, constants and diagnostic facts keep their own lifetimes rather than sharing one universal expression record.
- Stage 0 schedules compiler services. The compiler alone sequences local semantic stages and owns their outputs. Build code never mutates HIR or installs semantic summaries.
- Source preparation and provider binding are separate. Each preparation lane retains its token and declaration syntax for all later consumers.
- Shared argument parsing supports distinct signature-slot and record-field routing. Sharing punctuation never merges their semantic contracts.
- Cross-module facts use stable origins and canonical types, not donor-local indexes. Published module artefacts and generated sidecars are immutable.
- Moth-source modules and packages use Moth semantic interfaces even when emitted as Wasm. WIT is a foreign component boundary, not a replacement interface.
- Ordinary static `if` validates both branches before selecting executable work and preserves the selected lexical scope. Structural `$feature` selection is a separate pre-graph contract.
- Authored physical file references establish graph/input validity before executable reachability. Exact output liveness neither creates nor retracts graph membership.
- File values are `String`, with structural resource and site-root pieces until builder placement. Semantic origins, authored uses, byte sources and rendered URLs remain distinct.
- Source cannot select or inspect physical targets. One boundary-wide `NumericProfile` fixes the semantic `Int` width and `Float` precision independently of target partition and build profile.
- TIR is AST-local. Completed AST and HIR contain folded values or neutral owned runtime data, never TIR authority. HIR is the first backend-facing semantic IR.
- Zero, one and multiple results remain ordered semantic slots, with a separate error channel. They do not manufacture tuple types. Wiring capabilities do not turn strings into live templates.
- Borrow and lifetime-topology validation are mandatory. GC choice and build profile preserve access/lifetime legality. Physical planning cannot repair invalid topology or reject legal source because an optimisation failed.
- A compiler-owned validated memory plan determines each physical variant's layout, strategies, cleanup and count transitions. Lowerers only realise it. Full-control release lowering has no tracing-collector fallback.
- User diagnostics, operational failures and proven compiler bugs have separate failure lanes. Parallelism and reuse preserve deterministic identities, diagnostics and output order.

## Compiler storage and capacity

Long-lived, high-volume graph-like IR defaults to stage-owned dense stores addressed by compact typed IDs and ranges. Fixed-arity edges store IDs inline. Variable-arity edges use distinct typed ranges into their corresponding side stores. Construction uses growable storage and freezes to fixed dense ownership when later structural growth is no longer required. Recursive owning graph trees need a specific semantic or measured reason.

The typed expression store uses safe array-of-structures rows with an operation enum, inline fixed edges and typed side ranges. Keep result shape with its operation. Packed operation words add decoding and invariant cost; a result-only column adds storage and construction/drop work. Neither replaces the readable enum layout without whole-pass evidence that includes payloads, construction, destruction and invariants.

The rule includes retained typed semantic expressions in the AST stage. Parsing and receiving-context resolution may use short-lived scratch forms, but completed expression graphs do not retain recursive owning trees or unresolved parse states. Stage 4 defines their semantic ownership and HIR's concrete contract is in Stage 5. Source/token/diagnostic sizes and exceptions remain under the data-layout authority.

Each store names its issuing owner and the consumers that determine its lifetime. An index is meaningful only with that owner, not as a global identity or a persistent key. Typed ranges address entries in their owning side store, not an assumed contiguous run of child-node IDs. Range construction checks the `u32` start, length and end before allocation or mutation; empty ranges are valid, while multiple-result ranges contain at least two entries. Shared nodes are immutable. A transformation reuses an unchanged node or produces a replacement root and isolated changed data, without mutating another candidate's view.

Expression IDs are one-based, owner-local `NonZeroU32` values, allowing a four-byte ID and a four-byte `Option<ExprId>`. Their checked capacity does not change the zero-valid `TypeId` domain or existing zero-based HIR IDs. An expression view borrows its issuing store and carries its ID; child views keep that owner, and consumers do not pass detached ID/store pairs.

Expression candidates use producer-local scratch with distinct new-node and existing-node edge types. Validate scratch IDs and the borrowed existing-store owner before mutation, then promote only reachable scratch nodes in postorder and move their typed side data. A scratch ID never enters TIR or a retained AST root. Reuse an unchanged root, isolate a changed root until commit and drop failed candidate scratch. General suffix rollback is not the reclamation contract: an escaped root defeats a non-escape proof, and truncation does not return grown capacity. Append-only retention of complete discarded candidates is also avoided.

Discarding a handle does not reclaim append-only storage. Construction, failed candidates, folding scratch and generic validation have explicit release boundaries. Retained dead nodes, spare capacity, overlapping old/new forms and final destruction count towards the representation's memory cost. Freeze and publication are separate barriers: a store stops growing at its last structural producer, while validated remapping and summary-dependent finalisation finish before immutable publication. Expression compaction uses the surviving roots only after TIR's final reader ends, remaps reachable nodes and side data into exact-capacity storage and releases dead or replaced nodes. Measure the old store, new store and accessible temporary buffers together. Report allocator metadata, resize-time internal overlap and process RSS as separate observations with their coverage limits. The chosen representation preserves legitimate producers rather than freezing early and rebuilding a parallel store.

Compact encodings use typed accessors with one codec owner. Tag bits, optional-ID niches and short inline lists require checked capacities and measured common-path benefit. Full numeric payloads and exact spans remain lossless. The final encoding is selected from complete workload and retention evidence, not from record size alone. Type checking and folding throughput are primary consumers of this storage contract.

Capacity estimates are allocation policy, never semantic facts or hard limits. Reuse `FrontendArenaCapacityEstimate`, cheap statistics from parallel tokenization/preparation and existing header aggregation. Use naturally available exact counts, but add no counting traversal or duplicate lowering logic solely to predict capacity. Underestimation grows normally. Published HIR retains no unnecessary spare capacity. Compact-ID overflow uses the owning capacity diagnostic rather than truncation.

## Compiler input and result boundary

The build system selects source ownership, graph structure, providers and command roots. The compiler owns preparation, binding, semantic compilation and validation. Rust names may change without changing these boundaries.

For module compilation, a compilation boundary is one project graph or separately compiled package graph with its own identity context and publication owner. Entry activation and target partitioning do not create new semantic compilation boundaries.

### Canonical module compilation service

Stage 0 supplies one input to one compiler-owned service for each ready module. The service owns:

```text
bind provider interfaces
-> order local declarations
-> run AST semantics and build the public semantic projection
-> lower and validate HIR
-> collect active compiler-owned link facts
-> run borrow validation
-> complete generated semantic work and required call summaries
-> complete local lifetime-region and escape analysis
-> finalise the public semantic interface
-> validate and return the module result with its generated delta
```

Summary-dependent source/generated work converges inside this transaction. Every published source and generated function has validated HIR, borrow facts and local lifetime facts, with any conservative integer proofs paired to that exact executable. Link-level lifecycle validation remains a later operation.

A new local semantic stage belongs inside this service. Normal modules, support modules, project facades and synthetic single-file modules use it after preparation. The compiler receives compiler-owned options, not the project tool's configuration container.

Raw stage owners have frontend-only or narrower Rust visibility. The architecture boundary check also rejects reverse dependencies on build/project configuration. Build code cannot construct interface drafts, mutate semantic state, install summaries or rerun analyses. Named config and direct-template services are the only shorter semantic paths defined here.

Stage 0 may request provider-independent preparation before provider interfaces exist because graph discovery needs structural references. It schedules that work and consumes its structural results. The exception ends before binding and semantic compilation.

### Module compilation input

A module input contains:

- stable module identity and root role
- the selected semantic source set and retained `PreparedHeaderSyntax` for each source
- the matching Stage 0 resolved file-reference table
- graph-resolved providers and immutable dependency-ordered interfaces
- the selected namespace and capability surface
- resolved configuration values and visible synthetic compile-time interfaces
- the boundary `NumericProfile`
- final source identities and the diagnostic identity context

Stage 2 defines prepared syntax and bound headers. Binding consumes retained dependency clauses without retokenizing or reparsing. Provider-created bindings can exist before source compilation. A source provider must publish its public interface before consumer binding.

### Module compilation outcomes

A module returns success or diagnosed source through the typed operational-result boundary defined in `Diagnostics and deterministic identity`. Compiler bugs are not another result variant.

A successful compiler result is complete but unpublished. It contains the validated module lanes, public interface, generated delta, resource-source delta and identity data needed for publication. The build system remaps and publishes it atomically as a `CompiledModuleArtifact`. Production and publication are distinct ownership steps.

A source diagnosis publishes no partial module, public interface, generated sidecar or semantic resource table. A required generated failure diagnoses its enclosing module transaction. Build-only missing-resource watch interests may survive without manufactured semantic origins. Successful independent branches may remain available to tooling, but a backend never receives a partial linkable project.

Successful artefacts contain no errors and may retain structured warnings. Diagnosed results retain their report context without pretending to be artefacts. A shared module produces one diagnostic set, not a copy per blocked dependant. Build scheduling owns blocking and aggregation.

### Compiled module artefact

Separate four consumer lanes:

| Lane | Contract |
|---|---|
| `PublicSemanticInterface` | Complete canonical facts visible to semantic consumers, defined under `Public semantic interfaces`. |
| `ModuleExecutable` | Module-local `TypeEnvironment`, validated HIR, borrow facts and local lifetime/escape facts, plus optional profile-aware integer proofs paired with the executable. |
| `ModuleLinkFacts` | Backend-neutral per-function facts and generated materialisation requests, defined under `Per-function link facts`. |
| `ModuleCompilerMetadata` | Dormant root activity, folded fragments and runtime insertion indexes, folded root metadata, documentation/API-index data, semantic resource origins and non-executable uses, site-root use and warnings. |

The local lanes share a publication lifetime. Other modules consume the public interface rather than donor-local semantic state. Linkers and lowerers may consume the validated executable and link lanes through explicit handoffs. Retained compact identities remain paired with their lookup context.

Compile-time page fragments never live in HIR. The artefact validator checks folded fragment values, insertion indexes and their relationship to dormant root metadata. HIR validation checks executable fragment operations only.

### Module root semantic roles

A normal module may define declarations, dormant top-level runtime work and page fragments. Authored root runtime work or direct output requires an explicit purpose with an applicable consumer. `$page` supplies the HTML purpose. Compilation validates dormant work before publication. A dependency exposes declarations without activating that work or inserting fragments.

Support modules and the project package facade are API-only. They permit ordinary declarations and runtime function bodies, but have no implicit `start`, root runtime statements, page fragments, page purpose, route or builder artefact. Invalid root activity is diagnosed before executable HIR leaves the compiler.

The project facade consumes build-supplied facade visibility and produces an ordinary immutable module artefact. Its separate package assembly and project-wide assembly privilege remain build-owned.

Public visibility uses compiler-owned export metadata. `$export` is the accepted directive direction. Prefix-only versus optional directive-block form remains unsettled. Keep one final visibility syntax while preserving facade privacy and public re-exports.

### Normal-root `start`

The normal root's implicit `start` is compiler-synthesised, non-exported and unavailable to dependency binding. Purpose directives create neither another start nor a per-file compilation unit.

`start` owns dormant root runtime work and produces runtime fragment strings in source order. Entry assembly activates it at most once for that entry after compilation. Dependency binding and compilation never activate it.

Every executable normal-root `start`, including a synthetic single-file root, has built-in `Error!` alongside the existing fragment-collection success slots. Implicit checked failure materialises as `Error`. Typed `Error!` propagates with `!`; custom errors require explicit recovery.

Builders consume this typed outcome. They add no error-fragment channel and never execute `start` during compilation. Assertions and fatal runtime resource exhaustion remain outside recoverable failure.

## Diagnostics and deterministic identity

### Diagnostic lanes

`docs/compiler-data-layout-design.md` owns the complete failure and storage contracts:

- User-caused source, project, configuration, dependency, type, access and target failures use typed diagnostic drafts that compact into deterministic reports. Warnings share the diagnostic machinery. Deferred-feature and outside-design-scope reasons stay distinct.
- Expected operational IO, provider and host failures use `InfrastructureFailure`, outside the diagnostic store. A failure retaining compact compiler IDs carries their matching frozen identity context. Pre-source or non-source failures may instead carry ordinary filesystem/host context without compact IDs. Each failure has one context shape, never both.
- A proven compiler invariant violation uses `compiler_bug!`. It ends the owning compilation and carries self-contained bounded report facts before suspect worker state is discarded. Malformed input, capacity limits and unavailable capabilities never justify a bug panic.

Runtime program `Error` values and entry-startup failures belong to the program's execution contract, not these compiler failure lanes. Statically invalid numeric work and undischarged exported failure are source diagnostics.

Diagnostic schemas own stable codes and typed facts. Codes are not repurposed when prose changes. Stages preserve exact source spans, symbols and semantic reasons rather than pre-rendering messages. Type-display snapshots retain only what rendering needs, not complete type environments solely for diagnostics. Renderers resolve facts without mutating the report.

MON's public errors are a separate caller-owned data-service boundary, described under `Rust-only MON service`.

### Build-lifetime render context

One project or package boundary owns its mutable source, string and path identity construction. Source identities are final before downstream use. Inventory lanes register them upfront. Private discovery lanes normalise provisional identities once at their publication barrier.

Source-identity finalisation and span-table finalisation are different operations. A source's span owner remains live until its last span-producing consumer finishes, including config application and check-only work. The data-layout authority owns the exact build/freeze representation and private-discovery protocol.

Workers return append-only identity deltas. Canonical merging and remapping precede consumer access. File order, module order and diagnostic order never follow worker completion order. The build document owns the merge sequence.

A retained diagnosed or warning result carries its immutable identity context and report-local type-display data. An identity domain is the set of source, string and path IDs issued by one context. Reports from independent domains retain their own contexts, rather than combining raw IDs. Rare cross-domain diagnostic sites carry explicit ownership data instead of inferring an owner from an ID or path. A clean output with no compact compiler IDs need not retain a frozen compiler context.

Full table cloning is reserved for genuinely independent identity boundaries. Process-local IDs and absolute paths are neither semantic identities nor persistent compatibility keys. Persistent artefacts use canonical logical identity and self-contained or remappable data.

## Stable semantic identities

### Origin identities and export bindings

A declaration's semantic origin identifies its defining package, module and declaration. Source aliases do not change it. Private declarations gain no consumer-visible public identity. Donor-local AST, HIR and type indexes never cross semantic module boundaries.

An export binding separately identifies the exporting module, public name and original declaration. Re-exporting under another name preserves the origin. Renaming that public alias changes the exporter's public-interface fingerprint, not the declaration's origin.

### Cross-build stability

A public origin derives from stable project/package identity, canonical module path, root role, declaration name and category, plus receiver identity where relevant. It excludes root-filename suffix, ordinary source filename, source position, declaration order and thread scheduling.

Moving an exported declaration between ordinary files in one module preserves identity. Renaming it or moving it to another module changes identity.

### Type identity

Each compiled module owns one `TypeEnvironment`. Module-local semantic equality compares its `TypeId` handles. Interfaces instead carry canonical identities for builtins, nominal structs and choices, transparent aliases, constructed types such as options/collections/maps, concrete generic instances, exported generic parameters and binding-backed external types. Numeric identities retain width, signedness, precision or scale as applicable.

A consumer interns dependency types into local handles with a canonical-origin map. Cross-module equality compares canonical identity, never rendered names or unrelated local handles. After resolution, `DataType` is parse/diagnostic vocabulary only. Mutability and shared/exclusive access remain classifications, not manufactured type shapes.

Growable `{T}` and fixed `{N T}` collections are distinct canonical shapes. Fixed capacity is semantic identity, not an allocation hint. Maps retain key and value identities directly. Later stages query semantic shapes instead of parse syntax or private duplicate tables.

AST registers nominal identity and generic parameters early, then completes fields and variants after their type shells resolve. Body emission may intern derived or dependency-bound types but cannot mutate completed nominal declarations. Member queries borrow resolved views rather than cloning lists. Unmapped external parameters use an explicit unknown-external state, not sentinel valid IDs.

### Compiler-owned and binding-backed symbols

Compiler-owned builtins define language operations, builtin type policy, runtime error identity and closed cast contracts. They are neither source declarations nor builder bindings.

Binding-backed packages expose stable package/symbol identities, opaque types, constants and free functions. They may have recursive package-local namespaces but do not expose source-defined receiver methods. Source-owned wrapper types supply method APIs over external handles. Source-module namespace records stay shallow and field-access-only.

Binding metadata maps to imports, helpers, glue or native operations after HIR. WIT component discovery validates a supported foreign profile and projects it once into canonical Moth types and binding identities. AST and HIR carry no WIT syntax. The value-only V1 profile crosses independent semantic values, retains no Moth reference and returns independent Moth result graphs. Backend planning retains the closed boundary classification needed for Canonical ABI lowering.

Foreign `s8` through `s64` and `u8` through `u64` map to explicit `I*` and `U*` types. Foreign `f32`/`f64` map to `F32`/`F64`, not `Float`. `u8` does not imply `Byte`. The baseline WIT profile has no `f16` primitive, so `F16` needs an explicit foreign conversion or bit-representation contract. Correcting a foreign carrier never changes a Moth-native Core or Builder signature declared with `Int` or `Float`.

`Error.code` is a runtime Moth `U32` value with default `0`. It is distinct from compiler diagnostic codes, process-local IDs and Rust MON error codes. Its representation follows the ordinary numeric and runtime Error contracts.

An optional WIT-compatible export projects a package facade for foreign consumers without replacing its Moth interface or hiding Moth-only declarations. The bare `io` namespace is Core IO prelude policy, not a package category.

## Numeric model and profile

The canonical numeric and cast references routed by `docs/src/developer-docs/language/overview.mtf` own the complete numeric contract. This section owns compiler identities, profile propagation and separation from physical representation, not another operator matrix.

### Canonical numeric identities

The numeric types are `Int`, `Uint`, `Float`, `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32`, `U64`, `F16`, `F32`, `F64` and the `Dec` family, with scales `Dec0` through `Dec256`. `Uint` follows the profile-selected `Int` width, so it is `Uint32` under `Int32` and `Uint64` under `Int64`. `Byte` is a separate octet scalar, nominally distinct from `U8`, `I8` and `Char`, with no arithmetic.

`Int`, `Uint` and `Float` keep distinct canonical identities even when their selected precision matches a fixed-width type. `Dec0` aliases the scale-zero `Dec` identity. Other `DecN` types carry scale in identity. Leading-zero scales and scales above 256 are invalid.

### Compilation-wide numeric profile

`NumericProfile` independently selects 32- or 64-bit `Int` and `Float`. The default is 32-bit `Int` and 64-bit `Float`, and therefore `Uint32`. `Uint` follows that selection with no separate setting. The builder settles it before command-input materialisation and config compilation. It has no source, directive, build-input or command-line selection syntax.

The profile is fixed across directly linked Moth modules, generated functions, Core/Builder source packages and Moth-source dependencies. It never varies by function, target partition or development/release representation. Incompatible precompiled Moth artefacts require a matching build or rejection, never silent numeric adaptation. Foreign components retain explicit foreign types and separate value boundaries.

The profile affects numeric materialisation, folding, overflow limits, rounding, diagnostics and ABI/layout facts. `Uint` scalar element stride is 4 or 8 bytes where compact scalar storage is supported, following the selected `Int` width. Every numeric compiler service consumes it, including synthetic sources, config, direct templates and MON schema preparation. Existing fingerprint and compatibility owners include profile-dependent facts, including private implementation behaviour. They do not form a separate numeric cache system.

### Semantic and physical separation

Promotion, comparison, conversion, rounding and failure follow the accepted language rules on every target. Arithmetic checks its semantic result domain, not a narrow operand's storage width. Operator promotion does not authorise receiving coercions. Cast evidence follows source/target policy rather than an optimiser's proof about one value.

Semantic type, scalar storage layout and computation carrier remain distinct. A Wasm carrier's wrapping or truncating behaviour is not Moth arithmetic. Lowerers cannot substitute wrapping, saturation or arbitrary precision for a required checked result. Compile-time and runtime operations preserve the same failure order and rounding boundaries.

`Frontend stages > Stage 5: HIR and validation > Numeric ownership` defines layer responsibilities. `Backend-facing compiler handoff > Numeric scalar storage and carriers` defines physical handoff ownership.

## Public semantic interfaces

A public interface contains the complete semantic facts observable by consumers:

- exported origin identities and export bindings
- canonical exported type shapes
- owned folded constants and const-template values, including structural resource/site-root pieces and exact numeric width, precision or scale
- generic templates, bounds and required evidence
- exported traits and reusable conformance evidence
- receiver surfaces and visible methods
- parameter access modes and Wiring parameter contracts with canonical underlying types
- mutation, optional-transfer eligibility and effects
- complete result provenance, including fresh roots, parameter aliases, projections, detached stored results, result-to-result aliases and independent result graphs
- retained-parameter and outlives summaries
- retention cardinality and persistent-edge creation/destruction effects
- detached stored-result effects and whole-domain kills
- frontier-enabling retention effects and outcome-sensitive success/error effects
- external-boundary classifications
- project-context provenance for every affected exported fact

Per-function calls, capabilities, resource uses and other backend-planning facts live in `ModuleLinkFacts`, not the semantic interface. Exported resources carry stable origins, never donor-local IDs, output paths, routes or rendered URLs. Receiver methods stay attached to their type's exported source surface and cannot be bound, aliased or re-exported independently.

### Public-surface and package-export validation

Validation transitively covers every interface fact above, including signatures, fields, aliases, constants, generic templates, evidence, receivers and all access/effect/lifetime summaries. AST validates resolved source surfaces. Later analysis completes and validates its summaries before interface publication.

An export cannot expose a private nominal type, private trait/evidence identity, private receiver surface, hidden runtime anonymous-record identity or project-context fact prohibited by package policy.

The compiler records project provenance through folded values, compile-time-derived implementation facts and source/generated call edges. The build system owns external-package eligibility, including rejection when an exported function reaches a private project-dependent helper. Implementation-only dependence is not an escape from that policy.

Exported and custom-error boundary diagnostics carry a bounded failure witness (`BuiltinFailureWitness`). The witness retains at most three private call sites plus the terminal numeric operation or compound write-back, with its typed builtin cause codes and an elided-hop count for any further hops. The bound lives in the single shared `retain_bounded_witness_hops` in `src/compiler_frontend/ast/expressions/failure_facts.rs`, used by both the AST and HIR witness walkers. Source order selects the witness at each hop and canonical diagnostic ordering ignores worker completion order, so parallel and repeated compilation agree. The walk is a deterministic depth-first search over those ordered contributors: it skips recursive back-edges and backtracks from dead ends, so a later real producer behind an earlier recursive call still supplies the origin. The same-compound write-back keeps precedence over its arithmetic wherever the remaining order starts. Cross-module leaves, external leaves and unresolvable callees stop the witness as terminal leaves, all without inventing an origin or cause codes, and only a skipped back-edge permits trying the next contributor. A missing summary is a compiler error, never evidence of no failure. Donor ownership is preserved because only stable identities resolve and donor-local IDs never cross the boundary. The boundary emits one diagnostic, not a cascade at every private hop.

The evidence and corrective guidance each implicit-failure, catch and error-boundary diagnostic must show live in the Diagnostics section of `docs/src/developer-docs/style-guide/style-guide.mtf`.

### Synthetic compile-time interfaces

Synthetic interfaces enter through ordinary dependency binding. They contain stable member identities, folded backend-neutral values, source locations, member fingerprints and provenance, with no AST, HIR or runtime body.

The build-owned `@project` interface publishes the predefined project field set, not helper constants or `$config` declarations. AST consumes values and provenance but never owns bootstrap or namespace policy. The build document defines that field policy and package isolation.

## Fingerprints and reuse facts

Each successful base module has five distinct fingerprint domains:

| Fingerprint | Contents and exclusions |
|---|---|
| Public interface | The complete canonical `PublicSemanticInterface`. Excludes private bodies, source locations, warnings, formatting metadata and non-public dormant root work. |
| Implementation | Executable body semantics and non-interface implementation facts, including exported bodies whose public facts stay unchanged. Excludes dormant root work and generated sidecar bodies. |
| Dormant root activity | Implicit `start`, top-level runtime work, fragments and folded root-local entry metadata. |
| Runtime dependencies | Backend-neutral calls, helpers, capabilities, target-gated operations, glue requirements, resource uses and site-root requirements of executable functions/root work. |
| Documentation | Public documentation, editor metadata and API-index data. |

Generated requests come from the active specialised AST. They are materialisation dependencies carried with link data, not runtime-dependency fingerprint contents. A changed request set follows implementation invalidation and updates the generated sidecars. Sidecars retain their own implementation, runtime and compatibility fingerprints.

Configuration values and `NumericProfile` participate in whichever existing domains they affect. Configuration may change folded values, executable behaviour and derived public, root or link facts while preserving declaration/export existence and declaration-origin identity. Provenance survives folding. Structural `$feature` selection requires separate selection-compatibility facts, not ordinary configuration folding.

Private implicit-failure changes use the existing implementation, dormant-root activity and runtime-dependency facts they affect. They create no new fingerprint domain.

Resource bytes and rendered URLs are not semantic-origin identity. `docs/build-system-design.md` > `Incremental and persistent artefacts` owns recompilation, relinking and cache policy. Its resource section owns content/output invalidation.

## Generated concrete functions

Concrete generic functions live in generated sidecars owned by the consuming project or package boundary. Base module artefacts remain immutable.

A request key contains the stable generic declaration, canonical concrete type identities and required evidence identities. The declaring module validates and publishes its immutable generic template and retained materialisation context. A consumer emits durable requests only from its active specialised AST.

The compiler canonicalises and deduplicates requests against an immutable view of published instances and work completed within its transaction. It owns materialisation, HIR validation, borrow analysis, lifetime/escape analysis, call-summary installation and semantic convergence. The build system owns the published set, sidecar storage, placement, reuse and atomic publication with the requesting module.

Each sidecar owns its concrete HIR, generated-local type environment or immutable canonical-to-local delta, borrow/lifetime facts, complete effect summaries, link facts, fingerprints and its own numeric proof table computed from its HIR under the boundary profile after validation. It neither borrows the requester's mutable type environment nor extends the declaring dependency artefact. Its compatibility includes the boundary `NumericProfile`.

Nested requests, including cross-module and cross-package requests, converge inside the requesting compiler transaction using the declaring artefact's retained context. They never create another source-module job or alter provider-wave ordering. Build code does not filter requests later or rerun analyses to complete them.

The module outcome contract governs generated failures. A diagnosis discards the transaction's unpublished module/generated result. Operational failures and compiler bugs follow their distinct lanes. Already published independent artefacts remain immutable.

## Frontend stages

Stage 0 belongs to the build system. It selects graphs, source sets, provider order and command roots. It may schedule Stage 1 tokenization and Stage 2 header-syntax preparation before provider interfaces exist. Interface binding and subsequent local semantic stages run inside the canonical module service. Stage descriptions are ownership contracts, not independent build-side entry points.

### Stage 1: tokenization

Tokenization recognises lexical forms and records exact source locations, string/template delimiter context, numeric spelling, operator/assignment spacing and registered directive syntax. Retained tokens belong to their source. Later consumers use bounded ranges and typed handles rather than cloned token vectors.

The neutral numeric spelling and destination-aware materialisation service belongs to `moth-lexical`. Tokenization retains source-local spelling so the compiler can select that service for its receiving type, rather than assigning every literal an `i32` or `f64` value early.

Frontend directive names are always present. Builders extend one registry without overriding them. Tokenization and template parsing use that registry while retaining their distinct lexical modes. A descriptor records ownership, permitted context, form, signature, processing phase and effects. It does not grant arbitrary callbacks into semantic stages. Recognition distinguishes unknown names, wrong placement and unavailable capabilities.

Ordinary `.moth` starts in code mode. `.mtf` starts in an implicit template body while preserving original locations. Plain Markdown `.md` is prepared before tokenization and has no tokenizer entry mode. Tokenization does not resolve dependencies, types or declarations.

Structural `$feature` selection must exclude source before contained directives are resolved or graph/declaration facts are published. Its predicate grammar and lexical mechanism, including directive-controlled template body modes, remain open design. Ordinary static `if` provides no such exclusion.

### Stage 2: header syntax and interface binding

Header work separates provider-independent syntax preparation from binding against completed interfaces. A preparation lane is a source-processing context, such as canonical compilation or a separate check-only unit. It scopes syntax reuse without creating another physical source identity.

#### Header syntax preparation

Preparation is the sole owner of module-wide top-level declaration discovery. `PreparedHeaderSyntax` retains:

- tokens or source-kind payloads and source locations
- dependency/re-export clauses, direct selections and aliases
- declaration shells for constants, functions, structs, choices, aliases, traits and conformances
- declaration modifiers, including `$config` input-contract shells
- root metadata/purpose invocations and balanced argument ranges
- dormant start-body separation and compile-time fragment placement information
- structural provider references and structural file references
- conservative local declaration-ordering hints
- diagnostics and identity/remap data

Source-kind adapters may synthesise ordinary declarations. API-only root roles reject authored root runtime/output activity. Normal roots retain the corresponding dormant work and purpose facts.

Preparation neither checks executable bodies nor folds expressions. It retains directive arguments without parsing a parallel argument language. The shared argument owner parses them semantically once the receiving context is known. File classification reads the tokenizer's dense path rows, not surrounding expressions.

Input-contract shells do not create provider edges. Config compilation precedes source-graph construction, so an authored file-value path in `config.moth` is diagnosed rather than resolved into a graph edge.

#### Interface binding

Binding resolves retained clauses against immutable provider interfaces and produces `BoundModuleHeaders`: stable origins/export bindings, canonical types and folded values, final file-local visibility, source and binding-package namespaces, receiver surfaces, prelude/builtin reservations and completed collision results.

Provider-created binding interfaces may be available before source compilation. Source providers must compile first. Binding never copies provider declarations into a consumer, reparses source or bypasses a facade. It passes resolved file-reference facts to AST without interpreting their value meaning.

#### Four reference classes

Keep these facts distinct:

| Fact | Consumer and purpose |
|---|---|
| Structural provider reference | Stage 0 provider-graph construction. |
| Structural file reference | Stage 0 content-source membership and physical input resolution. |
| Dependency symbol binding | File visibility and AST semantic use. |
| Local declaration-ordering edge | Stage 3 ordering within one module. |

A dependency clause may produce a provider edge and later a symbol binding. Its consumed path row is never also published as a structural file reference. A dependency-bound declaration is never a node in the consumer's local declaration graph.

Local edges cover alias targets, field and signature types, explicit constant types, constant initialisers, fixed capacities, structurally visible const-template controls and trait/conformance references that require local ordering. Declaration-shell parsing is shared with body-local declaration parsing where the syntax is equivalent.

#### Source-kind adapters

An `.mtf` source contributes one private synthetic `content #String` declaration. Its initialiser is a structurally built `$md` template over the original body tokens. Nested templates inherit Markdown formatting unless an explicit directive overrides it.

Plain `.md` renders raw Markdown to HTML and contributes the same declaration shape with a synthetic string-literal initialiser. Ordering and AST folding then treat both as ordinary constants. Neither creates a separate AST, HIR, borrow or backend pipeline.

A recognised source kind unsupported by the active builder receives a typed dependency diagnostic. Resolution never falls through to another extension candidate.

#### Direct Moth template service

The direct service reuses tokenization, synthetic-header preparation, local ordering and AST folding. It extracts folded `content` and warnings, then stops before HIR, borrow/target validation, lowering and output writing.

For a standalone source it owns preparation through folding. For a Stage 0 content bundle it consumes the retained preparations without repeating them. Tooling supplies inputs and receives owned folded values, never assembling raw stages itself.

The service owns the bundle's source database and live span builders through folding/extraction. It finalises tables after AST readers end. If discovery fails first, the preparation owner finalises its known snapshots at that terminal boundary. Aggregation retains each document's warning context even when a later document fails. Private discovery normalises source identities once before success or diagnosis, as specified by the data-layout authority.

#### Project config compilation service

One named compiler service prepares the single authored config source, binds its permitted compiler surface, orders declarations and performs AST checking/folding. It consumes the boundary `NumericProfile` and stops at folded values, with no HIR, borrow facts, link facts or public module interface.

It returns typed folded directive arguments, input-contract results, original key/argument locations and diagnostics. Build-owned config consumers apply the settings. They do not scan recognised record names or compose compiler stages.

The config outcome returns its live source-local span owner on success and diagnosis. Build validation may still produce spans during config application and output-path checks, so that owner freezes only after those consumers finish. Both short services preserve the same source/identity contracts as canonical compilation.

### Stage 3: local declaration ordering

Stage 3 topologically orders retained local declaration shells, diagnoses cycles and preserves source order among independent declarations. Stage 0 has already ordered providers. Stage 3 appends builtins and dormant normal-root `start` after declarations.

Same-file constants retain source-order semantics and reject forward references. Cross-file constants use header-provided local edges. Cross-module constants are already folded interface facts. Directive retention changes none of these rules.

A concrete required edge with no local declaration is a graph diagnostic. A conservative symbol-shaped hint that cannot be identified as local may reach AST for a precise type/expression diagnostic. Unresolved hints do not all become missing-header errors.

AST consumes the resulting declaration order linearly. Local nominal identities are registered before alias targets resolve. Completed member shells follow the alias targets and capacity constants they require. An alias that folds a module constant is published at its own ordered position, never provisionally.

Stage 3 does not order modules or body-local declarations, inspect executable body references or copy dependency symbols into its graph. Dormant `start` is not a dependency participant. Missing local edges belong in header preparation. Missing providers belong in Stage 0.

### Stage 4: AST semantics

The typed abstract syntax tree (AST) stage consumes sorted shells and bound visibility. It resolves declarations and canonical types, parses/checks executable bodies, applies receiving-context rules, folds constants/templates, validates public surfaces and prepares active executable work for HIR.

Its owners cover body-local declarations, generic inference/evidence, traits/conformances, casts, file values, Wiring contracts, TIR and folded root metadata. Root metadata enters the non-HIR lane through ordinary visibility, never through downstream constant-name scans.

The semantic order is:

```text
parse and type-check complete authored bodies
-> fold constants and final compile-time expressions
-> specialise known-Bool if branches
-> derive active generated requests and executable summaries
-> validate active terminality
-> hand the completed active AST to HIR
```

This specifies dependencies, not a fixed count of internal passes. `Static Bool control-flow specialisation` defines exactly what remains validated and what selection removes.

The frontend owns one implicit-failure fact on expression and function summaries. Checked numeric failure is implicit built-in failure, delivered by the enclosing handler or function contract without wrapping or an implicit panic. Private functions without an error slot infer and propagate it inside the same module without postfix `!` at each internal call. This inferred failure is an internal lane, not a public effect row or a third error channel.

Built-in `Error!` materialises that failure as an `Error` value. A custom `E!` does not: its body needs local recovery or explicit mapping. Export validation requires callables without an error slot to discharge bare failure before publication and diagnoses those that do not. Handler selection and typed-error compatibility stay frontend-owned.

Statically known invalid numeric work remains a source diagnostic, including inside `Error!` and `catch`. Compound write-back checks conversion to the destination separately and writes only after success.

#### Dependencies and visibility

AST validates uses through bound file visibility. It never rebuilds dependencies or top-level visibility. One collision policy covers declarations, dependency clauses, aliases, prelude symbols and builtins, with no silent shadowing.

A top-level lookup that cannot work from sorted declarations and bound visibility reveals a missing preparation, binding, ordering or graph fact. It does not justify another discovery pass.

#### Type checking and coercion

Expressions have natural semantic types. Only an explicit receiving-boundary owner applies contextual coercion. These boundaries include declarations/assignments, returns, concrete parameters, fields, defaults, typed collection/map entries, string/template content, explicit cast targets and `then` arms with a known receiver.

Compiler and binding-backed calls use the same rule. AST carries `TypeId` through fields, receivers, calls, operators and compatibility checks. A receiving annotation does not retroactively change an already typed operator result.

#### Typed semantic expression ownership

The AST stage constructs a typed semantic expression IR while parsing, resolving receiving contexts and folding. It does not add an untyped whole-program AST or defer these decisions to HIR. A completed node records its resolved operation, ordered child IDs and one result shape. The safe operation enum remains beside the result in an array-of-structures row. `SuccessShape` is explicitly `Zero`, `One(TypeId)` or `Multiple(TypeRange)` with at least two entries; the optional error type is separate. An optional `none` is one optional-typed result, a missing default is distinct from zero results, and divergence remains a control-flow fact. Resolved numeric domain, conversions, cast evidence and failure disposition remain owned facts for folding and HIR lowering to consume. Folding receives the canonical result type and resolved operation directly rather than deriving semantic types from diagnostic spelling.

Separate these responsibilities at their real owners:

| Owner | Retained facts |
|---|---|
| Parser and receiving-context scratch | Unresolved numeric spelling, operator stacks and incomplete construction while their immediate context is being resolved. These states do not escape as completed semantic expressions. |
| Declarations and symbols | Declaration identity, visibility, binding mode, receiver identity and links to any real initialiser. A declaration without an initialiser does not manufacture a `NoValue` expression. |
| Callable and constructor contracts | Resolved parameter types, access/capability requirements, defaults, receiver information and success/error signature. Parsing, inference and validation borrow the same contract and retain one argument-slot mapping. |
| Semantic expression store | Resolved value-producing operations, compact graph edges and authoritative result shapes. Zero results remain a real shape, distinct from a missing default. |
| Constant owner | Immutable value content and its resolved classification, including exact numeric values and structural strings. Repeated uses refer to content without expanding it into owned expression subtrees. |
| Occurrence and cold facts | Authored locations, diagnostic evidence, failure contributors, catch eligibility, const-record classification and synthetic-interface provenance where required. Shared constant content does not absorb occurrence-specific evidence. |
| TIR | Template structure and exact-view authority during AST semantics, under `Templates and TIR`. |

The common expression record does not carry declaration-only fields, cloned diagnostic `DataType` trees, absent-default sentinels or rare feature payloads. Underlying semantic types remain `TypeId`s. Authored type syntax stays with retained declarations and source ranges. Diagnostic compaction captures needed display facts before their type owner can be released, under the data-layout authority. Shrinking or dropping the expression store does not release the paired diagnostic `TypeEnvironment` or `DiagnosticRenderContext`. Retain their current dependency through all existing render and report readers until the separate diagnostic snapshot migration establishes a new release boundary.

Cold facts retain their semantic meaning through folding and replacement. An empty contributor list is not by itself proof that all facts are absent: catch eligibility, typed producer conflicts, explicit exits and deferred assertion/catch evidence can still matter. Failure summaries do not repeatedly rescan expression subtrees to reconstruct origin lists. Facts needed after expression storage ends move into the consuming HIR or diagnostic owner first.

One `AstPhaseContext` owns the expression store paired with its module-local `TemplateIrStore`. Dynamic TIR expressions and overrides refer only to committed IDs in that expression domain. Each concrete generated AST build and each config or direct-template service has its own pair. No expression ID crosses those domains.

Store domains follow actual lifetimes. TIR-referenced expressions stay available for every exact-view consumer in their module build. Do not compact or remap expression IDs while any TIR view can still read them. After TIR's final structural, nested-value, folding, runtime-handoff and required census/debug readers finish, drop the TIR store and compact the completed AST graph from its surviving roots and owned default/metadata dependencies. Remap reachable nodes and typed side ranges into exact-capacity storage and release dead or replaced nodes only at that boundary. Dropping a view alone does not reclaim append-only entries.

Generic-body validation roots remain available through finalizer specialisation, terminality, failure-boundary and any TIR readers, then leave completed AST roots. The immutable generic template retains canonical syntax and context separately, from which a concrete body gets a distinct local graph. Candidate specialisations share unchanged immutable data without leaking replacements between candidates.

Module compilation, generated materialisation, config and direct-template services consume the same semantic builders and fold owners through their defined entry points. Each service releases its scratch and expression stores after producing the owned values, executable data and diagnostic context needed by its outcome. Public interfaces contain canonical value/type facts, never expression IDs. HIR receives a deliberate projection and retains no AST/TIR authority.

#### Result slots and receiving boundaries

An expression produces zero, one or multiple ordered result slots. The common zero/single representation is allocation-free. A multiple-result payload contains at least two slot types. There is one authoritative resolved result shape per expression, not a duplicated call-local return list.

An optional `none` is one result. Zero results do not imply divergence. Terminality remains a separate control-flow fact. A single collection or record is also one result, not a way to transport multiple returns.

Single-value consumers validate exactly one slot. Multi-bind, returns and value-producing control flow validate arity and coercion per slot. A producer executes once regardless of result count. Folded multi-result producers use ordered ordinary single-valued constants through the same receiving path, without a synthetic tuple type or an external-only value hierarchy.

All right-hand results are computed before assigning existing multi-bind targets. Distinct result slots preserve result-to-parameter and result-to-result provenance. They do not prove separate allocations or pairwise disjointness, including when a summary classifies results as fresh.

Source misuse receives a typed arity/type diagnostic at its receiving site. A completed semantic node with an invalid shape violates a compiler invariant. No consumer silently selects the first slot, substitutes an optional `none` for zero results or manufactures a tuple to fit a single-value API. Physical result packing belongs only to ABI owners.

#### Numeric typing and literal materialisation

AST consumes the boundary `NumericProfile` for `Int`, `Uint` and `Float`. Fixed-width types, `Byte` and `Dec` scales are profile-independent.

Numeric literals retain source-local spelling until a destination is known:

- An unconstrained whole literal defaults to `Int`. Decimal/exponent spelling defaults to `Float`. Small values never infer a narrow type by size.
- A direct typed receiver materialises a lone literal of the requested numeric type without an `Int` intermediate. A `U64` literal can therefore exceed the default `Int` range.
- A concrete `Dec` or `Uint` receiver additionally types unresolved literal arithmetic before operator typing and folding. The `Dec` scale flows down raw arithmetic and parenthesised groups to pending literal leaves, which materialise directly at that scale; an explicit `Uint` receiver reaches unresolved whole-number leaves the same way at the selected unsigned width. Without an explicit receiver, a typed `Dec` peer supplies its scale to a wholly literal-only opposite subtree from either side, while a local typed `Uint` peer supplies `Uint` the same way. Only `Dec` and `Uint` peers extend to compound literal subtrees, with different exponent rules: `Dec` context never crosses a power exponent edge, a cast operand, a call argument, a comparison or logical operator, a typed suffix needing a completed value, or an unresolved generic slot, while a raw `Uint` exponent receives `Uint` context too.
- Whole literals may initialise binary floats, `Uint` and the `Dec` family. Decimal/exponent spelling never initialises an integer, `Uint` or `Byte` merely because its mathematical value is integral.
- Binary float literals round directly to the destination using round-to-nearest, ties-to-even. Inexact finite values are valid. Rounded non-finite results fail. Subnormals and signed zero are preserved.
- Signed integer minima materialise with their sign without first rejecting the positive magnitude. Negative values cannot initialise unsigned types or `Byte`.
- `Dec` uses its exact decimal scale rule, not binary literal rounding. Each leaf must fit the selected scale exactly. A `Uint` leaf must fit the selected unsigned width.

An immediate concrete numeric peer may type an otherwise untyped lone literal before operator promotion. A literal outside that peer's range is diagnosed. A receiving annotation does not retrospectively retag an already typed operator result. Typed operands keep their identities. Generic inference uses these local rules, without distant conversion search. Parentheses preserve grouping without adding a receiving boundary.

Groups retain their infix syntax as pending fragments until ordering flattens them into one RPN stream. Destination selection runs on the ordered stream before literal materialisation, materialisation runs before operator typing and operator typing runs before named-constant substitution and folding. Completed AST and HIR retain resolved semantic values only. Neither pending form survives there.

Materialisation never routes through a fixed `i32`/`f64` bottleneck. Operator promotion, comparison and cast eligibility remain separate decisions. The canonical numeric and cast references own their matrices and checked result domains.

#### Core external constant evaluation

A binding may advertise a compiler-owned typed constant-evaluation operation beside its backend lowerings. AST dispatches that operation through the resolved stable binding identity into compiler-owned Rust evaluation. Package spelling, helper names, arbitrary callbacks, JavaScript execution and source purity annotations are not dispatch mechanisms.

V1 permits trusted Core-origin registrations only. Parameters use shared access, success slots have the accepted fresh-result classification, no error channel exists and the complete signature matches the operation's supported value contract. Ordered multiple results and supported optionals follow ordinary result-slot rules. Opaque handles, unresolved types and unsupported value representations are ineligible. An invalid trusted registration or evaluator result is a compiler bug, not a fold refusal.

Required and opportunistic folding share one evaluator and ordinary constant owners. Normal visibility, argument order, access checks, arity and source handling syntax are validated first. Evaluation requires genuine compile-time inputs with the concrete data the operation observes. Shared access alone is not constness.

Every output slot retains its semantic type and conservative input/config/project provenance. Exported constants publish completed values. Consumers never rerun a provider's evaluator. Folding an ordinary runtime declaration does not turn it into a source constant declaration.

A fully folded call contributes no runtime call, helper or asset requirement, but keeps its semantic input dependencies. A visible Core call may fold even when the selected target lacks its runtime lowering. A surviving runtime call still requires that lowering, and folding never makes an unavailable package visible.

A missing evaluator or runtime argument produces a required-constant diagnostic where constness is required and retains the runtime call otherwise. The same distinction applies to a bounded evaluation refusal. An observing operation on an unresolved structural resource string cannot guess its characters: required folding diagnoses it, while opportunistic evaluation keeps the runtime operation. Checked-value failures retain the language's established compile-time failure policy rather than silently becoming runtime work.

Operations use deterministic work/output limits and checked capacities through the shared constant-evaluation owner. Refusal carries a typed reason and produces no partial output. Optional absence remains an ordinary successful value, distinct from refusal, zero results and an error channel. Fallible calls, mutable execution, arbitrary source-function interpretation and user-selected evaluators are outside V1.

The closed operation vocabulary contains only registered operations with real consumers. Registration checks the complete signature before installation, and evaluation validates every returned slot. Provider JavaScript, Builder and Dependency registrations cannot select compiler-owned Rust evaluation. A runtime call may retain individually folded arguments without losing their evaluation order or effects.

#### Call-shaped syntax and assertion intrinsics

One shared argument owner parses delimiters, commas, multiline whitespace, positional/named entries, mutable-access markers and expression boundaries. Signatures route values to parameter slots and retain that mapping for default, type and access validation. Anonymous records use named-only field routing instead. No consumer reroutes the source list or manufactures a callee signature merely to parse a record.

Template-specific slot labels, indices and helper categories retain their own meaning over this shared list syntax. Directive signatures require compile-time arguments, reject escaping/mutable evaluation and distinguish standalone invocations from declaration modifiers. Builders cannot turn the shared parser into an arbitrary source-transform API.

`assert` is a reserved statement intrinsic with compiler-owned expectations equivalent to:

```moth
assert |condition Bool, message String? = none|
```

These expectations create no importable, shadowable or first-class function. Shared call validation handles argument shape, defaults, types and access. Assertion semantics handle placement, completed-statement suffix rejection and the prohibition on escaping message evaluation through implicit failure, `!`, `?`, `return`, `break` or `continue`.

A handled fallible expression retains its ordinary call/value location separately from the authored propagation-operator location. Call side-table mapping remains call-owned while escape diagnostics identify the operator.

The completed assertion carries a typed condition and typed optional message. Omission uses ordinary typed `none`. Even for a compile-time `true` condition, the message is parsed, type-checked, generic-inferred, evidence-checked and fully normalised, including nested templates. Finalisation then replaces it with typed `none` and discards provisional message requests. No inactive TIR, runtime payload, Wiring capture, generated sidecar or link/target fact survives from that message.

A compile-time `false` or dynamic assertion retains message work only on the failure edge. Stage 5 defines its lazy lowering and target validation checks capability without silently discarding evaluation.

#### Value-producing blocks and terminality

Value-producing `if`, match and block-form `catch` are closed receiving constructs, valid where a receiver is explicit. Every producing path satisfies its required slot arity and types.

AST owns receiving-context and terminality diagnostics. A function requiring success results cannot fall through without producing them. A completed AST violating that rule is a compiler bug at HIR handoff. Both branches of a value-producing `if` satisfy completeness before static selection. Runtime conditions retain all-path rules.

#### Constants, build configuration and const records

Header preparation discovers constant dependencies. AST checks and folds them. `$config` marks one following explicitly typed top-level compile-time binding as an input contract. Project fields and ordinary helpers never supply or block inputs implicitly.

Bootstrap defaults fold within the config service. Source defaults remain self-contained primitive literals or `none`, resolved after graph discovery and before module AST semantics. The resolved value is an ordinary folded constant, with no runtime wrapper, HIR category or visibility rule. The build document owns the exact domain, precedence and matching contract.

A module folds each ordinary constant and const template once. Exported facts are owned backend-neutral values. Consumers do not parse or fold provider templates again. Advisory private const facts change neither declaration semantics nor ordering/visibility.

Fully folded structs and anonymous records may form const records. Anonymous const records use named-only `(name = value)` entries and permit an empty record. Fields may contain inline nested anonymous values, bound values and explicit nominal constructions. They declare no nested types and imply no conversion. Whole const records are field-access-only compile-time groups, not runtime objects that can be passed, returned, stored at runtime or used through runtime methods.

#### Runtime anonymous records

A runtime anonymous record uses an ordinary hidden nominal struct, distinct from a const record. Each static literal site has one type per owning compiled body, qualified by module/body and concrete materialisation identity where needed. Repeated execution reuses that type. Different sites remain distinct even with identical fields. Copies and aliases preserve identity.

Resolve nested children before registering ordered parent fields. Hidden anonymous parents may contain hidden children, including an existing child value. This exception does not allow hidden types in authored nominal fields, function arguments/returns, exported surfaces, collections/maps or generic arguments. Check restrictions recursively through parents, optionals and captures. An extracted ordinary leaf retains its normal uses.

Empty runtime records are invalid. Constant-looking fields alone do not select the const-record path. Ordinary struct construction, projection, copy, access and lifetime rules apply, with no anonymous-specific HIR/backend node or structural type unification. See `docs/src/docs/structs/anonymous-records.mtf` and `docs/roadmap/plans/runtime-anonymous-records-plan.md` for the full local-use contract.

#### Static Bool control-flow specialisation

Ordinary statement and value-producing `if` use the normal folded Bool authority, with no config-specific branch node. Selection follows full name, visibility, type, generic-evidence, cast, constant-expression and value-production validation of both authored branches.

A known `true` selects the `then` branch. A known `false` selects the `else` branch or an empty scoped result when absent. Preserve the selected lexical scope. Runtime or unknown conditions remain ordinary `if`.

Terminality and durable executable summaries use the active specialised AST. Inactive code publishes no generated request, HIR, effect, project-context, link or target fact. Every `if` remaining in HIR has a runtime condition. This rule adds no match folding, loop unrolling or general control-flow partial evaluator.

Structural references were already graph-active before selection. Frontend errors in inactive ordinary branches remain errors. Structural `$feature` exclusion is a different earlier contract.

#### Generics

The declaring module validates immutable generic templates. At a call, AST infers concrete arguments from immediate call arguments and expected result context, resolves visible trait evidence and diagnoses failures at that call site.

`ModuleMaterialisationPreparation` is retained separately from the executable `Ast` and its expression store. Before HIR lowering consumes the `Ast` or that store is released, project the generic body to stable owned syntax and capture canonical owned callable signatures, receiver/type facts, folded defaults and module constants. The preparation contains no expression IDs, expression views or borrows into the completed expression store. Keep it through same-module generated requests and summary convergence. Freeze the stable generic context after those readers finish, then release preparation.

Active calls emit requests under `Generated concrete functions`. HIR and backends receive concrete targets, never unresolved generic inference or evidence work. Generic materialisation preserves result-slot, numeric-profile, source-identity and Wiring contracts.

#### Traits, conformances and casts

Headers retain trait/conformance shells. AST resolves requirement types, stable trait identity, explicit conformance, evidence visibility, bounds, bound-provided receiver calls and incompatibility diagnostics. Exported evidence uses stable identities rather than structural reconstruction.

Traits are compile-time contracts, not value types. Static bound calls become concrete executable targets before HIR. No trait object, erased dispatch or runtime evidence crosses that boundary.

Cast evidence proves that an explicit source/target conversion is permitted. Builtin conversions use compiler-defined policy. Source-authored evidence comes from explicit conformance of an eligible same-file nominal source type to a compiler-owned cast trait, together with its required receiver method. Matching method shape alone is insufficient. Fallible/infallible evidence for the same target remains mutually exclusive.

AST resolves evidence and folds permitted conversions. Builtin runtime conversions become explicit HIR cast operations. Source-authored evidence becomes an ordinary direct source-function call before or during HIR lowering. Evidence metadata itself never enters HIR.

Contextual coercion remains separate from explicit casting. User-defined cast targets, generic target parameters, opaque foreign targets and a general user conversion framework are not implied. The canonical cast references own eligibility, handling syntax and the numeric target table.

#### Templates and TIR

AST owns all template semantics. Template IR (TIR) retains template structure while composition, formatting and folding are resolved. A view selects effective reads through module-owned override layers called overlays.

One module AST build owns one `TemplateIrStore`. Parser emission writes text, expressions, child templates, slots, inserts, wrappers and control-flow roots directly into that store. All TIR IDs are module-local typed IDs.

`Template` is a thin handle carrying a durable TIR reference and source location while AST construction is active. The reference contains a module-local root, the root phase and a value-carried `TemplateViewContext`. It is not a registry handle.

The phase sequence is:

```text
Parsed -> Composed -> Formatted -> Finalized
```

Folding requires `Composed` or later. AST-to-HIR handoff requires `Finalized`.

An exact `TirView` is the structural read authority after parser emission. Its `TirViewIdentity` contains the module-local root, phase and complete value context. The context contains only the optional expression, slot-resolution and wrapper-context overlay IDs, whose payloads remain owned by the module store. This identity determines effective reads, preparation and fold-cache keys and cycle detection. Consumers do not carry parallel authority tokens, store identities, context tables or overlay stacks outside the view.

Recursive consumers use two explicit view transitions:

- Structural child and wrapper transitions preserve the current complete expression overlay. Parsed references ignore their referenced slot-resolution and wrapper-context overlays. Composed or later references supply those two structural dimensions. Resolved slot sources and structural helpers retain the complete current context.
- Nested-value transitions enter an independently owned nested `Template` through that value's complete context rather than inheriting the containing structural root's expression overlay.

A composed or finalised root overlay contains effective overrides for every structural descendant reachable through children, wrappers, resolved slots, branches, fallbacks, loops and helper roots. Expression lookup uses that complete overlay followed by structural fallback.

One semantic preparation owner:

- validates every required reachable TIR root, node, overlay, wrapper and slot plan
- follows structural and nested-value transitions
- detects cycles by exact view identity
- classifies the value as foldable, runtime or helper
- preserves lazy runtime semantics
- treats missing authority as a compiler bug

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

Root-local page and documentation metadata folds through this AST path into `ModuleCompilerMetadata`. TIR does not cross that boundary, and HIR does not recover metadata by scanning constant names.

No TIR store, ID, view, overlay, preparation type or registry crosses into a completed module, public interface, HIR or backend.

Missing roots, phases, overlays or exact-view authority are compiler bugs. Template meaning is never reconstructed from a second representation.

Dec formatting uses the common value-to-string path. It does not add Dec-specific TIR nodes.

The `$md` formatter owns automatic heading IDs. It derives each heading's `id` during formatting from the heading's complete static visible label, and omits the `id` when the label contains a dynamic expression or child template. IDs add no TIR node, anchor kind or metadata, and builders never scan rendered HTML to recreate them.

#### File values and resources

A dependency clause binds declarations or a namespace and produces no value. An explicit-extension path in expression position is a file value of language type `String`. Each path occurrence has one of these owners.

Tokenization retains dense path rows. Header preparation classifies rows outside dependency-clause ranges without parsing surrounding expressions:

| Spelling | Retained meaning |
|---|---|
| Bare `@/` | Site-root string piece, with no file edge. |
| Explicit `.mtf` or `.md` | Content-source reference. |
| Explicit `.moth` | Diagnostic-only `SourceKindNoFileValue`, with no physical target. |
| Another explicit extension | Resource-file reference. |
| Extensionless non-dependency path | Left for AST's semantic diagnostic, with no physical target. |

Stage 0 resolves each physical-target-bearing reference once and publishes its outcome. All retained occurrences are classified even when the containing expression later fails syntax validation. That later error does not retract graph/input facts. Diagnostic ordering keeps a speculative missing-file error from displacing the primary syntax error.

`SourceKindNoFileValue` never causes target resolution, semantic-source membership, a byte-source record or a watch interest. AST reports `MothFileHasNoValue` regardless of whether a matching file exists. The same rule applies to another recognised source kind with no file-value semantics.

Ordinary static `if` keeps both branches structurally selected. A physical reference remains graph-active even when AST later removes its executable use. Referenced content is prepared and validated regardless of the consuming expression's eventual liveness. The canonical file-path reference owns the complete list of authored positions. The build document owns the discovery worklist and physical containment policy.

AST's file-value resolver receives the source-owned path identity and its resolved entry, never a filesystem resolver. It interprets:

- `.mtf`/`.md` as the existing synthetic `content` constant, with no second parser, renderer or observable filename
- an ordinary resource as a stable semantic origin plus an authored use location and one structural Resource piece
- `@/` as one SiteRoot piece, with no resource identity
- `.moth` as the typed no-file-value diagnostic

Direct content constants use retained local ordering edges. Actual content-dependency cycles are diagnosed by declaration/source ordering, not an AST recursion guard.

A structural string is an ordinary `String` whose ordered pieces include Text, Resource or SiteRoot. The anchor pieces preserve identity until the builder supplies final characters. Such strings remain legal mutable values, parameters, returns, optionals, collection elements and exported constants. Their presence alone causes no rejection.

One module-local folded-value owner supports the plain-text fast path and structural pieces. Its postorder visitor is the sole recursive conversion route into public values, HIR constants and direct-template results. Consumers never reconstruct the string from AST expressions or TIR.

Composition preserves pieces through assignment, copying, storage, interpolation, concatenation, slots/wrappers, export/re-export and calls/returns. Observation needs concrete characters: equality, length, search/prefix/suffix tests, parsing, String casts, hashing, compile-time map keys/duplicate checks and character-inspecting formatters or host operations.

A required compile-time observation of unresolved pieces is diagnosed. Partial symbolic equality, partial hashing and guessed URLs are invalid. Opportunistic Core constant evaluation may instead retain the runtime operation under its explicit refusal contract. Runtime semantics are ordinary string semantics because the selected physical variant materialises the pieces before execution observes them.

Direct file values in templates become TIR Resource or SiteRoot nodes without intermediate text. They produce output and carry no Wiring dependency. Formatters treat them as opaque anchors. Every exact-view traversal, subtree copy and identity remap preserves their stable meaning. Local IDs may be reassigned, but an omitted structural case is a compiler bug.

The direct `.mtf` service has no containing output artefact or route. It returns owned structural content and resource-source facts. Plain-text extraction succeeds only when no unresolved piece remains. Frontend code never invents a URL context.

#### Wiring boundary

Wiring has two constrained parameter contracts. A **wire** preserves a binding instance's continuing identity. A **constructor route** records how future input completes a known choice constructor. These routes are unrelated to HTML URL routes. Messages remain ordinary choice values.

`of` is head-directed: the preceding compiler-known head determines whether the specification describes an ordinary generic type, a wire parameter or a constructor-route parameter. Sharing syntax adds neither user-defined specification grammars nor first-class capability containers. `docs/src/docs/wiring/wiring.mtf` owns the exact Wiring source contract and its channel boundary.

A V1 `Wire of T` parameter accepts a named local binding or a compatible existing wire parameter. An ordinary local acquires identity only when a wire-receiving call needs it, with no `Wire()` constructor or call-site marker. Mutable and immutable locals are eligible. Projections, indexed places and fresh/computed expressions are not. An ordinary `T` receiver reads the current value under normal access rules. A compatible wire receiver forwards identity. An ordinary value parameter does not preserve its caller's identity merely because its body could use one.

Identity belongs to the binding instance, not its contents, address or source position. Reassignment preserves an eligible local's identity. Separate calls or loop executions create distinct instances where ordinary binding semantics do so. Assignment from a wire reads an ordinary value, not a new wire capability or a deep copy.

A wire grants no exclusive authority and is not an active long-lived value borrow. Actual reads, captures and mutations obey ordinary borrow validation. Identity never proves storage outlives a consumer or legalises an escape.

V1 constructor routes have zero or one future input. At an immediate route-receiving boundary, a direct choice variant/application may capture a contiguous leading payload prefix. Future input supplies the remaining trailing payload in declaration order. Captures are ordinary expressions evaluated once at formation, with normal evaluation, error and ownership rules. They are not saved expressions to rerun later. A fully supplied zero-input route retains ordinary complete-constructor argument rules.

An unchanged compatible route parameter may be forwarded. Exact input/output types and ordinary concrete generic inference apply, without callable variance or first-class constructors. Outside this receiving boundary, incomplete construction remains an error. A route is neither a destination, an event registration nor an implicit send.

Wire/Route capabilities are not ordinary stored or returned values. V1 capabilities have no field/choice-payload storage, collection/map storage, capability aliases, defaults, capability comparison, serialisation or channel transfer. Routes have no general application/composition, and a function name, closure or arbitrary expression cannot replace a direct constructor route. Returning an ordinary `T` read from a wire remains an ordinary value return.

AST resolves parameter contracts, binding identity, constructor identity and capture semantics. HIR retains the minimal neutral facts and ordinary captured values. Public interfaces and generated materialisation preserve contract kinds and canonical underlying types. Target validation rejects a reachable contract it cannot realise rather than silently reducing it to value-only behaviour.

The normal argument owner knows the immediate receiving parameter before validating constructor completeness. It never retries a failed ordinary constructor as a route. Capability arguments use the same typed expression, native result-slot and ordinary captured-value owners as other calls. Static expression IDs are not runtime binding-instance identities. Public signatures, fingerprints, imports and generic remapping preserve the distinction between an ordinary value parameter, a wire and a route without exporting local identities or capture storage.

Templates remain ordinary strings with no hidden wire, route, subscription or rerender recipe. Structural resource pieces follow their independent contract. Persistent observation, event delivery, UI scheduling and route application require separately defined consumer/lifetime contracts. This foundation supplies none implicitly.

### Stage 5: HIR and validation

High-level intermediate representation (HIR) is the first backend-facing semantic IR. It lowers completed typed AST and concrete generated functions into explicit control flow, places, locals, calls and effects. Each module or generated body retains its paired type environment. Cross-module calls use stable targets without copying callee bodies into callers.

A HIR `RegionId` identifies the lexical scope tree. Source-declared lifetime regions use distinct `DeclaredRegionId` metadata, never a manufactured `TypeId` or a reused lexical-region identity.

#### Dense HIR ownership

HIR expressions live in module-owned dense storage addressed by `HirValueId`. Store position supplies identity, so expression records need no redundant self-ID. Fixed child relationships store IDs inline. Variable-sized argument, element, field and string-piece payloads use typed ranges into dense side stores.

A place consists of a root local and an ordered projection range. Indexed projections refer to expression IDs rather than owning recursive expressions. One builder appends during construction. IDs/ranges survive vector reallocation and compact conversions reject overflow. Completed HIR freezes to fixed dense ownership before publication.

All visitors, remappers, validators, side tables and lowerers consume the same store. Numeric value payloads retain their full width/precision/scale and are not mistaken for expression graph edges. The storage contract creates no persistent-cache, mmap, unsafe-pointer or general allocator framework. `Compiler storage and capacity` owns capacity estimation and lifetime barriers. Summary-dependent private failure rewrites complete before final immutable publication, and published link facts describe the resulting CFG.

#### Lowering boundary

HIR owns explicit control flow, local/place access, lexical regions and terminators, stable source/binding calls, concrete generated targets, runtime template construction, slot accumulation, map operations, numeric/cast operations, external finite-float checks, Wiring argument/capture facts, module constants and function origins.

Effectful expression work is linearised into ordered statement preludes and temporary locals before its final value is used. Plain binary expressions remain valid for booleans and comparisons. Scalar arithmetic and negation use explicit checked numeric operations. Template strings use explicit append operations. Validation rejects work left in the wrong representation.

HIR contains no unresolved generic/trait work, TIR, compile-time page fragments, config evaluation, target-selected source branches, absolute source paths, HTML URL routes, rendered URLs, content hashes, output paths or builder names. It does not choose lifetime topology, runtime ownership or physical layout.

HIR explicitly distinguishes local definition from assignment to an existing
place. Every execution of a definition establishes a new dynamic occurrence of
that local, including re-entry through a loop. A static `LocalId` is not that
dynamic source binding identity. Local-origin metadata keeps authored bindings
separate from compiler temporaries, and a compiler definition does not itself
make a local Wire-capable. Assignment through an existing mutable alias writes
through to its referent rather than detaching it.

An assignment from the same unprojected binding preserves its existing binding
relationship and value provenance after the ordinary read and write checks. It
does not establish an alias of that binding's own cell.

The value's resolved place/rvalue classification determines the incoming binding
relationship. An internal result-local load can be an rvalue. A result binding
may share an allocation without aliasing another binding's cell, and materialising
a produced value does not imply an independent copy. Parameter entry follows the
function-call ABI. Dedicated operation destinations explicitly distinguish
definition from update, including generated numeric state updates. CFG transfers
name their source and destination locals, read all incoming values before writing
and define the selected edge's destinations. Lowerers consume these facts directly
without recovering them from borrow solver modes or entry-state snapshots.

Structural String constants retain ordered Text/Resource/SiteRoot pieces. Folded fragments retain the same value shape plus insertion indexes in metadata. Per-function and metadata walks collect resource uses and a separate site-root-use fact, since SiteRoot has no `ResourceId`. The builder supplies a validated URL map for each physical variant. Lowering materialises final characters from that map without mutating canonical HIR. Inactive static branches contribute no executable uses.

#### Result channels

HIR preserves ordered success slots and an optional single error channel. A call executes once and produces ordered result locals. Fallible calls have mutually exclusive continuations: success locals are defined only on the success edge and the error local only on the error edge.

An infallible `Call` statement carries its ordered arguments and zero, one or multiple success destination locals. A fallible `Invoke` terminator carries the resolved target and arguments, a success successor with ordered destination locals, and a failure successor with one error local. Success and failure edges define their own destinations without a semantic carrier local or tuple extraction. Destinations are unique within each edge, and the success/error destination sets are disjoint. Before selecting an edge, transfer kills both successor destination sets, then defines only the selected edge's locals, so reused locals cannot expose stale values across loops or repeated calls. Ordinary `Jump` argument transfer reads all sources before writing destinations.

Successful returns carry ordered values. Error returns carry one error value. Zero success results remain valid for both infallible and fallible functions. There is no semantic `Result<Tuple<...>, Error>` carrier or hidden tuple/payload extraction path.

Value-producing branches and catch recovery join individual slots through ordinary block/value targets. Propagation transfers the error channel without constructing an aggregate. Result-slot identity survives analysis, generated artefacts and public summaries. Backends may pack physical results only at the ABI boundary while preserving order, references and exactly-once evaluation.

HIR makes implicit-failure edges, handler joins and built-in `Error` materialisation explicit. The private inferred lane preserves the same success/failure exclusion: success slots do not exist on failure edges. Handlers and propagation consume frontend-owned contracts rather than reconstructing expression coverage or error compatibility.

The private same-module failure lane uses the same explicit success/error control-flow contract. Any shared physical carrier belongs to the target ABI only. The lane adds no public or foreign result contract, global last-error variable, semantic aggregate or fabricated payload extraction. Installing the lane and pruning obsolete handlers preserve slot definitions, CFG validation and final link facts.

JavaScript may pack results into a private array/object and external wrappers may translate their success/error envelope at the ABI boundary. Supported Wasm result types use native ordered results through function types, calls and returns. Unsupported aggregate, handle or fallible ABI capabilities receive the normal target diagnostic until their lowering contract exists. ABI storage alone creates no source allocation family and merges no result lifetimes.

#### Lazy assertion failure messages

An assertion message is an ordinary HIR value used only on the failure edge. Message preludes stay in that block. The optional message is evaluated once, then `AssertFailure` terminates. A compile-time `true` assertion has no message runtime work. A compile-time `false` assertion remains terminal after its failure-edge work.

Assertions remain unrecoverable and checked in release. Allocation and stack exhaustion are outside `catch`; growable `push` remains infallible in source and fatal on allocation exhaustion.

Validation, remapping, display, borrowing and reachability treat the message as an ordinary value use. It creates no assertion-specific ownership or Wiring category. HIR also carries the compiler-owned distinction between default/fully folded messages and messages requiring runtime construction. Target validation consumes it rather than inferring it from source.

#### HIR validation

Validation completes before borrow or target validation. It checks definition/type links, dense IDs and ranges, lexical regions, control-flow graph (CFG) shape, block ownership/terminators, locals/places, dominance, side-table mappings, function origins, constants, patterns, expressions and Wiring contracts/captures.

Call validation also checks result count, slot types, channel compatibility and edge-specific definitions. Finite binary-float values remain an invariant. `NaN`, infinity, a missing trusted local or malformed completed control flow are compiler bugs.

The module artefact validator, not HIR validation, checks folded fragments and their insertion indexes. A structured HIR view may be derived and validated for a structured lowerer. It remains cached derived data, never another semantic authority.

#### Numeric ownership

- `moth-lexical` owns shared numeric leaf facts: spelling grammar and classification, token-free materialisation, precision-aware formatting, fixed scalar identities, width/profile/precision vocabulary and borrowed decimal scale/text facts. It owns neither compiler type lookup nor a general numeric runtime.
- `moth` owns semantic numeric typing, receiving rules, promotion, constant evaluation and cast evidence. Its immutable Dec coefficients and exact arithmetic retain the compiler's `num-bigint`, `num-integer` and `num-traits` dependencies rather than moving them into either extracted crate.
- HIR records canonical numeric domain, operator and failure mode, not backend helper names or one statement family per target. `NumericFailureMode` selects the disposition required by the enclosing handler or function contract. A backend never infers it from the function name.
- Compile-time and runtime operations round/fail at the same semantic boundaries. `Dec` rounds at every language-level operation result.
- Semantic discharge is separate from check elision. The pure predicate `numeric_operation_cannot_fail`, beside the promotion table, proves fixed-width integer `Add`/`Subtract`/`Multiply`/`Negate` infallible when the operands' complete canonical ranges keep the mathematical result inside the result domain (for example `U8 + U8 -> U32`). It reads types only, never a `NumericProfile` or build profile. AST failure facts and HIR emission call it with the same pre-promotion operand domains: a discharged operation keeps catch eligibility, records no implicit contributor and lowers as a `NumericOp` with `NumericFailureMode::Infallible`, which backends lower like `Trap` and which never requests the recoverable-numeric Wasm capability or becomes a private-lane producer.
- `NumericProofs` is an optional sparse side table of proven-safe integer operations and fallible integer narrowings, keyed by HIR statement id and stamped with one `NumericProfile`. Each base executable and generated sidecar computes its own table from that executable's validated HIR before publication; facts stay paired with that exact immutable executable. Summary convergence itself does not alter numeric statements. After convergence, the private failure lane rewrites escaping private checked operations from trap delivery to `ReturnError`. That changes failure delivery, not which results are valid.
- An absent or empty table, a missing statement fact or a profile-mismatched query retains runtime checks; statement ids need no remap. The analysis reads HIR without mutating it and changes neither source acceptance, AST evidence nor target gates.
- The analysis derives closed `i128` intervals from canonical integer bounds, including the complete `U64` range, fixed integer literals and infallible integer casts. It walks each block independently with a single last-written-local cache. Every write invalidates the earlier fact: a direct-local assignment replaces it with its new interval, while projected writes clear the cache; calls and unknown statements also clear it. If checked endpoint arithmetic cannot represent an intermediate in `i128`, the analysis declines proof.
- `Power`, real division, `Byte`, `Float`, binary floats and the language `Dec` domain never prove. Lowerers elide only a proven predicate they support; source validity, evaluation and failure order, and required recovery or narrowing carriers remain unchanged.
- Target validation checks reachable numeric capabilities. Lowerers consume the boundary profile, canonical domain and physical plan rather than parallel width tables.
- Binary-float and Dec formatting use the common value-to-string boundary for templates and runtime lowering.

Target-specific check elision remains a lowerer responsibility; each backend consumes only the proof classes it supports. Check elision is independent of failure delivery and preserves explicit failure continuations. Scalar size, alignment and computation carriers remain distinct from semantic numeric identity. Broader numeric proof algorithms remain deferred.

#### Call targets

Executable source calls distinguish module-local functions, stable cross-module functions and stable binding-backed external functions. HIR stores no dependency alias, package spelling or backend runtime name as a call target.

Borrow validation resolves calls to access/effect summaries. Target validation and lowerers resolve executable targets through explicit graph, capability and link-plan inputs.

### Stage 6: borrow validation

Borrow validation runs for every canonical module and generated concrete function. It enforces shared/exclusive access, mutable call access, conservative collection/map aliasing, control-flow joins and no-later-use proof for optional transfers.

Its solver-independent output contains:

- normalised semantic places and overlap facts
- value origins and preliminary provenance
- shared/exclusive loan or access liveness
- path-sensitive future-use and last-use classifications
- optional affine-transfer candidates
- proof that persistent edges or affine roots cannot disappear while dependent temporary borrows remain usable
- proof that no capable source alias survives a proposed cleanup frontier
- preliminary return-root alias/projection evidence
- resolved external access-boundary classifications

An affine responsibility is cleanup responsibility that may transfer at a proven final-use site. It neither proves uniqueness nor creates another lifetime owner. Optional inferred transfer is an optimisation, not an acceptance requirement. Transfer requires proof on every relevant path. Otherwise the operation remains a borrow. Both immutable and mutable parameters may receive cleanup responsibility at a proven final-use call.

Borrow validation reads validated HIR and produces immutable side tables. It neither rewrites HIR nor decides final result provenance, retained-edge summaries, lifetime topology or physical ownership. The memory authority owns the detailed solver contract, not a particular abstract-state implementation.

Borrow validation consumes HIR's definition/update and incoming-value contracts.
A re-entered definition retires the prior dynamic binding state before establishing
the new relationship. Assignment preserves existing slot/alias write semantics.
Local modes are analysis state used to validate access and derive permitted
optimisation facts. They cannot reclassify a definition as an assignment or become
a backend's authority for the source operation. Binding identity remains separate
from allocation provenance and lifetime ownership.

Cross-module transfer consumes stable exported summaries without opening a callee's HIR as local control flow. Binding-backed calls use semantic package access/mutation/alias contracts, not source spelling or backend helper names. Missing trusted summaries are compiler bugs. Separate result slots preserve known alias relationships without inventing disjointness.

Closed external profiles restrict transfer: WIT value-only and restricted host-value crossings are non-consuming. Mutable opaque-handle access does not transfer Moth storage through the native ownership ABI. Donor-local type, allocation-family, region and counter IDs never enter public summaries, and REC is not source semantics.

Wire identity is not a persistent value borrow. Ordinary reads, captures, mutations and their lifetimes still undergo normal validation. Fresh rvalues passed to mutable call slots become hidden locals before checking, without becoming legal mutable receivers merely because they were materialised.

GC-native targets may ignore physical affine-cleanup facts but cannot skip access or lifetime-topology validation. Collector choice preserves these source-law decisions. Numeric-profile differences and target capability checks remain separate contracts.

## Lifetime-region and escape validation

Lifetime-region and escape validation is a backend-neutral analysis after Stage 6 and before target planning, not a numbered Stage 7. It consumes validated HIR and immutable borrow/effect facts without rewriting HIR.

The memory authority defines the full model. In this boundary:

- An **allocation family** is a containing allocation together with field, element and other projections still rooted in it. A projection is not an independent allocation merely because it has a distinct place.
- A **region** is a semantic lifetime-owner domain, distinct from a lexical scope or physical arena. Inferred regions have compiler-chosen non-lexical intervals. Declared regions have explicit membership, cannot be silently widened and retain their allocations until region exit.
- A **retained edge** is a directed storage relationship from a retaining object to a referenced allocation that survives the immediate operation. A **retention domain** groups these edges for creation/destruction analysis.
- A **final cleanup frontier** is a control-flow point after which the relevant edges and usable aliases are gone and cannot be recreated. The proof conditions are under `Retained-edge analysis`. A **region epoch** is one population-and-teardown cycle while the retaining aggregate may remain alive.

Every allocation family has one proven semantic lifetime owner. A retained value's region must be the same as or outlive its container's region. Inference starts with the narrowest valid owner and widens only along the nearest existing ancestor on one ordered lifetime chain. It cannot move laterally across independently ending siblings or invent a lifecycle root to avoid a diagnosis.

Local analysis owns allocation-family identity, complete result provenance, retention cardinality, escape/outlives constraints, detached stored results, whole-domain kills and candidate cleanup frontiers. It publishes immutable intervals and exit-specific lifetime/effect summaries. Frontier-enabling summaries describe effects that may enable cleanup, not donor-local concrete frontier IDs.

Caller/link analysis combines summaries over the reachable graph with aliases, future edge creation, other retention domains and builder-supplied lifecycle roots. Local module compilation alone cannot establish all cross-module or lifecycle relationships.

Topology legality is a mandatory proof. Diagnostics distinguish topology proven invalid from topology not proven legal by conservative analysis. Both differ from failure to prove an optional transfer or physical optimisation. Backends consume validated topology and cannot reopen source legality.

### Retained-edge analysis

Retained-edge analysis owns domains, direct edge creation/destruction, whole-domain kills, path-sensitive final frontiers, region epochs and semantic commit points. A semantic commit is the successful mutation boundary at which the retained-edge state changes. The **may-coexist relation** records which direct edges may exist together in one reachable committed state.

A final cleanup frontier may end an inferred region before the aggregate that held its values. On that path, every relevant retained edge must be gone, no live local/projection alias may survive, no capable source may recreate an edge and no external or builder lifecycle may retain the family. Any surviving aliasing aggregate must be unable to retain it again.

Whole-domain kills such as `clear`, destruction or replacement can enable that proof. Individual `remove` or `set` does not establish a frontier. Clearing declared-region edges does not reclaim their storage before region exit. Failure to prove a frontier retains storage conservatively, rather than invalidating otherwise legal topology. Per-removal uniqueness scans and alias registries are excluded.

Cycle validation runs over direct retained edges that may coexist in one reachable committed state. A successful retention-sensitive mutation removes old edges and adds new edges atomically at its semantic commit. A failed mutation preserves the old topology and its retained-edge obligations, while incoming affine responsibility follows its separately planned failure path.

Path or epoch separation may prove that edge sets never coexist. A path-insensitive union of pre-commit and post-commit edges is therefore not proof of a runtime cycle. Where coexistence cannot be disproved, topology is not proven legal and receives the normal diagnostic. A real direct self-cycle or multi-family strongly connected component (SCC) requires one declared region. Inference never invents a cyclic region.

### Backend-neutral memory requirements

After topology, intervals, frontiers and epochs are complete, analysis publishes the final target-independent requirements shared by physical variants. They contain allocation families, validated owners, intervals, frontiers, epochs, retained-edge/domain facts, cardinality, declared-region membership, affine cleanup/transfer candidates, hidden-destination constraints, lifecycle/external constraints and possible Retained Edge Counting (REC) use.

REC may serve runtime-dependent persistent-edge multiplicity when edges disappear independently and simpler cleanup, region or bounded-retention strategies are insufficient. It is not an alias counter or a way to legalise topology. Requirements carry candidacy facts only. The memory authority owns the selection ladder.

Requirements contain no selected physical strategy, allocator, counter/arena/handle layout, concrete retained-edge obligation transition or planned affine-responsibility transition. A hidden-destination constraint describes a required result-storage relationship, not a backend's physical allocation choice. Target, build-profile and layout-dependent decisions belong to the memory planner.

### Memory-strategy planning

After build-owned partitioning and compiler-owned target validation establish a candidate physical variant, the compiler-owned planner consumes validated topology, backend-neutral requirements, target, build profile and memory capabilities. It returns one `ValidatedMemoryPlan` for that variant.

The planner selects one physical strategy per allocation family. Choices include stack/inline placement, static affine cleanup, inferred-region allocation, declared-region bulk reclamation, REC and permitted host-GC representation. Borrow and lifetime analysis supply facts, never representations. Lowerers realise the plan, never choose strategies. Selection is deterministic for the same inputs and cannot affect source legality.

Field-sensitive family/layout refinement runs per candidate variant. It rebuilds affected direct family edges and revalidates outlives, SCC and family-base facts. A split establishes independent child ownership before the runtime escape it supports, never retroactively detaching an escaped projection. Unproven refinement falls back to the unsplit family with conservative retention, never a source diagnostic.

The planner owns every concrete physical transition. It translates each semantic commit into normalised per-family obligations, fuses same-family removals/additions before lowering and selects root-to-edge or edge-to-root reclassification. Backends encode these transitions rather than deriving retain/release operations from borrow facts.

Physical coalescing finishes before plan validation and fingerprinting. It may retain storage longer without widening semantic topology or changing outlives/legality. A full-control release variant cannot select `HostGC` or fall back to a tracing/reachability collector. Failure to provide a physical strategy for proven-legal topology is a compiler bug.

`docs/src/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf` > `ValidatedMemoryPlan contract` owns complete plan contents and publication invariants. Its `Planning order` and the build document define orchestration. `check` validates the plan before stopping short of lowering/output.

### Declared regions and backend handoff

AST owns declared `name:` scopes, `into name` placement and freshness validation. The exact `_:` header is invalid. Declared-region identity is separate from type identity and lexical HIR regions. HIR records region metadata/exits and explicit recoverable failure control flow before memory analysis. It still does not decide exact lifetime topology.

The declared-region and lifetime/escape references under the memory authority own the detailed source and analysis rules. `Backend-facing compiler handoff` defines the final input boundary. Borrow facts and topology remain validation context, not alternate authorities for physical cleanup or counting.

## Per-function link facts

The compiler records backend-neutral facts for each source and generated function:

- stable local/cross-module source calls and binding-backed calls
- runtime helper and capability requirements
- numeric/cast, map and other target-gated operations
- required Wiring contracts/captures where execution needs a capability
- resource uses in deterministic source order and a separate site-root-use fact
- per-function project-context provenance

Generated-function requests accompany module link data as materialisation dependencies. They remain distinct from runtime-dependency fingerprint contents. Non-executable fragments and root metadata carry their own resource/site-root/provenance facts in the metadata lane.

These per-function facts are the linking authority. Module-wide summaries are derived indexes only. Reachability follows stable call edges recorded from already-specialised HIR, without rescanning HIR for link facts, folding constants, choosing branches, mutating HIR, inspecting borrow facts or choosing targets/output. Some target checks also inspect the paired semantic type environment rather than reconstructing types from syntax.

## Target-contract validation

The build system supplies explicit entry/package/tooling roots and the completed deterministic target assignment. Compiler validation runs after HIR, borrow and complete link-level lifetime-topology validation.

It traverses reachable source and generated functions, checks target-gated operations, binding profiles and host capabilities, and uses semantic types where link facts alone are insufficient. Mixed-target validation checks each function against its assigned target and validates every cross-target edge. Unsupported reachable features are structured user diagnostics. Inconsistent trusted compiler/builder metadata is a compiler bug.

Unreachable private functions and statically removed executable branches impose no target requirements. Their authored source still received frontend validation. Target rejection never changes source meaning or hides an invalid inactive branch.

An assertion message needing runtime construction requires a target that can execute it faithfully. Otherwise validation reports the authored message location. Default or fully folded values may use static trap lowering where their source-visible work is complete. Lowerers cannot discard reachable evaluation silently.

Reachable site-root strings and browser document-title calls likewise require explicit host policies/capabilities. A backend name alone does not prove a browser host or a meaningful site root.

Target validation precedes physical memory planning. A failed physical refinement falls back conservatively without reopening source or target validity. Accepted `$layout` direction constrains representation without creating another nominal type category. Its exact field/layout contract remains open design, so consumers must not invent one.

## Backend-facing compiler handoff

A lowerer receives only explicit validated inputs:

- selected module/generated HIR and paired type environments
- the boundary `NumericProfile` and stable call targets
- borrow/lifetime facts and exported summaries where needed as validation context
- paired profile-aware numeric-proof facts for predicate elision only; they do not change source legality or mutate HIR
- the variant's `ValidatedMemoryPlan`
- closed external-boundary classifications
- per-function link facts and selected function/import/export/capability plans
- required semantic layout identities and builder lifecycle/runtime plans
- the validated structural-string URL map for the containing output context

`ValidatedMemoryPlan` is the sole authority for aggregate layouts, memory strategies, cleanup, inferred/declared-region operations, REC and concrete ownership/count transitions. Source and generated functions follow the same handoff. Missing compiler-owned boundary classifications or required plan facts are compiler bugs, not invitations to infer a fallback.

Lowerers implement language-owned operations with native instructions or helpers only when they preserve the full contract. They never load/parse source, rebuild visibility, infer generics, reconstruct traits, fold templates, interpret TIR, choose entry/route policy or write final project outputs. They cannot weaken numeric checks, finite-value validation, cast failure, map behaviour, error propagation or Wiring semantics. A target may elide a numeric predicate only when its paired proof establishes safety and the lowerer preserves evaluation order, failure semantics and required carriers.

### Numeric scalar storage and carriers

A semantic type describes language meaning. Scalar storage describes bytes/alignment. A computation carrier is the target register/value representation used to operate on those bytes. Carrier or storage width never changes canonical identity.

`docs/src/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf` > `Scalar storage and carriers` defines ordinary Wasm scalar sizes, carriers, alignment and load/store behaviour. The runtime/backend memory authority owns physical planning and aggregate layout. Other targets supply physical policies preserving the same semantics.

Scalar metadata alone creates no allocation family and moves no aggregate-layout or cleanup authority from `ValidatedMemoryPlan`. Compact storage creates neither a buffer source type nor arithmetic for `Byte`.

## Rust-only MON service

The standalone Rust crate `moth-mon` owns the isolated literal-data codec. Its allowed dependency direction is `moth` -> `moth-mon` -> `moth-lexical`, with a direct `moth` -> `moth-lexical` edge. Neither extracted crate depends on `moth` or `xtask`, including development and build dependencies. Independent Rust consumers depend on `moth-mon` without linking the compiler. The supported `pub use moth_mon as mon;` convenience re-export exposes the same API and Rust types, not a compatibility shim, wrapper or second implementation. Canonical Rust API documentation belongs to `moth-mon`. The codec receives complete caller-supplied text or values and a prepared schema, then returns owned data or encoded text. It performs no project discovery, file IO, module compilation, expression evaluation or backend lowering.

`moth-lexical` also owns the shared identifier character rules, source-word
inventory and reserved user-name predicate. Its numeric ownership appears
under `Stage 5: HIR and validation` > `Numeric ownership`.

`moth` retains `Token`/`TokenTag` mapping, token-store/materialisation
adapters, `NumericScalar`/`TypeId` projections and `NumberValue` with its
arbitrary-precision arithmetic. Compiler diagnostic codes, payload construction
and renderers remain local, while the shared numeric-text reason vocabulary
sits in `moth-lexical`. The compiler-local call-argument owner continues to
serve source calls, constructors, directives and named records. The
compiler-local declaration-shell owner continues to serve headers and
body-local declarations. MON uses neither parser.

`docs/src/docs/mon/mon-format.mtf` owns root/field grammar, qualifiers, maps, defaults, encoding and receiver validation. `docs/compiler-data-layout-design.md` > `MON data handoff` owns cursor/input lifetime, spans, bounded allocation and public error context. Rust API signatures and routine internal shapes belong in public doc comments and their implementations, not a second field catalogue here.

### Isolation and schema ownership

The literal reader uses a bounded MON cursor, not AST expression parsing, compiler tokens, TIR or compiler identity tables. It consumes the token-free numeric service described under `Stage 5: HIR and validation` > `Numeric ownership`, while retaining MON-local traversal and escape decoding. Syntax sharing never authorises expression evaluation or widens source-language escapes.

A caller prepares and validates its finite schema/default tree once, then reuses it immutably. Schema eligibility is transitive. Owned decoded values remain valid after the caller releases input, with no public borrowed document view, alias preservation or cyclic value graph. Internal borrowing during a call is permitted.

Schema preparation captures `NumericProfile` for `Int`/`Uint`/`Float`, defaulting to the standard profile for standalone Rust callers. `Uint` follows the profile `Int` width there too. Explicit-width and Byte schemas are profile-independent. The accepted numeric extension covers all fixed widths, `Uint`, Byte and Dec scales through 256 without another numeric parser. Integer/Byte targets require whole-number spelling, with `Uint` applying the shared unsigned policy and retaining the full `u64` payload, fixed floats use direct binary rounding and Dec conversion remains exact. Encoding preserves typed round trips, including signed zero. Map-key schemas include `Uint`, fixed integers and Byte, while binary floats and Dec remain excluded.

### Publication and receiving contracts

Document/value encoders share one writer. Strings encode as quoted data, never trusted raw MON fragments. The decoder validates the complete receiving schema and applies defaults only at missing fields. Optionality alone does not allow omission. Supplied `none` is not replaced and nested records do not deep-merge with defaults.

Resource and site-root pieces must have final characters before conversion to MON data. Unresolved anchors fail conversion. The codec exposes no Resource value and never reconstructs dependencies from strings.

Every public operation returns a complete result or its first structured `MonError`, never partial output. Schema misuse and MON internal failures use this public projection, not compiler diagnostics, `InfrastructureFailure` or `compiler_bug!`. Available decode spans are exact half-open byte ranges in the original input. Rendering their source excerpts or line/column locations requires that input, not compiler identity storage. The owned reason, detail and value path remain available without it.

Finite receiver limits are policy, not grammar variants. Budget failures remain distinct from syntax and schema failures. Checked counters charge before allocation/expansion, including defaults, repeated values and numeric scratch. Map entries incur their additional node charge consistently across parsed, programmatic and default paths. Accepted recursion and rejected-value/schema cleanup stay bounded. The data-layout and format authorities retain the exact limits and accounting rules.

Writing is deterministic for the same ordered value, schema and encoder version, not canonical-byte equivalence. Applications own versions, migrations and resource interpretation. Producer-side validation never replaces receiving-boundary validation.

### Moth-native integration boundary

Moth-native encode/decode, `$mon` convenience, automatic schemas from ordinary Moth types and static MON assets are accepted extensions. They reuse this codec and normal output ownership without serialization traits, another literal parser or hidden schema/runtime state in strings/templates. Exact directive/command syntax and schema-extraction/runtime handoffs remain open design. Dynamic schemas, public borrowed views, streaming and canonical-byte services require a separately accepted use case.
