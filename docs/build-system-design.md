# Moth Build System Design

> **Final design only.** This document specifies accepted build-system design, regardless of implementation progress. Status, migration steps and backend coverage belong in the roadmap and progress matrices. `V1` identifies a deliberately bounded feature contract, not unfinished implementation. Mark genuinely unsettled design explicitly.

The build system selects commands/capabilities, bootstraps configuration, discovers source, constructs project/package graphs, schedules compiler services, plans linked artefacts and owns output writing.

`docs/compiler-design-overview.md` owns semantic identities, compiler stages, artefact contents, generated semantics, fingerprint domains and validation. Its authority rules and companion references apply here. This document owns orchestration and build policy, not another definition of compiler facts.

Accepted plans resolve design gaps and explicit supersession. Implementation behaviour never overrides them. Preserve unique accepted contracts in permanent owners before retiring a plan. Exact public directive signatures belong in the canonical project-structure references, with architecture and consequential validation rules retained here.

## Task-reading guide

Read the opening authority text and `Architectural invariants` in both documents for every build-system task. Heading paths use `>` for nesting. Read a selected section through the next heading of the same or higher level, including children unless narrowed explicitly. Read both documents fully for cross-boundary changes or thorough reviews.

| Task | Read here | Compiler companion |
|---|---|---|
| Commands, capabilities and tooling | `Selected command and capability surface` and `Command and tooling policies` | New semantic capability or validation-root owners |
| Config, inputs, project globals and root metadata | Relevant `Project bootstrap` subsection | Stage 4 constants/directives and config service |
| Discovery, ownership and source preparation | `Source indexing and source sets` and `Prepared-source orchestration` | Module input and Stage 1-3 contracts |
| Modules, packages, facades and namespaces | Relevant `Project and package topology` subsection | Stable identities and public interfaces |
| Compile waves, publication and failures | `Deterministic scheduling and graph outcomes` | Module outcomes and diagnostics/identity |
| Generated functions | `Generated-function boundary` | `Generated concrete functions` |
| Entries, packages, reachability and validation roots | `Entry and package link planning` | Link facts and target validation |
| HTML assembly, partitioning, variants and runtime | Relevant `HTML project builder` subsection | Backend handoff and routed memory contracts |
| Resources, URL contexts and conflicts | `Resource linking and output placement` | Stage 4 file values/resources |
| Output roots, manifests and transformations | `Output ownership` | Selected builder/lowerer handoff |
| Development reuse and persistence | `Incremental and persistent artefacts` | Fingerprint domains |
| MON tooling and static assets | `MON tooling and static assets` | `Rust-only MON service` |

Use `index.md` and owning module entry points for code locations. Source-path inventories are navigation, not design contracts.

## Architectural invariants

- One command selects one artefact builder and any tooling overlays before config validation. Source config does not select physical targets or builders.
- `config.moth` is one self-contained compile-time source, with no source dependencies or package resolution.
- Stage 0 owns source ownership, canonical project/package graphs, legal topology and deterministic scheduling. Each physical module compiles once per boundary.
- Compiler-owned preparation retains syntax for graph construction and later binding. Build code neither parses a competing grammar nor assembles semantic stages.
- Published modules, interfaces and generated sidecars are immutable. Diagnosed modules expose no partial interface. Builders receive success-only linkable inputs.
- Compiler expression stores end at their owning semantic handoff. Build aggregation consumes owned values and validated artefacts with their required identity contexts, never AST expression IDs or live parser/TIR authority.
- Entry activation, package assembly and target partitioning consume compiled facts without deferred source compilation.
- Moth-source packages remain Moth-linked when emitted as Wasm. WIT supplies foreign bindings, not native semantic interfaces.
- Physical file references establish conservative graph/input membership before executable selection. Exact output liveness does not change that membership.
- Compiler-owned resource origins/uses are distinct from build-owned byte sources, placement and URL contexts. Builders never rediscover resources by scanning rendered strings.
- Ordinary `$config` and static `if` do not change source graphs, declarations, exports or package topology. Structural `$feature` selection has a separate pre-graph contract.
- One boundary-wide `NumericProfile` fixes semantic numeric precision. Target partition and development/release policy never reselect it.
- The compiler validates lifetime topology and selects physical memory strategies. The build system supplies roots, target assignments and profiles, then stores the validated plans.
- Builders/lowerers return output records. The build system validates, writes and owns manifests, conflicts and stale cleanup.
- Parallelism, caching and reuse preserve deterministic identities, diagnostics and output order.

## Selected command and capability surface

Bootstrap starts with a command selecting the artefact builder, build profile, tooling overlays, explicit inputs and target intent. Builder-selection source/CLI syntax and a Moth-native build-script design remain unsettled.

The selected builder supplies the boundary's `NumericProfile` before command values or config numbers are materialised. The shared profile vocabulary belongs to `moth-lexical`, while compiler and build services retain boundary-wide selection and propagation. The build profile controls optimisation/planning effort, instrumentation and physical representation policy, not semantic numeric precision or mandatory access/lifetime legality. `docs/compiler-design-overview.md` > `Numeric model and profile` owns numeric values and compatibility.

The capability surface contains registered directive signatures/contexts, source-backed Core/Builder packages, binding-backed packages, template directives, import providers, runtime requirements, supported source kinds, primitive build globals and target-affinity/capability metadata.

Frontend names and builtins remain authoritative. Registration rejects duplicates and overrides. Unknown directives, wrong-context uses and recognised unavailable capabilities receive distinct diagnostics rather than silent acceptance. Registries grant constrained powers, not arbitrary semantic callbacks.

Builder globals express stable semantic configuration. They never reveal a physical target, backend, OS or architecture through `$config`. Command target intent stays outside source-visible configuration. Tooling overlays extend analysis without becoming competing artefact builders.

## Project bootstrap

### Self-contained `config.moth`

`config.moth` is one build-owned compile-time source, not a semantic module. It creates no `start`, HIR, runtime artefact or package interface. Bootstrap creates no config dependency graph, companion source set, package resolver or second project scan.

The build system calls the named compiler config service with the source, capability surface and numeric profile. It consumes folded directive arguments, input-contract results, key locations and diagnostics, then applies validated project/builder settings. The compiler alone sequences preparation and folding.

Completed config values use the compiler's ordinary constant projection. Applying them does not retain or reconstruct an expression graph. Source-span construction remains live for the later build validations described below, independently of the expression store's lifetime.

The source identity is registered upfront. Its live span owner survives config application and output-path validation on success and diagnosis, then freezes at the last producer boundary. The compiler/data-layout authorities define that handoff.

Allowed source consists of earlier private helper constants, explicit `$config` bindings, exactly one `$project` invocation and the active builder's required config invocation. Helpers may use scalar/optional values, anonymous const records, supported folded collections and folded templates.

Reject source dependency clauses before path resolution, authored file-value paths, runtime or mutable declarations, functions, named support types, traits/conformances, public exports, standalone output templates, page fragments, `$page`, nested config sources and inactive-builder settings retained as a holding area.

Config-local helpers do not become declarations visible to project modules. Only specialised build interfaces publish folded settings. Synthetic single-file operations use explicit synthetic configuration without inventing source directives or implicit input providers.

