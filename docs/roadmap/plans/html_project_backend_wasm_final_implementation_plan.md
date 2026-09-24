# HTML mixed JavaScript and Wasm backend implementation plan

## Purpose

Implement the HTML project builder mixed JavaScript and Wasm backend strategy, final Wasm LIR and runtime design, and structured ABI. Replace the current experimental whole-module Wasm path with function-level partitioning, generated JavaScript companions and page-local shared Wasm runtime.

## Status

- Status: queued.
- Current slice: activation inventory not started.
- Blockers: delivered canonical module/link inputs, unified numeric semantics and the required physical-layout/runtime capabilities.
- Next action: refresh owners and record the activation revision and validation results in local working notes.

## Hard prerequisites

- delivered final TIR one-store/exact-view architecture
- canonical module compilation with immutable artefacts and graph-aware backend handoff
- per-function link facts and target validation roots
- unified numeric types with a compilation-wide NumericProfile, fixed scalar JS/Wasm execution, runtime Byte and U32 Error.code

## Required authority documents

- `docs/compiler-design-overview.md` for HIR, borrow facts, target validation and per-function link facts
- `docs/build-system-design.md` for HTML project builder, mixed-target planning, physical variants, runtime and memory, output ownership
- `docs/src/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `docs/src/docs/progress/@page.moth` for current support
- canonical package/module references for the graph contract
- canonical numeric and cast references for profile, fixed-width, Byte and Number semantics

## Activation snapshot to refresh

The following are legacy implementation locators, not accepted final architecture or claims about the activation tree. Earlier numeric work replaces scalar bridges before this plan starts. Inventory what actually remains rather than deleting a legitimate I64/U64 or Int64 path because it shares an old name.

Legacy build handoff:

- `BackendBuilder::build_backend` receives `modules: Vec<Module>`, not a graph
- `Module` carries `entry_point`, `hir`, `type_environment`, `borrow_analysis`, `warnings`, `const_top_level_fragments`, `entry_runtime_fragment_count`, `external_package_registry` and `module_external_imports`
- `FileKind` supports `Wasm(Vec<u8>)`, `Js(String)`, `Html(String)` and raw bytes

Legacy HTML builder:

- `HtmlProjectBuilder::build_backend` loops over modules and compiles one module at a time
- backend mode is selected through `Flag::HtmlWasm`
- `compile_one_module` validates the module for either JS or Wasm as a whole

Legacy Wasm mode:

- exports `start` as `moth_start`
- bootstrap instantiates `page.wasm`, calls `moth_start`, reads returned handles and hydrates slots
- helper export structs are duplicated between HTML-Wasm and core Wasm request types

Legacy Wasm LIR:

- `WasmLirFunction` contains a flat `Vec<WasmLirBlock>` with `Jump` and `Branch` terminators
- an unconditional Int-as-I64 ABI bridge and bridge-like `StringFromI64` instruction predate the numeric profile
- user functions are emitted through a dispatcher-loop strategy using an artificial program counter
- the emitter defines memory inside each emitted module

Legacy JS backend:

- has three lowering configs: direct JS, HTML page bundle, HTML-Wasm companion
- function emission policy is all-functions or reachable-from-start

## Required inputs

Consume these completed compiler/build inputs. The referenced permanent documents own their contracts.

- success-only `ProjectCompilation` (see `docs/build-system-design.md` "Success-only ProjectCompilation")
- compiled module artefact lanes (see `docs/compiler-design-overview.md` "Compiled module artefact")
- generated sidecars (see `docs/compiler-design-overview.md` "Generated concrete functions")
- entry and package assemblies (see `docs/build-system-design.md` "Entry and package link planning")
- per-function `ModuleLinkFacts` (see `docs/compiler-design-overview.md` "Per-function link facts")
- explicit target-validation roots (see `docs/build-system-design.md` "Target-validation roots")
- build-owned target assignment (see `docs/build-system-design.md` "Mixed-target planning and validation")
- stable source and binding call identities (see `docs/compiler-design-overview.md` "Stable semantic identities")
- selected builder capability metadata (see `docs/build-system-design.md` "Selected command and capability surface")
- backend config fingerprints (see `docs/build-system-design.md` "Physical variants")
- output manifest ownership (see `docs/build-system-design.md` "Output ownership")
- explicit root-only HTML entry metadata and purpose facts from compiled module metadata (see `docs/build-system-design.md` "Root metadata and purpose directives" and "Entry candidates and selection")
- validated `$layout` representation contracts when delivered. ABI and struct-lowering phases honour offsets, alignment and those contracts without inventing a new nominal type category
- the early-selected NumericProfile, canonical numeric identities, scalar size/alignment/stride facts and explicit numeric failure/conversion HIR

### Numeric handoff

Consume the scalar implementation already delivered by the numeric checkpoint:
I8/U8/Byte use one memory byte, I16/U16 two, I32/U32/F32 four and I64/U64/F64
eight. Narrow integers and Byte use i32 carriers. F16 stores two bytes and uses
an f32 carrier with explicit binary16 conversion. Int and Float follow the
selected numeric profile, not the selected backend. Reuse existing scalar
checks, conversions, formatting and helpers rather than implementing a second
numeric runtime while restructuring control flow.

NumericProfile is fixed before semantic compilation and shared by every direct
Moth dependency and every JS/Wasm partition. A physical variant may not choose a
different Int width or Float precision. Carry profile compatibility through ABI,
layout and variant identity. Preserve deliberate Core/Builder Int/Float language
signatures separately from foreign fixed-width ABI types. Error.code is U32 in
both targets, including values above I32::MAX.

Number/NumberN have frontend and HTML-JS support but no Wasm arbitrary-precision
runtime from that checkpoint. Retain precise reachable target rejection until a
separate runtime delivery exists. This plan does not add that runtime or weaken
Number's exact scale and per-operation rounding rules.

## Mixed-target sequence

Keep this sequence explicit:

```text
entry or package roots
-> exact reachable function and link-fact union
-> target-affinity and capability analysis
-> deterministic target assignment
-> compiler target validation against that assignment
-> permitted cross-target edge validation
-> selected backend lowering
-> physical variant and output planning
```

`check` performs the same planning and validation, then stops before lowering.
Use the full lifetime and physical-memory planning order in the build authority
when implementing this abbreviated target-flow outline.

## Required final design

See `docs/build-system-design.md` "HTML project builder", "Mixed-target planning and validation", "Physical variants" and "Runtime and memory" for the full contracts.

Retain:
- JavaScript-owned `start` (the implicit `start` is lowered to JS, not Wasm)
- backwards propagation of JavaScript requirements (DOM, browser, project JS force JS ownership, requirements propagate to transitive callers)
- no Wasm-to-JavaScript Moth call after propagation
- JavaScript-to-Wasm wrappers (JS-owned functions may call Wasm-owned functions through generated wrappers)
- explicit partition reasons (every decision records why)
- entry-specific partition (partitioning is per-entry, not global)
- physical variant keys (deduplicate by entry or package assembly identity, function set, target assignment, profile-compatible ABI, layout, capability requirements and backend config fingerprint)
- one Moth Wasm entry bundle per physical entry variant. Semantic modules do not force separate Wasm modules
- page-local runtime and memory (each page owns one runtime instance and one memory for its Moth Wasm entry bundle)
- generated JavaScript companions (each entry physical variant has a JS companion facade)
- structured derived HIR view (backend-neutral, derived, cached only as derived data)
- structured backend-owned Wasm LIR (not a second frontend semantic authority)
- imported runtime memory (the entry Wasm bundle imports page runtime memory)
- WIT is reserved for foreign component bindings and optional foreign projections. Moth-source dependencies stay Moth-to-Moth
- explicit selected-function, import, export, capability and layout plans
- central output writing (builders produce output records, build system writes)
- no compatibility adapters

The final design removes (deleted rather than retained through compatibility adapters):
- dispatcher-loop control flow as the durable backend shape
- `moth_start`
- per-module memories
- helper-export booleans
- the legacy unconditional Int-as-i64 bridge, not valid 64-bit numeric lowering
- flat basic-block Wasm LIR
- whole-module JS or Wasm mode
- global `HtmlWasm` mode as durable architecture
- current runtime helper duplication

## Non-goals

- no Wasm-to-JavaScript Moth call
- no per-module Wasm memory
- no dispatcher-loop LIR
- no unconditional Int-as-i64 bridge
- no `moth_start` export
- no whole-module JS or Wasm validation mode
- no compatibility adapters for old paths
- no standalone Wasm output pipeline design beyond the HTML builder orchestration
- no WIT component import or export implementation beyond preserving its foreign-binding boundary
- no conversion of Moth-source package interfaces to WIT
- no second scalar numeric implementation, per-partition NumericProfile or Wasm Number runtime

## Risks and blockers

- canonical graph-aware compiler inputs must exist before this plan consumes them
- Wasm LIR redesign is a large structural change that must preserve all existing semantic contracts
- partition correctness depends on accurate per-function link facts from the compiler
- ABI and layout design must coordinate with the runtime memory model

## Implementation phases

Each phase must leave one coherent path. Reference `docs/build-system-design.md` "HTML project builder" and "Mixed-target planning and validation" for full contracts.

### Phase 1: Current owner and test baseline

Context: refresh all code anchors and establish a test baseline before the structural refactor.

- Confirm final TIR, canonical module compilation and the numeric scalar checkpoint are accepted.
- Confirm the backend consumes only neutral owned runtime template payloads and ordinary HIR string operations. No TIR identity, view, overlay or preparation state reaches link planning or lowering.
- Record `git rev-parse HEAD`, branch and `git status --short` in local working notes.
- Inventory current `BackendBuilder::build_backend`, `HtmlProjectBuilder`, JS backend, Wasm backend, LIR, emission and runtime owners.
- Identify already-removed numeric bridges and existing fixed-width helpers. Reuse them.
- Run baseline `just validate` and `just bench-check`. Record results.

### Phase 2: Graph-aware HTML link planning

Context: the HTML builder must consume entry assemblies and linked module calls rather than looping over a flat module list.

See `docs/build-system-design.md` "Entry and package link planning" and "Per-function reachable unions".

- Replace any remaining `BackendBuilder::build_backend(Vec<Module>, ...)` with consumption of `ProjectCompilation`.
- Build HTML link plans from entry assemblies and per-function link facts.
- Compute exact reachable function and runtime-fact unions for each entry.
- Plan one Wasm selected-function closure per entry physical variant. Do not map semantic modules one-to-one to Wasm binaries.
- Keep backend-facing access narrow: module artefacts, generated sidecars, entry assemblies, package facade and runtime metadata.
- Remove source-path graph reconstruction from the builder.

### Phase 3: Target affinity, partition and compiler validation

Context: replace whole-module JS or Wasm mode with function-level partitioning.

See `docs/build-system-design.md` "Mixed-target planning and validation" for partition rules.

- Compute target affinity from semantic package and capability metadata.
- Apply partition rules: `start` is JavaScript-owned, DOM and browser JS force JavaScript, JS requirements propagate backwards, no Wasm-to-JS Moth call after propagation, JS-to-Wasm wrappers, remaining functions default to Wasm.
- Every decision records an explicit reason.
- Partitioning is entry-specific and independent of development or release mode. NumericProfile is already fixed and never changed by partitioning.
- Run compiler target validation against the completed deterministic partition.
- Validate every function against its assigned target.
- Validate permitted cross-target edges, including fixed-width and profile-selected scalar conversions at the wrapper boundary.
- `check` runs the same sequence and stops before lowering.
- Preserve Number's target restriction until a real Wasm runtime supports it.
- Carry assertion-message capability facts through the selected-function partition: JavaScript-owned
  functions retain lazy runtime message evaluation, while enabling dynamic messages in Wasm requires an
  explicit selected-target representation and failure-presentation path (including the HTML handoff) before
  target validation may be relaxed.

### Phase 4: Selected JavaScript emission and companions

Context: JS backend needs a partition-selected emission mode, not all-functions or reachable-from-start.

- Lower only the selected JavaScript function set.
- Emit required runtime helpers only for selected functions.
- Render compile-time fragments into the document.
- Emit runtime fragment slots.
- Invoke active `start` once through the selected runtime path.
- Hydrate runtime fragments in source order.
- Generate JavaScript companion facades for entry physical variants.

### Phase 5: Page runtime and entry memory

Context: each page owns one runtime instance, one memory and one Moth Wasm entry bundle.

See `docs/build-system-design.md` "Runtime and memory".

- Generate one page-local Wasm runtime instance per entry.
- Emit one Moth Wasm bundle for the selected Wasm-owned function closure of each entry physical variant.
- Bundle ordinary Moth module dependencies into that entry output rather than creating per-module Wasm modules.
- Import runtime memory into the entry bundle.
- Keep imported WIT components separate with private component memory.
- Keep separately emitted Moth-source package artefacts on Moth-native link and ABI contracts rather than WIT.
- Project-level runtime bytes may be emitted once and instantiated separately for each page.
- Remove per-module memory section emission for user modules.

### Phase 6: Structured HIR view and structured Wasm LIR

Context: replace flat basic-block LIR with structured, emission-shaped IR.

See `docs/build-system-design.md` "Runtime and memory" for the LIR contract.

- Add a backend-neutral structured HIR view derived for structured lowerers (cached as derived data only).
- Replace `WasmLirBlock` flat block model with structured body-tree LIR.
- Remove `Jump` and `Branch` terminators from final LIR.
- Make Wasm LIR structured and backend-owned. It is not a second frontend semantic authority.
- Generate runtime helper functions through structured Wasm LIR like user functions.
- Preserve numeric check order, conversion boundaries and scalar carrier/storage distinctions while moving their existing lowering into the new structure.

### Phase 7: ABI, layouts and runtime helpers

Context: consume the delivered scalar ABI and complete aggregate/runtime lowering.

- Remove any remaining unconditional Int-as-I64 bridge. Int uses i32 or i64 according to NumericProfile. Keep I64/U64 and legitimate 64-bit formatting support.
- Remove the legacy `StringFromI64` bridge instruction where it still exists. Reuse semantic numeric formatting rather than replacing every 64-bit formatter with `StringFromI32`.
- Remove `Void` as a real ABI type. Represent no result as `results: []`.
- Consume scalar ABI mappings and complete mappings for handles, strings, collections, structs, choices, options and errors.
- Consume compiler-owned physical struct layouts: field offsets, alignment, scalar widths/strides and representation constraints. Lower construction, field access, mutation and ownership hooks from those facts.
- Honour compiler-owned `$layout` contracts when present. Do not manufacture a new nominal type category, a universal C ABI or SoA layout here.
- Preserve one-byte Byte/U8/I8 fields, two-byte I16/U16/F16 fields and compact scalar collection strides rather than widening stored values to their carrier width.
- Error payloads retain `code U32 = 0` with i32 carrier semantics. Values above I32::MAX remain valid.
- Document-title capability belongs to JavaScript affinity where the host advertises it. This mixed-backend plan does not implement `io.set_title`, feature selection or `$export` migration.
- Complete Wasm choice lowering from validated tag/payload layouts: construction, equality, matching and generic choices.
- Design runtime string model: allocation, UTF-8 layout, interpolation helpers, host string extraction and release hooks. Keep backend ownership here. Defer profiling-led string-building work to its existing owner without moving output assembly into TIR.
- Remove helper-export boolean structs. Replace with runtime capability and helper import plans.

### Phase 8: External packages and cross-target glue

Context: external JavaScript glue must be demand-driven per entry.

See `docs/build-system-design.md` "External JavaScript".

- Build-level runtime emission deduplicates runtime assets, required module specifiers and shared provider runtime files.
- Entry-level glue generation emits only wrappers for external functions referenced by the selected JavaScript bundle, required import preambles and import-map entries.
- Direct builder packages and provider-created packages use the same binding identity and runtime asset model.
- Keep foreign I*/U*/F* widths distinct from deliberate Moth Int/Float signatures. Preserve finite validation, F16 conversion boundaries and U32 runtime error codes.
- Reserve WIT/component handling for a later foreign-Wasm binding delivery. This phase must not encode Moth-source package interfaces as WIT.

### Phase 9: Physical variants, manifests and output ownership

Context: partitioning is entry-specific and physical variants are deduplicated by a conceptual key.

See `docs/build-system-design.md` "Physical variants" and "Output ownership".

- Deduplicate completed validated physical variants by entry or package assembly identity, selected concrete function set, target assignment, profile-compatible ABI identity, layout identity, memory-plan fingerprint, runtime capability requirements and relevant backend config fingerprint.
- Entries with the same key reuse one variant.
- One source function may be JavaScript in one entry variant and Wasm in another, with the same NumericProfile semantics.
- Each entry physical variant has a generated JavaScript companion facade and one Moth Wasm bundle for its selected Wasm-owned closure.
- Central output writing with manifests, stale cleanup and conflict diagnostics.
- Output ownership is keyed by stable builder identity and build profile.
- Reject an output root containing a manifest owned by another builder.
- Reject an output root containing a manifest owned by another build profile, including development versus release for the same builder.
- Keep stale cleanup scoped to the selected builder and profile owner.
- Do not add a force-overwrite escape hatch.
- Keep shared-root transformation unavailable until an explicit ordered output-pipeline design exists.

### Phase 10: Delete whole-module modes, dispatcher paths, bridge APIs and old bootstrap

Context: the refactor is not complete while old whole-module modes, dispatcher loops, bridge instructions and bootstrap paths remain.

- Delete `Flag::HtmlWasm` whole-module mode selection.
- Delete `moth_start` export and bootstrap path.
- Delete dispatcher-loop emission code and `WasmCfgLoweringStrategy::DispatcherLoop`.
- Delete any remaining `StringFromI64` legacy bridge instruction, preserving current 64-bit numeric formatters.
- Delete per-module memory section emission for user modules.
- Delete per-module Wasm emission from the final HTML entry path once entry bundling is in place.
- Delete `WasmFunctionEmissionPolicy::AllFunctions` and `ReachableFromExports` from the final module path.
- Delete whole-module validation in `compile_one_module`.
- Delete unconditional Int-as-I64 bridge assumptions, not supported I64/U64 or profile-selected Int64 lowering.
- Delete helper-export boolean structs.
- Delete duplicated helper export types between HTML-Wasm and core Wasm.
- Search for `DispatcherLoop`, `StringFromI64`, `moth_start`, `export_str_ptr` and old helper booleans. Review each hit. Legitimate i64 operations are required, not forbidden names.

### Phase 11: Complete backend-specific integration coverage and docs

Context: documentation, tests and progress matrix must reflect the final backend shape.

- Add Wasm validation and artefact assertions to canonical integration cases.
- Reuse the numeric parity corpus with mixed-target wrappers, compact fields/collections and Error.code U32 coverage.
- Update `docs/build-system-design.md` only if a durable mixed-target contract is confirmed missing.
- Update `docs/compiler-design-overview.md` only if HIR view or target validation ownership moves.
- Update Wasm capability matrix and backend coverage rows in the progress matrix.
- Update `index.md` as owners move.
- Rebuild generated documentation through the compiler.

## Old owners and paths to remove

- `BackendBuilder::build_backend(Vec<Module>, ...)` flat handoff
- `Flag::HtmlWasm` whole-module mode selection
- `moth_start` export and bootstrap path
- dispatcher-loop emission code
- the legacy `StringFromI64` bridge instruction, not legitimate 64-bit formatting
- per-module memory section emission for user modules
- `WasmLirBlock` flat block model with `Jump` and `Branch` terminators
- unconditional Int-as-I64 ABI assumptions
- helper-export boolean structs
- duplicated helper export types between HTML-Wasm and core Wasm
- `WasmCfgLoweringStrategy::DispatcherLoop`
- `WasmFunctionEmissionPolicy::AllFunctions` and `ReachableFromExports` from the final module path
- whole-module validation in `compile_one_module`

## Required tests

Cover:

- JavaScript-owned `start` lowering
- backwards propagation of JavaScript requirements
- no Wasm-to-JavaScript Moth call after propagation
- JavaScript-to-Wasm wrappers
- explicit partition reasons
- entry-specific partition
- physical variant keys and reuse, including incompatible NumericProfile rejection
- one Moth Wasm entry bundle across ordinary module dependencies
- page-local runtime and entry-bundle memory
- WIT components remain separate external bindings and `MothSource` packages are not reclassified
- generated JavaScript companions
- structured Wasm LIR emission
- imported runtime memory
- explicit selected-function, import, export, capability and layout plans
- fixed integer/float and Byte wrapper parity, including I64/U64 extremes, F16 rounding and every numeric profile
- compact scalar fields and collection strides consumed from validated layouts
- Error.code above I32::MAX through source, backend and wrapper paths
- preserved Number Wasm target rejection without a separate runtime delivery
- central output writing
- legacy dispatcher-loop, `moth_start`, per-module memory and unconditional numeric bridge paths are gone
- no old bootstrap path remains
- `check` performs the same planning and validation as `build`
- foreign builder manifest conflict is rejected before writing
- same builder with a different build profile cannot silently claim the same output root
- stale cleanup never deletes files owned by another builder or profile
- no force-overwrite path bypasses output ownership
- two independent builders cannot simulate an output pipeline by sharing one root

## Documentation and progress-matrix impact

- update `docs/build-system-design.md` only if a durable mixed-target contract is confirmed missing
- update `docs/compiler-design-overview.md` only if HIR view or target validation ownership moves
- update Wasm capability matrix and backend coverage rows in the progress matrix
- preserve the numeric authority's scalar/profile contract and distinguish delivered scalar support from new aggregate/runtime support
- update `index.md` as owners move

## Validation requirements

Each code-bearing phase runs:

```bash
cargo fmt
just validate
just bench-check
```

Run the documentation release build when source docs change.

## Final architecture audit

Before marking this plan complete, verify:

- the backend consumes `ProjectCompilation`, not `Vec<Module>`
- function-level partitioning replaces whole-module JS or Wasm mode
- `start` is JavaScript-owned with backwards propagation
- no Wasm-to-JavaScript Moth call exists after propagation
- Wasm LIR is structured, not flat basic-block with dispatcher loops
- each entry physical variant emits one Moth Wasm bundle rather than per-module Wasm modules
- the entry Wasm bundle imports page-local runtime memory rather than owning a separate memory
- WIT is not used as the semantic interface for Moth-source modules or packages
- numeric semantics/profile are shared across partitions and scalar storage remains compact
- U32 Error.code and valid I64/U64/Int64 paths survive removal of legacy bridges
- `moth_start`, the legacy `StringFromI64` bridge and helper-export booleans are gone
- no compatibility adapter remains
- `check` runs the same planning and validation as `build`