The accepted replacement contract is `docs/roadmap/plans/general-directives-and-project-config-plan.md`. Its `$config`/`$project`/`$html_builder` model is the only config architecture specified here.

### Project directive

One closed `$project` invocation supplies project identity/settings. It creates no ordinary symbol named `project`. `docs/src/docs/project-structure/project-config.mtf` > `$project` owns the complete positional signature and defaults.

`name` and `version` are required Strings. The name is a valid Moth project identifier, never inferred from the checkout directory. Version has no default. A starter's `"0.1.0"` input fallback is not a project-field default, and a same-named input contract does not satisfy an omitted directive argument.

Only predefined fields are accepted and published through `@project`. Arbitrary metadata, misspellings and `metadata = ...` are errors. Reusable project constants belong in ordinary source/support packages, not an open config record.

Project identity, discovery and compiler-control fields are fixed-only. Reject `$config` dependence in `name`, `entry_root` and `template_const_loop_iteration_limit`, including indirect dependence through helpers. An equal folded literal does not erase input provenance. Ordinary input-independent constant expressions remain valid.

`entry_root` defaults to `"src"` and obeys the containment rules under `Source indexing and source sets`. The loop limit uses one compiler-owned default and resource-bound validator, separate from the profile-selected `Int` range.

### Bootstrap `$config` declarations

`$config` takes no arguments and modifies exactly one following explicitly typed top-level compile-time `#` binding. Its name is the input name, its type is the contract and its initialiser is the fallback. Whitespace/comments may separate modifier and declaration, but another source item or block boundary may not. Duplicate or dangling modifiers are errors.

The domain is `String`, `Int`, `Float`, `Bool`, `Char` and one optional layer. Fixed-width numerics, Byte, Dec, aliases, inferred types, collections and records are excluded. Runtime/mutable bindings, parameters, fields, locals and multi-bind targets are ineligible. Ordinary constants still require initialisers.

A non-optional binding without an initialiser requires an input. A fallback supplies the missing value otherwise. An optional binding without an initialiser and one with `= none` share the same normalised absence default. A present optional fallback stays present.

Bootstrap resolution order is:

1. Explicit CLI or programmatic input for that name.
2. A compatible builder-provided primitive global.
3. The validated declaration fallback or optional absence.
4. A required-input diagnostic.

Config fallbacks may fold the permitted single-file surface and earlier helper constants. They finish as a supported primitive/optional and retain normal same-file forward-reference errors. Resolution occurs inside config compilation before settings such as `entry_root` are applied.

Project fields and ordinary helper constants neither supply nor block same-named inputs. Supplying an input binding to a differently named directive parameter does not rename its contract. Project metadata publication and input resolution are separate systems.

### Command build-input typing

Repeated `--input name=value` arguments are typed before contracts are discovered:

1. Split at the first `=` and retain later `=` characters. Require a lower_snake_case name.
2. Recognise exact lowercase Bool, a complete signed whole-number literal, a complete decimal/exponent literal, a quoted Char or a quoted String, in that order.
3. Treat other text, including empty text after `=`, as String.

A quote-starting value must be a complete valid quoted literal. Malformed quotes are diagnostics, not String fallback. Shared `moth-lexical` numeric validation/materialisation enforces range and finite-value checks using the already selected profile.

Bare `none` is String text. Optional absence comes from omission/default resolution. A concrete `T` may satisfy matching `T?` as present, with no other coercion. In particular, `Int` does not satisfy `Float`.

Explicit quotes force String for ambiguous values:

```bash
moth build . --input 'label="true"'
moth build . --input "separator=':'"
```

Programmatic APIs use the same typed carrier and conversion policy. Unknown inputs are diagnosed only after the selected contract inventory is complete, not merely because config lacks a matching declaration.

### `ProjectGlobalsInterface` and `@project`

Folded `$project` settings produce one immutable synthetic interface at permanently reserved `@project`. It publishes only predefined fields as namespace members, not a nested `project` value or all config helpers/inputs.

The compiler's synthetic-interface contract supplies stable member identities, folded values, field fingerprints and provenance without AST/HIR/runtime bodies. Classification is ProjectLocal and MothSource for provenance/capabilities, but the interface is not discovered as a source package.

Normal project modules and project-owned support packages may explicitly import it. Injection and direct re-export are forbidden. No child module, support package, alias, Core/Builder package or binding may claim its root.

Internal declarations may derive from project values while carrying project-context provenance. External facade policy rejects prohibited exposure. Field-granular dependencies drive reuse. Source locations and resolution origin remain diagnostic provenance rather than semantic fingerprint inputs.

### Source contracts and static specialisation

Selected-source `$config` contracts use the same primitive/optional domain and explicitly typed top-level binding rule. Their defaults are narrower than bootstrap defaults: a matching String, signed integer/float, Bool or Char literal, or `none` for an optional. A present primitive literal may supply an optional default.

Names, projections, operators, calls, casts, templates, collections, records and references to other inputs are invalid source defaults. This permits literal normalisation before provider interfaces or AST evaluation, without a second general constant evaluator in Stage 0.

Preparation retains each contract's name, type/optionality, required/default state, normalised default and source location. A boundary-wide barrier validates all selected contracts before module AST compilation.

Matching names agree on all those semantic contract properties. Explicit overrides do not excuse conflicting defaults. Same-file duplicates and no-shadowing rules still apply. Matching declarations in different modules share an input, not a local declaration identity.

Selected-source resolution order is:

1. The already resolved matching bootstrap contract, after compatibility validation.
2. Explicit input for a source-only contract.
3. A compatible builder global.
4. The common source fallback or optional absence.
5. A required-input diagnostic.

Project metadata is never an input provider. Each project/package boundary has its own namespace. Consumer inputs do not implicitly satisfy dependency contracts. Unknown explicit inputs are diagnosed after all selected-source contracts are known.

AST receives an ordinary folded constant. A Bool input uses ordinary `if`, with both branches frontend-validated before executable selection. Config values do not change source discovery, declarations, exports or graph topology. The compiler's static-specialisation section owns downstream suppression rules.

### `$html_builder` and builder settings

An authored HTML directory config contains exactly one closed `$html_builder` invocation. Bare invocation uses defaults. Empty directive parentheses are invalid. The command already selected the builder, so this directive never chooses a backend or runtime platform.

The canonical project-config reference owns its signature, positional order and defaults. Shared argument validation fills defaults once. The typed HTML settings owner checks domains such as URL style, origin path, non-empty language/present favicon and output-root containment/profile conflicts.

Settings consume permitted folded values without declaring inputs implicitly. An untracked favicon URL may be a folded String. Tracked resource paths belong in normal-root `$page` metadata because config has no source graph.

Project settings and page metadata have distinct signatures. Their explicit document fallback rules do not create a generic cross-scope inheritance or merge system. Multi-builder configuration requires structural selection rather than silently ignoring inactive directives.

### Root metadata and purpose directives

`$page` declares HTML root metadata and purpose, not an embedded config source or a modifier on the next declaration. It is valid once at the top level of a normal module root, or the explicitly selected synthetic single-file root. It is invalid in ordinary helper files, API-only roots, config, functions, branches, loops, initialisers, templates and export declaration groups.

The directive describes the whole root regardless of position. Its arguments use ordinary file visibility, including earlier constants, dependency-bound constants, explicit project globals and resolved source inputs. Same-file forward references remain invalid. The canonical `entry-config.mtf` owns the closed argument signature.

Headers retain invocation and structural references. Module AST checks/folds it once into non-HIR metadata with authored argument locations. It creates no public declaration, runtime value or project global. Every selected module validates its own metadata, even when used only as a dormant provider. Consumers never inherit provider page metadata.

Authored root runtime work or direct output requires a purpose with an applicable consumer. This uses authored activity, not optimised HIR liveness or rendered length. An inactive root `if`, a runtime binding that folds and an empty direct fragment retain their authored role. Compile-time declarations and semantically classified documentation/comment templates are exempt.

Entry filtering retains unpurposed executable candidates for diagnostics rather than dropping them. A dependency cannot legalise invalid root work by leaving it dormant. Metadata presence alone is not execution permission. The accepted `$test` direction still needs its own selection, lifecycle and failure contract and cannot be assumed to supply one.

### Fixed bootstrap order

```text
select command, builder, build profile, NumericProfile and overlays
-> type explicit command inputs
-> construct the compiler/builder capability surface
-> call the config compiler service and apply folded settings
-> publish predefined project globals and input results separately
-> derive entry_root and construct canonical source/provider graphs
-> collect, validate and resolve selected-source input contracts
-> compile dependency-ordered waves through the module service
-> atomically publish complete module/generated results
-> assemble success-only project inputs
-> select entry/package roots and exact reachable unions
-> validate link lifetimes, partition targets and validate physical plans
-> lower variants and emit owned outputs
```

All numeric steps consume the same profile. Source contracts resolve after discovery but before module semantics. The compiler document owns local semantic sequencing. The HTML planning section owns the detailed link-to-output order. `check` stops after applicable validation, before lowering and emission.

Structural `$feature` selection must use bootstrap-available facts and act before excluded source publishes graph or declaration facts. Selected-source input values cannot drive this pre-graph step. Feature declarations, predicate grammar and lexical exclusion remain open design.

## Source indexing and source sets

After config supplies `entry_root`, Stage 0 builds one canonical source index per project/package boundary. A directory project's entry root is a relative directory strictly below the project root. Reject empty paths, `.`, parent components, absolute/escaping paths and symlink resolution to the project root itself. Single-file compilation is a separate synthetic-module mode.

The index inventories normal/support roots, an optional project facade, nearest ownership, supported source-kind candidates, provider-owned files, logical source/namespace identities, path collisions and deterministic order. It does not scan arbitrary `.moth` files as page entries. Root-purpose selection consumes retained facts.

Project-local packages are structural support roots or the project facade, not a configurable `package_folders` list or implicit `/lib` scan.

### Owned source set

`OwnedSourceSet` contains recognised source files whose nearest root is the module. It governs legal filesystem boundaries, collisions, diagnostic attribution, orphan detection and inventory identity. Ownership alone introduces no declarations into the module.

### Semantic source set

`SemanticSourceSet` contains the root, owned `.moth` files reachable through source clauses, reachable builder-supported source assets and other inputs explicitly classified as semantic by the selected builder.

A content asset reached through a dependency clause or an expression-position structural reference has the same membership and preparation. Only this set contributes canonical module declarations, HIR, interfaces and link facts. Its contents determine the semantic source fingerprint.

Provider-backed explicit-extension files follow their provider's ownership contract. They produce binding interfaces and runtime facts, not ordinary module declarations.

### Check source set

`check` also diagnoses owned `.moth` files outside the canonical semantic set. Each orphan is a separate check-only source unit under its nearest module namespace, using the same provider interfaces and visibility rules.

A check-only unit cannot add declarations to the canonical artefact/interface or become a backend root/link input. It has a separate tooling fingerprint. This lets tooling inspect disconnected code without changing dependency semantics.

## Prepared-source orchestration

Stage 0 schedules compiler-owned tokenization/header preparation and retains its result for graph discovery and later compilation. It consumes structural provider/file references, input-contract shells and root-purpose facts, passing the remaining syntax into the compiler unchanged. Stage 2 defines the payload and four reference classes.

Preparation happens once for a source in a preparation lane, then all consumers in that lane reuse it. A separate canonical/check-only analysis may use another lane without creating another physical source identity. Such work appends spans through the same live source-local owner. This is distinct from repeatedly parsing the same source to reconstruct graph or semantic facts.

Source IDs and span-table freeze have separate barriers. Workers return live span owners on success and failure, including discarded speculative preparations, before fallible aggregation. The boundary keeps each owner until its last canonical/check-only producer ends. AST lookup borrows end before exclusive table installation and terminal immutable publication. A package interface may publish before its source lookup context freezes if check-only producers still remain.

These source ownership rules come from the data-layout authority. Direct-template bundles transfer their snapshots/live owners to the compiler service on success. Failed discovery finalises known sources at its terminal boundary. Retained earlier warnings keep the correct document context when a later document fails. Private discovery normalises provisional source identities once before publishing either outcome.

### Physical reference resolution

Every retained physical-target-bearing reference receives one filesystem resolution under ownership/containment rules. A `.mtf`/`.md` reference adds the content source to the semantic set through its normal adapter before AST. An ordinary resource creates an input/watch interest and may register an unhashed byte source.

Stage 0 may inspect filesystem metadata needed to resolve and validate targets. It neither reads resource contents nor hashes unused bytes, chooses output paths, renders URLs or decides output liveness. AST receives the resolved outcome and has no filesystem resolver.

SiteRoot, extensionless non-dependency paths and `SourceKindNoFileValue` do not enter physical resolution. They create no source membership, byte-source record or watch interest. The compiler owns their value/diagnostic meaning.

A resource input records canonical physical source, owning root, validated logical target and watch interest. AST later creates stable semantic origin and authored-use facts. Association with the byte source publishes only with a successful module. Missing-target watch interests have no manufactured semantic origin. One physical file may back several origins, while equal origins must agree on byte-source facts.

Ordinary static `if` excludes no references from discovery. Executable folding cannot retract established graph/input validity. Structural feature selection, where defined, precedes publication of these references instead.

### Content-discovery worklist

Content sources may contain more references, so discovery grows the selected set until stable:

1. Seed from the module root and dependency-clause sources.
2. Prepare each newly selected source and consume its structural references.
3. Add newly discovered content sources and resource inputs.
4. Repeat until no source is added, settling membership before header aggregation. The canonical module service owns subsequent binding and local declaration ordering.

Deduplicate membership by canonical source identity, not authored spelling. Keep every reference location for diagnostics. Multiple references reuse one content preparation. Repeated discovery is not a cycle. Actual synthetic-content dependency cycles belong to local ordering. Final order and diagnostics are independent of worklist insertion order.

Stage 0 never binds symbols, orders local declarations or enters executable AST/HIR/borrow analysis. Input contracts and root purposes use retained metadata, not a second scanner. Provider discovery that mutates shared identities/caches/tables stays serial unless it supplies deterministic worker deltas and remapping.

## Project and package topology

A **module** is a directory-scoped compilation/visibility unit rooted by `@*.moth` or `+*.moth`. A **package** is a named reusable dependency root. A **binding** is a typed bridge to an implementation outside Moth source. A **prelude** is implicit dependency policy, not a package kind. "Library" is informal.

### Module roots and dormant work

A directory has at most one root. `@*.moth` defines a normal module. A `+*.moth` inside the source tree defines an API-only scoped support module. One optional project-root `+*.moth` beside config defines the external package facade. Root filename suffixes are cosmetic. `config.moth` is not a root.

Normal modules may own dormant runtime work/fragments. `$page` selects HTML entry purpose without changing module identity or making helpers independent entries. Compiler root-role rules define API-only restrictions and implicit `start`. Stage 0 ensures selected normal roots complete local semantic validation before activation, including unpurposed-work diagnostics.

### Module-root-relative dependency clauses

Source clauses resolve from the owning module root, not the declaring file's directory. For example:

```text
src/
  @site.moth
  accounts.moth
  internal/deep/renderer.moth
```

`@accounts Account` in `renderer.moth` resolves to `src/accounts.moth`.

Parent components and `@./...` are invalid. Paths may traverse unrooted directories within the same module. A child normal-module or support-package boundary ends traversal and exposes only its facade. `@child/internal` cannot bypass it. Scoped support packages are addressed by package name. Providers use an explicit owner without file-relative fallback.

One clause resolves one provider/module/package surface. Direct selections are flat binding names within it and create no independent provider edges. Interface binding consumes this Stage 0 namespace without probing ordered fallback candidates.

Normal modules may depend on owned ordinary files/directories, direct child normal modules, visible support packages, registered Core/Builder/dependency packages and explicitly permitted provider files.

They cannot depend on parents, ancestors, normal siblings, grandchildren directly, sibling descendants, unrelated branches or private files of another module. A child facade re-exports deeper declarations needed by its parent. Legal normal-module topology is acyclic by construction, with a defensive graph cycle validator retained.

### Scoped support packages

A support root exposes a package named by its containing directory:

```text
site/
  @site.moth
  markdown/
    +package.moth
    parser/@parser.moth
    rendering/@rendering.moth
  pages/
    @pages.moth
    article/@article.moth
```

`@markdown` is visible to `site`, `pages` and `article`. Consumers see only its public facade, never private paths such as `@markdown/parser`.

For support package `S` whose nearest ancestor normal module is `P`, `S` is visible to `P`, normal sibling modules of `S` and their descendants. It is not visible above `P` or outside that subtree. Its private implementation descendants do not depend on their own facade. Same-owner-scope support siblings cannot depend on one another.

The support facade may depend on owned files, any descendant module in its private subtree, strictly outer-scope support packages and registered packages. It cannot depend on its parent, normal sibling consumers or same-scope support siblings.

The same name may occur in disjoint scopes. Overlapping visible scopes are rejected with both locations. Direct normal-sibling dependencies remain outside the accepted topology rather than becoming an implicit fallback.

### Project package facade

The optional project-root facade compiles as an ordinary API-only module with a special Stage 0 assembly namespace rooted at `entry_root`. It may define its own declarations and reference descendant public interfaces below that root regardless of ordinary lexical module visibility.

The facade never bypasses public boundaries, is invisible to internal modules, cannot import `@project` and cannot expose prohibited project-dependent facts. Provider edges ensure its dependencies compile first. It has no root runtime activation or route.

`ProjectPackageAssembly` is a separate link plan over the compiled facade, selected descendant interfaces, reachable generated functions and permitted runtime requirements. It never recompiles or mutates the facade. A project may be both application and package. Without this facade it has no external Moth package surface. Its package identity uses `$project`'s `name`.

### Namespace and collision policy

Dependency resolution has no precedence, nearest-match shadowing or ordered fallback. Reject overlapping visible identities among project globals, support packages, child modules, extensionless files, internal directory segments, external project package name, Core/Builder roots and dependency aliases, including case-only variants.

Recognised extensionless source kinds share a namespace. `docs.moth`, `docs.mtf`, `docs.md` and `docs/` cannot coexist where each means `@docs`. An explicit-extension provider file may share a stem with a directory only when syntax stays unambiguous. Diagnostics identify every conflicting declaration and the overlapping scope.

### Package classification

Package origin and backing are independent:

| Axis | Values |
|---|---|
| Origin | Core, Builder, ProjectLocal or Dependency. |
| Backing | MothSource or ExternalBinding. |

For example, `@html` is Builder/MothSource, a virtual Core IO package is Core/ExternalBinding, a support facade is ProjectLocal/MothSource and an annotated project JavaScript file is ProjectLocal/ExternalBinding.

Classification records provenance/implementation. It changes no namespace precedence, visibility, facade privacy, dependency syntax or receiver semantics. Source and binding registries remain separate because their discovery and runtime contracts differ.

Precompiled is a storage state, not another backing kind. A precompiled Moth Wasm artefact remains MothSource. An imported WIT component is ExternalBinding.

### Dependency package graphs

Each source dependency compiles in its own package graph, with its own config, private project globals, source index, immutable module artefacts, external facade and compatibility facts. Its private implementation never joins the consuming project graph or sees the consumer's `@project`.

Dependencies use the selected builder's frontend capability surface and the directly linked compilation's `NumericProfile`. Compatibility records the Core/Builder interfaces actually used, not just a builder name. Incompatible precompiled numeric profiles require a matching build or rejection.

An external facade may expose no declaration whose public semantic facts or reachable executable implementation depends directly or transitively on private project globals. This includes constants/defaults, types, generic templates/bounds, trait evidence, receivers, all access/effect/lifetime summaries, generated code and compile-time-derived implementation facts. An exported function calling a private project-dependent helper is also prohibited.

Project-dependent declarations remain internal and unreachable from the external surface. Their dependencies still participate in implementation/compatibility fingerprints. Public projection and executable provenance come from compiler-owned facts, not source rescanning or implementation-only specialisation exceptions.

Each package owns its build-input namespace. Consumer CLI/programmatic inputs never supply dependency contracts implicitly. A dependency resolves its own config/defaults and compatible builder globals. Source and precompiled forms preserve this semantic model.

Project dependency declarations and file-local clauses have separate roles. A project declaration names a direct external package and may assign a local root alias before Stage 0 graph construction. Source clauses bind only already registered roots. They neither acquire undeclared packages nor expose transitive dependencies. Build-input contracts do not declare, alias or enable packages.

Canonical package identity stays separate from a local alias. Aliases affect binding/diagnostics, not artefact or resource-output identity, and collisions are checked before source compilation. The declaration carrier and resolver/catalog handoff remain design-gated under `docs/roadmap/plans/package-dependency-declarations-and-manager-foundations-plan.md`. No config-import preamble is implied. Remote fetching, registries, version solving, lockfiles and publishing remain open design.

### Core and Builder source package graphs

Source-backed Core/Builder packages also compile as separate immutable graphs under the consuming numeric profile. They do not receive the project's globals. Moth-native `Int`/`Float` signatures remain native contracts rather than being rewritten to physical foreign carrier types.

A package needing project-specific compile-time input receives an explicit builder-owned synthetic interface declared in capability metadata. It is not `@project`, is never implicitly injected, carries provenance/fingerprints and makes the resulting artefact project-specific. Otherwise compatible pure package artefacts remain reusable.

Binding-backed Core/Builder packages remain virtual semantic interfaces without source module graphs.

## Deterministic scheduling and graph outcomes

### Compile waves

Stage 0 builds dependency-ordered waves. A source provider publishes before a consumer that needs its interface. Ready jobs may run in parallel only with deterministic identities and canonical delta merging independent of completion order.

For each job:

```text
ready module and completed provider interfaces
-> build compiler input and call the canonical service
-> inspect success, diagnosis or operational failure
-> on publishable success, merge string delta
-> merge path delta using the string remap
-> remap retained module/generated/resource payloads
-> atomically publish the complete result
```

Diagnosed jobs merge and remap their diagnostic identity data in canonical order without publishing semantic lanes. Source deltas merge in source order and module deltas in canonical module order. Warnings/diagnostics never follow worker completion time. Completed artefacts share the final boundary path lookup context when they retain its IDs. Report contexts freeze under the source/identity lifetime contract, including diagnosed outcomes.

Directory modules and synthetic single-file compilation use the same semantic service after their preparation paths. Generated convergence and local lifetime completion stay inside it. Later link planning validates cross-module and builder-lifecycle relationships.

### Graph compilation outcome

Graph outcomes separately retain successful artefacts, diagnosed modules and blocked modules. A blocked record identifies the required failed provider and is not a cascade source diagnostic by default. Blocked modules are not semantically compiled. Independent branches may continue and a shared failing module is diagnosed once.

Operational failure aborts the owning compilation through `InfrastructureFailure`. A proven compiler bug also ends it, using the compiler-bug boundary rather than an ordinary diagnosed outcome. Neither publishes partial semantic state.

### Success-only `ProjectCompilation`

A linkable `ProjectCompilation` contains the canonical structure, project globals, successful module artefacts, generated sidecars and selected entry/package assembly inputs. Every artefact required by those roots has succeeded.

`build`/`dev` never invoke backends for a diagnosed required module, generated transaction or package surface. `check` may retain independent successes for analysis without presenting them as a partial linkable project. Graph structure prevents duplicate compilation/diagnostics rather than relying on renderer deduplication.

## Command and tooling policies

### `build`

Build compiles the union required by selected artefact entries, any project package facade, transitive source providers, required Core/Builder/dependency packages and generated work. After successful compilation it performs link/target/memory validation, lowering and owned output emission.

HTML application build requires at least one `$page` under `entry_root`, not necessarily a homepage. Synthetic single-file HTML output requires `$page` on that root. A package facade is not an implicit page.

### `dev`

The first build compiles the complete selected graph under build's page policy. Subsequent requests reuse immutable successes under `Incremental and persistent artefacts`. Changed inputs invalidate the appropriate semantic, physical or output facts.

Development tooling launches the real homepage when present, otherwise the lexicographically first normalised page route. It invents no homepage, redirect or route alias. Requests to an absent root retain normal missing-route behaviour.

Each compilation request uniquely owns its mutable worker state. A long-lived host may catch a compiler-bug unwind only at that boundary, discard all failed worker state and retain the outer service. Host state updates after the worker joins. No panic occurs while holding shared host-state locks and no partially mutated compiler context is reused. Operational failures use their typed result rather than panic catching. The data-layout authority owns the full isolation contract.

### `check`

Check compiles every discovered project module below `entry_root`, check-only orphan units, the project facade and required packages/generated work. A valid zero-page tree or declaration-only synthetic root is allowed. Unpurposed root runtime/output still fails.

Actual linkable roots receive the same selected-target and memory-plan validation as build. Check performs no backend code generation or output writing. Unsupported features in unreachable private functions impose no target failure.

### Tooling overlays

Check and LSP-style analysis overlay the selected builder's capabilities. They may add diagnostics, lints, analysis output, tooling configuration and callable validation roots. They do not duplicate packages, source kinds, directives or binding metadata. Tooling metadata alone creates no artefact entry.

## MON tooling and static assets

The standalone Rust crate `moth-mon` owns the MON literal-data codec, with `moth::mon` as a supported convenience re-export of the same API and types. It consumes the shared `moth-lexical` policies without depending on compiler or build services. Its ownership, schema and error contracts live in the compiler design's `Rust-only MON service`.

Callers own input IO and supply complete text or values plus a prepared schema. Compiler/build callers pass their boundary's selected numeric profile into schema preparation. Independent Rust callers select a schema profile themselves or use the standard default. The codec returns owned results and performs no input/output writing. Crate extraction creates no new builder, command, tooling overlay or compilable source kind.

A `.mon` file is literal data, not a compilable Moth source kind or semantic provider. It may be referenced as an ordinary resource under the explicit-extension resource rules. Parsing MON never discovers dependencies from its strings.

The accepted static MON builder direction consumes the standalone codec and emits compile-time-known assets through ordinary resource planning, output records and manifest ownership. Exact command/directive and Moth-native integration contracts remain open. This direction creates no alternate parser, resource registry or output-writing path.

## Generated-function boundary

Each module receives an immutable view of published generated identities/summaries. The compiler returns its completed generated delta atomically with the module result. Build code publishes, stores, places and reuses complete sidecars without mutating base/generated HIR or installing summaries.

The compiler completes native success/error channels, private failure rewrites and their final link-fact refresh before handing over the result. Canonical parameter contracts distinguish ordinary values, wires and constructor routes across publication. Module-local expression IDs, binding identities and capture storage never become build-level or cross-module keys. Frozen generic syntax/context remains compiler-owned and can support later materialisation after an earlier validation body's expression storage has been released.

Nested requests converge within the compiler transaction. They create no extra source jobs or wave edges. Cross-package instances belong to the consuming boundary and retain numeric-profile compatibility. Active-request selection, semantic deduplication and diagnosed transaction rollback follow the compiler authority, not a build-side post-filter.

## Entry and package link planning

### Entry candidates and selection

Retained `$page` presence identifies an HTML candidate before argument folding, including metadata-only/empty pages and descendant pages not imported by the site root. It becomes an entry only after ordinary module and metadata validation. Entry eligibility requires no non-empty HIR, artificial fragment or generated runtime.

Use the normal-root inventory and retain unpurposed executable candidates for diagnosis. Ordinary files and private dependency-package roots do not become application pages. Runtime work or fragments alone do not select an entry.

A canonical normal module may support multiple entry assemblies under a builder's contract. HTML V1 defines at most one directory-derived route entry per normal module, with no filename-derived page identity or arbitrary route override.

### Entry assembly

An `EntryAssembly` selects an already compiled normal module's dormant `start`, runtime/static fragments, resolved page metadata and entry-owned runtime requirements. It activates only that root, never provider roots reached through dependencies.

Assembly triggers no local parsing, typing, generic inference, HIR construction, borrow checking or local lifetime analysis. Subsequent link-level lifetime validation still instantiates the completed summaries with actual roots/lifecycles. The compiler owns `start`'s non-exported contract: built-in `Error!` alongside its fragment-collection success slots, including for a synthetic single-file root. Builders consume the typed outcome and add no error-fragment channel.

### Package assembly

`ProjectPackageAssembly` selects the compiled facade, descendant public interfaces, reachable generated work and permitted runtime requirements. It changes neither visibility nor public-boundary privacy. A package diagnosis prevents publication/lowering without mutating base artefacts.

### Per-function reachable unions

Build planning computes exact reachable unions from compiler-owned per-function link facts plus selected non-executable metadata. The compiler document owns the fact inventory. Module-wide indexes may accelerate traversal but never replace those facts.

Resource-bearing exported constants and selected static metadata can contribute without executable functions. Generated requests are completed materialisation dependencies, not calls invented by the linker. No source, AST or dependency grammar is reopened to discover runtime requirements.

### Target-validation roots

Explicit roots include an entry's active `start`, its linked call graph, reachable generated functions, externally callable package exports and additional callable roots from the builder/tooling contract.

The compiler validates those roots against build-owned target assignments. Check uses the same planning semantics, then stops before lowering. Root selection and target partitioning never change source semantic identity.

## HTML project builder

The HTML builder owns directory routes, document assembly, browser-runtime policy and mixed JavaScript/Wasm partitioning. These are builder contracts, not core language semantics.

### Entry and fragment assembly

For each validated `$page` entry, the builder plans:

1. Activation of that normal module's compiled dormant root.
2. Compile-time fragments at their recorded runtime insertion indexes and slots for runtime fragments.
3. Exactly one invocation of active `start` through the selected runtime path.
4. Runtime fragment publication in source order, only after that invocation succeeds.
5. Route HTML and companion outputs from folded page metadata and directory routes.

This is generated runtime behaviour, not execution during semantic compilation. Plain JavaScript and mixed output share one ordinary fragment insertion contract. Templates produce String values and retain structural resource/site-root pieces until their normal output context resolves them. Fragment insertion grants no persistent Wiring observation or event-delivery lifecycle.

The generated caller branches on `start`'s typed outcome. Only success publishes that invocation's runtime fragments. Failure leaves static HTML in place and does not undo earlier explicit IO. Release output shows a fixed safe notice, never application `Error` markup. Development presentation stays local to the failing invocation and does not fail the completed compilation or other entries.

The compiler owns `start`'s signature, checks, failure mapping and explicit result control flow. The builder owns the generated caller and terminal presentation. It consumes the typed result and does not reinterpret HIR or catch ordinary Moth errors as host exceptions.

On error, the generated caller reports one unsuccessful invocation and stops normal startup completion. There is no automatic retry, later-statement continuation or rollback. Propagation does not print at every layer. A terminal host writes to standard error and exits with a nonzero status independently of `Error.code`; code zero remains valid error data. An embedding host delivers the failed outcome to its owner. A test runner records runtime failure unless the expected outcome matches it. These rules apply to existing hosts. They do not require a new CLI runner, server-side renderer or embedding API.

Compile-time diagnostics, compiler operational failures, compiler bugs and a compiled program returning `Error` stay different lanes. Reporting a browser entry failure does not fail the completed compilation.

Runtime fragment results for one invocation stay staged until success. On error, discard unpublished results and never read undefined success slots. Staging is not a transaction over the DOM or application state: earlier explicit IO, title changes and host calls remain done. A wholly static page does not gain mandatory runtime code merely because the conceptual `start` contract permits `Error!`. Wrappers stay reachability-driven. An error handler covers this invocation only, not later callbacks.

Release presentation uses a small generic startup-failure notice, escaped as text and independent of the failed application template. It does not place the raw application `Error` message in public page content. Development presentation is specified under `Development entry-error reports`. See the progress matrix for current support.

### Development entry-error reports

In `moth dev`, a `start` `Error` must be unmistakable even when the static page succeeded. Reuse the existing compiler-error presentation: a prominent view covers the affected page and retains hot reload, with a distinct runtime title and category. A console-only report is not enough.

Report in the affected browser and the server terminal. Reuse dev-client injection, origin-aware routing and error-page rendering. The browser-to-server report is the only added path. It carries entry, build and invocation identity, plus bounded category, message, code and available source information.

The protocol is dev-only and same-origin. Treat browser input as untrusted. Validate kind, shape, `U32` code, current build and known entry. Bound payloads and per-client reporting, escape display text and terminal control characters, and reject malformed or stale reports without panicking the server. Respect the configured origin prefix. Do not accept arbitrary HTML or compiler-ID payloads. A reporting failure must not recursively invoke the reporter.

A runtime report does not change global build success, block other entries or give other tabs a compile-error page. Deduplicate by build, entry and invocation. Ignore old-build reports. Clear the originating view on a fresh page invocation through normal reload. Do not clear a failed invocation merely because a compiler rebuild succeeded elsewhere. If the network drops, keep the local browser view and console report and do not retry application startup.

Development publication couples output bytes with build identity. Compile outside the shared publication lock, then hold that lock through the existing output writer and the build-state update. Capture each HTTP response's state and complete output bytes under the same lock, then release it before network writes or reload broadcasts. A write failure publishes diagnostics rather than a successful generation, even if emission changed some files. Failed builds replace page HTML with diagnostics while keeping supporting assets reachable.

A page from generation N can fetch JavaScript or Wasm companions after generation N+1 publishes. Each response captures one generation, but separate requests do not pin an output batch. Reports retain the page's generation N and the server rejects them as stale after N+1.

Recovery uses a generation handshake, not multi-request asset pinning. Every served page, including diagnostics pages, carries its exact generation as text. Each event-stream connection registers before it reads the published generation and announces that generation first. A later publication wakes registered connections to announce the then-current generation, so no publication falls between the snapshot and the subscription. The browser reloads only when an announced generation differs from its page's. A page that missed a broadcast therefore catches up on its next connection or reconnection, and reconnecting to the same generation never reloads. Transport loss neither reloads nor retries application startup.

Faults that escape the generated startup lifecycle wrapper, including owned Wasm instantiation, use a separate category. This covers only work that wrapper invokes or awaits. It adds no Moth async surface, global `window.onerror` hook or global unhandled-rejection capture. Unrelated scripts, extensions and later callbacks stay outside scope.

Three outcomes stay distinct:

- Entry Error: `start` returned its declared `Error!` outcome.
- Assertion failure: compiler-owned assertion identity identifies the fault, including through generated calls and glue. Never match message text.
- Unexpected startup fault: another exception or trap escaped the owned wrapper. It may come from generated code, a host binding or runtime setup. Do not label every runtime exception as a compiler bug.

Unknown faults remain unknown startup faults. Normal typed errors use explicit result branches, not this interception path.

A module without `$page` emits no independent page, but its reachable declarations, resources and runtime requirements may contribute to another entry/package. Unpurposed authored root work is still invalid.

The canonical entry-config and HTML-routing references own document defaults and public metadata shape. A present page title overrides the route/project fallback and receives the initial builder prefix/postfix. Absent fields use documented defaults and `none` differs from a permitted present empty String. Escape title text and attribute values in their output contexts. Insert authored folded `head` as HTML without escaping it or flattening meaningful whitespace. These rules do not form a generic schema merge system.

Runtime `io.set_title` requires an advertised browser document-title capability. It changes live title text without reapplying initial prefix/postfix, changing routes, mutating folded metadata or rewriting static outputs. A non-browser JavaScript host receives the same explicit capability rejection as another unsupported target, never a silent no-op.

### Mixed-target planning and validation

The fixed dependency order is:

```text
entry/package roots and exact reachable facts
-> instantiate lifetime summaries with builder lifecycle roots
-> validate complete lifetime topology
-> complete intervals, frontiers and epochs
-> publish backend-neutral memory requirements
-> compute target affinity and deterministic partition
-> validate assigned targets and cross-target edges
-> create candidate physical-variant scopes
-> refine target/profile families and layouts
-> revalidate affected family-edge facts
-> select and validate the physical memory plan
-> combine validated memory-plan and structural-URL identities
-> complete the physical-variant key
-> lower the validated variant
-> verify collector-free artefacts where required
```

The compiler owns topology, target validation and physical strategy decisions. The builder supplies lifecycle roots, partition and output context, then orchestrates the services. Shared topology/requirements are target-independent. Refinement and physical plans are per variant. The semantic numeric profile is fixed across all variants.

Output-context policies must be available for target capability validation. Concrete URL maps must be validated before final variant identity and lowering. These are build-owned inputs, not decisions delegated to the memory planner. Check performs applicable context, target and memory-plan validation without lowering/output. Output path/conflict validation precedes resource content IO under the resource/output contracts below.

Partition rules:

- `start` is JavaScript-owned.
- DOM, browser, project JavaScript and other JS-required dependencies force their containing function to JavaScript, propagating backwards to transitive callers.
- Neutral console IO does not force JavaScript. Remaining supported functions default to Wasm.
- Wasm-owned Moth functions cannot call JavaScript-owned Moth functions after propagation. JavaScript-owned functions may call Wasm through generated wrappers.
- Every assignment records its reason. Capability metadata, not package-name checks, supplies affinity.
- Partitioning is independent of development/release mode and leaves canonical HIR shared.

Source cannot inspect partition, target, OS or architecture. Unsupported numeric or aggregate runtime features are rejected through target contracts rather than silently weakened or assigned different semantic widths.

### Physical variants

A physical variant is a target-specific implementation of one selected assembly/function set with validated layout, runtime and memory decisions. Deduplication applies only after all required plans are validated.

An HTML Wasm variant is one entry bundle containing the selected Wasm-owned reachable Moth closure across semantic modules. A semantic module is a compilation/visibility boundary, not necessarily a Wasm binary. Separately emitted Moth-source packages use native Moth linking/ABI contracts, not WIT.

The complete conceptual key includes:

- stable entry or package assembly identity
- selected concrete function set and target assignment
- build profile and boundary `NumericProfile`
- ABI and layout identities
- runtime capabilities and relevant backend configuration
- validated memory-plan fingerprint
- relevant normalised resource URL map and site-root rendering policy, or their fingerprints

Reuse requires equality of this full key, including assembly identity. Matching function sets or pre-plan layouts alone is insufficient. This rule defines no extra equivalence between distinct assembly identities. One source function may be JavaScript in one entry variant and Wasm in another.

The memory fingerprint normalises the refined family graph, strategies, inferred/declared placement, affine-responsibility transitions, retained-edge obligation transitions, hidden destinations, REC representation, cleanup/destruction and physical coalescing. Process-local/donor-local indexes are never hashed directly. Coalescing is complete before validation/fingerprinting, not a late lowerer decision.

Each entry physical variant has a JavaScript companion facade. Its selected Wasm closure emits once as the entry bundle. Runtime instances remain page-local even when compatible output bytes are reused.

### Link planning and lifetime topology

Link analysis instantiates local summaries with builder lifecycle roots before target assignment. It carries shared validated topology and backend-neutral requirements, not one project-global physical memory plan. Linking does not reopen source or mutate HIR.

Page, mount, request, frame and arena roots are lifecycle inputs when a consumer contract defines them, not exceptions to source memory law. They let storage outlive a lexical function only when the ordinary single-owner and retained-edge outlives rules hold. The existence of a lifecycle category does not imply a Wiring observer or UI runtime.

Caller/link analysis derives concrete cleanup frontiers from exported effects, aliases, retention domains, future edges and lifecycle roots. Exported summaries stay canonical and carry no donor-local family/region/counter/frontier IDs. Topology-relevant implementation/link changes invalidate affected assemblies.

### Memory-strategy plans

For each target-validated candidate, the build system supplies profile, capabilities, shared topology and requirements to the compiler memory planner. It stores the resulting `ValidatedMemoryPlan` without selecting strategies or deriving cleanup/count transitions itself.

Compiler and memory authorities own plan contents, refinement fallback and publication invariants. The validated plan fingerprint participates in the complete physical key. Shared topology can remain reusable when only a target/profile representation changes.

### Backend capability metadata and collector-free verification

A backend declares whether it supports collector-free release lowering. This is capability metadata, not a source-visible no-GC mode or config field.

A full-control release backend realises every topology accepted by semantic and target validation without a tracing/reachability collector. Physical-planning imprecision cannot become another source rejection or collector fallback. Debug/development may use GC for simpler lowering or instrumentation. GC-native targets may use their host collector in any profile. These choices preserve mandatory access/lifetime legality. Numeric profiles and reachable target capabilities are independent semantic/target inputs.

After lowering, a builder may verify that an artefact contains no tracing runtime. That reports an output property without changing source legality or behaviour.

### Runtime and memory

Each page owns one runtime instance and one memory for its native Moth Wasm entry bundle. The bundle imports that runtime memory. Runtime bytes may be emitted once and instantiated separately for each page.

WIT components remain foreign bindings with private runtime memory and closed value-conversion profiles. Their boundary does not require shared page memory. Separately emitted Moth-source package artefacts remain native Moth packages, not implicit components.

Wasm lowering consumes the explicit function/import/export/capability/layout and validated memory plans. The runtime supports allocator, inferred/declared-region, REC counter and destruction-plan requirements selected by those plans. Lowering encodes planned obligations/discharge sites rather than inventing generic retain/release operations.

Wasm LIR is structured and backend-owned, not another frontend semantic authority. The architecture has no durable dispatcher-loop backend, per-semantic-module memory, `moth_start` export, helper-export booleans or unconditional Int-as-i64 bridge. Valid I64/U64 and profile-selected 64-bit Int lowering remain required numeric cases.

### Lowerer use cases

The HTML JavaScript lowerer emits the selected function set and demanded runtime helpers. Shared entry assembly owns fragment/document behaviour. The page companion emits only the external wrappers and imports required by that entry.

A standalone JavaScript request may explicitly select all HIR functions. The standalone Wasm backend owns HIR-to-Wasm-LIR lowering, request validation, runtime contracts, binary emission and backend debug output. HTML-Wasm orchestration wraps that backend rather than becoming another compiler pipeline.

### External JavaScript

Build-wide provider emission deduplicates runtime assets, module specifiers and shared provider files. Entry-level glue contains only wrappers, import preambles and import-map entries required by the selected JavaScript bundle.

Direct builder bindings and provider-created bindings share stable binding identity and runtime asset policy. No per-entry scan recreates source dependencies.

### Resource emission

The HTML builder assigns URL contexts, plans exact resource outputs, deduplicates origins/sources/reads and produces resource warnings from semantic use locations. It returns ordinary output records. The following section owns the general resource contract.

## Resource linking and output placement

### Graph activity versus emission

Prepared-source orchestration registers graph-active inputs and watch interests before reachability. A resource unused by any selected output remains watchable but is never read, hashed or emitted. Content-source compilation and ordinary resource-byte IO are different operations.

SiteRoot is a structural string piece, not a resource. It has no origin, byte source, filesystem target, watch record or resource-union membership. Builders render it from the final consuming artefact's project-origin policy, including when it arrives in a dependency's exported constant. They never prepend that origin to ordinary resource URLs.

A reachable site-root use requires an explicit policy or target rejection. The backend never guesses `/`. Separate site-root-use facts from functions and metadata drive capability checks, physical specialisation and static rerendering. Changing origin policy invalidates affected physical/output variants, not public semantic interfaces.

### Boundary-wide source registry

One build registry aggregates project, source-package and provider resource results within the consuming compilation boundary. It has one record per stable semantic origin and one byte-source record per canonical file or generated payload.

Several origins may share a byte source. Equal origins must agree on byte-source facts. Physical paths are IO facts only and content hashes are output-invalidation facts. Successful module deltas merge transactionally. Diagnosed modules publish no origin association.

Preparation/semantic validation never read or hash ordinary resource contents. Output placement/conflicts are validated before emission-time content metadata, hashing or byte reads. This restriction does not forbid the filesystem metadata already needed for discovery and containment checks.

### Exact resource unions

Entry unions include selected `start`, reachable source/generated functions, runtime/static fragments, selected page metadata and reachable provider runtime requirements. Preserve authored metadata/use spans for diagnostics.

Package unions include selected exports, resource-bearing exported folded values, reachable implementations and provider runtime requirements allowed by that target. Compute unions from link facts and metadata, without rescanning HIR. Unused private resources contribute nothing.

### Output placement

Project-local resources preserve their paths relative to `entry_root`. Source/Core/Builder/dependency package resources use an injective package prefix followed by the package-relative path. Provider-managed/generated resources use their declared stable output paths and byte sources.

One encoder owns package prefixes. Injective means distinct stable package origins/names/instances cannot map to the same prefix. Consumer aliases never change output identity. A future package-instance policy must remain part of that identity rather than relying on display names.

### URL contexts

A URL context is the artefact whose resolution rules observe the string, not necessarily the JavaScript or Wasm file containing its generated code. Page HTML, inline CSS and ordinary page runtime strings use the page document. Standalone CSS uses its stylesheet. Other sinks/builders need explicit policies.

A reachable use without an assignable context is rejected before lowering. URL rendering uses the validated output path, computes a lexical relative path from the context artefact's parent, uses `/`, percent-encodes UTF-8 segments and prefixes same/descendant paths with `./`. Parent-relative `../` remains valid in output URLs. Project HTML origin is never prepended.

Functions constructing structural runtime Strings may therefore lower differently by entry context. Their normalised URL maps participate in physical variant identity, not canonical HIR, public type identity or source legality. Static fragments use the same placement authority.

### Conflicts and content IO

An origin used by several entries emits once where output placement matches. Equal output path and equal origin deduplicate. Equal output path with different origins fails, even when bytes happen to match, with both useful use locations.

Resources participate in the same conflict checks as HTML, CSS, JavaScript, Wasm, manifests and provider outputs. An unchanged provider use and an ordinary resource use deduplicate only when origin and path both agree. Transformed/generated outputs have distinct identity.

All output paths/conflicts validate before content hashing, emission-time metadata checks and byte reads. Diagnostics use semantic origins and authored uses, never reconstructed rendered strings. Read each physical source once per build state and hash reachable bytes once. One read may feed several distinct output records.

### Invalidation

Stable-origin changes follow semantic fingerprint rules. Byte-only changes invalidate content/output and affected provider transforms without recompiling semantic consumers. Route, output-root, origin-policy and URL-context changes replan affected physical/output facts without reopening source legality.

## Output ownership

Artefact builders own output settings/defaults through their config contracts. Non-emitting builders register none. HTML defaults to `dev` and `release`, with overrides through `$html_builder`.

Output roots are relative to the project root, outside `entry_root`, free of parent traversal and contained by project output policy. The build system validates paths/conflicts, skips unchanged writes, maintains manifests and removes stale artefacts. Builders and lowerers return records rather than writing final outputs directly.

Manifest ownership is keyed by stable builder identity and build profile. Development and release cannot silently claim the same root. A conflicting existing owner is diagnosed before writes. One builder never deletes another manifest's files, and normal invocations have no force-overwrite escape hatch.

### Deliberate output pipelines

Output transformations such as minification require an explicit ordered pipeline. A stage consumes its predecessor's manifest, declared inputs and a bounded output contract. The final manifest records the complete pipeline identity. Independent builders cannot simulate composition by overwriting one another.

Exact pipeline syntax remains open design. The ownership requirement applies regardless of its eventual syntax.

## Incremental and persistent artefacts

The compiler defines semantic fingerprint contents. The build system owns invalidation, relinking and compatibility over those facts. A fingerprint is a stable summary used to detect changes, not a second semantic representation.

### In-memory reuse

The first development build compiles the full selected graph. Later builds reuse successful immutable artefacts. A changed module rebuilds. Semantic consumers rebuild only when a provider's public-interface fingerprint changes, including any exported access/effect/lifetime fact. There is no separate exported-effect fingerprint.

Entries relink or regenerate when their implementation, dormant-root, runtime-dependency, generated, entry-settings, project-field or relevant backend-config inputs change. A private body change need not recompile semantic consumers. Dormant root changes affect entries that activate that root. Runtime-dependency changes update capabilities, glue and resource planning.

Configuration and numeric-profile dependence uses existing fingerprint domains, including private implementation and compatibility. Static Bool selection creates no new fingerprint family and never changes declaration identity. Project-field dependencies remain field-granular. Documentation-only changes update documentation/editor indexes without invalidating semantic consumers or executable variants.

The M02 reuse gate requires evidence that a semantic or physical cache reused an artefact before testing invalidation. Cover private effect/body changes, public contracts, entry outcome changes, generated sidecars and backend/profile/ABI compatibility against that demonstrated reuse. A cold recompilation or a writer's skipped-unchanged result alone cannot satisfy this gate. Rebuild/write/serve tests prove current output freshness separately, including exact entry cause codes and diagnosed exports without treating old successful output as the new result.

### Physical variant invalidation

Changed memory-plan inputs invalidate affected variants without necessarily recompiling modules. Shared topology/requirements may survive replanning. Different plan fingerprints cannot reuse emitted code even when other pre-plan facts match.

URL maps and site-root policies follow the same physical/output separation. Resource bytes follow content invalidation. A provider public-interface change still invalidates semantic consumers independently of these physical changes.

### Persistent compatibility

Persisted modules, packages and generated artefacts preserve the same semantic boundaries. Reuse requires compatible compiler artefact/language versions, stable project/package identity, source/config fingerprints, dependency interface fingerprints, used Core/Builder capabilities, frontend feature selection, numeric profile, relevant ABI/layout policy and generated request identity where applicable.

Persisted physical variants also require the complete validated physical identity, including memory-plan and applicable URL-map/site-root fingerprints. Process-local IDs and absolute paths are not compatibility keys. Canonical identities and self-contained or remappable lookup data preserve meaning across processes.

Incompatible artefacts are discarded and rebuilt. Normal builds perform no best-effort deserialisation, partial migration or compatibility repair. Exact persistent encoding remains a separate design choice constrained by these rules.
