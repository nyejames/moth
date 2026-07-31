# HTML mixed JavaScript and Wasm backend implementation plan

## Purpose

Implement the HTML project builder mixed JavaScript and Wasm backend strategy, final Wasm LIR and runtime design, and structured ABI. Replace the current experimental whole-module Wasm path with function-level partitioning, generated JavaScript companions and page-local shared Wasm runtime.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/html_project_backend_wasm_final_implementation_plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - current owner and test baseline
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
POST_TIR_REVIEW_COMMIT: 1298da468
BRANCH: main
IMPLEMENTATION_SCOPE: HTML project builder, JS backend, Wasm backend, output writing
```

## Hard prerequisites

- final TIR one-store/exact-view architecture and post-TIR roadmap review accepted at `1298da468`
- canonical module compilation with immutable artefacts and graph-aware backend handoff
- per-function link facts and target validation roots

## Required authority documents

- `docs/compiler-design-overview.md` for HIR, borrow facts, target validation and per-function link facts
- `docs/build-system-design.md` for HTML project builder, mixed-target planning, physical variants, runtime and memory, output ownership
- `docs/src/docs/codebase/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `docs/src/docs/progress/@page.moth` for current support
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md` for the graph contract
- `docs/roadmap/plans/number_type_numeric_plan.md` for numeric target validation

## Current implementation state

These are current implementation snapshots, not accepted final architecture.

Current build handoff:

- `BackendBuilder::build_backend` receives `modules: Vec<Module>`, not a graph
- `Module` carries `entry_point`, `hir`, `type_environment`, `borrow_analysis`, `warnings`, `const_top_level_fragments`, `entry_runtime_fragment_count`, `external_package_registry` and `module_external_imports`
- `FileKind` supports `Wasm(Vec<u8>)`, `Js(String)`, `Html(String)` and raw bytes

Current HTML builder:

- `HtmlProjectBuilder::build_backend` loops over modules and compiles one module at a time
- backend mode is selected through `Flag::HtmlWasm`
- `compile_one_module` validates the module for either JS or Wasm as a whole

Current Wasm mode:

- exports `start` as `moth_start`
- bootstrap instantiates `page.wasm`, calls `moth_start`, reads returned handles and hydrates slots
- helper export structs are duplicated between HTML-Wasm and core Wasm request types

Current Wasm LIR:

- `WasmLirFunction` contains a flat `Vec<WasmLirBlock>` with `Jump` and `Branch` terminators
- ABI includes `I64` for Moth `Int`
- instructions include bridge-like `StringFromI64`
- user functions are emitted through a dispatcher-loop strategy using an artificial program counter
- the emitter defines memory inside each emitted module

Current JS backend:

- has three lowering configs: direct JS, HTML page bundle, HTML-Wasm companion
- function emission policy is all-functions or reachable-from-start

## Required inputs

The plan must consume these from the canonical module plan. Each references where the full contract lives.

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

## Required final design

See `docs/build-system-design.md` "HTML project builder", "Mixed-target planning and validation", "Physical variants" and "Runtime and memory" for the full contracts.

Retain:
- JavaScript-owned `start` (the implicit `start` is lowered to JS, not Wasm)
- backwards propagation of JavaScript requirements (DOM, browser, project JS force JS ownership, requirements propagate to transitive callers)
- no Wasm-to-JavaScript Moth call after propagation
- JavaScript-to-Wasm wrappers (JS-owned functions may call Wasm-owned functions through generated wrappers)
- explicit partition reasons (every decision records why)
- entry-specific partition (partitioning is per-entry, not global)
- physical variant keys (deduplicate by module identity, function set, target assignment, ABI, layout, capability requirements and backend config fingerprint)
- page-local runtime and shared memory (each page owns one runtime instance and one shared memory)
- generated JavaScript companions (each selected module variant has a JS companion facade)
- structured derived HIR view (backend-neutral, derived, cached only as derived data)
- structured backend-owned Wasm LIR (not a second frontend semantic authority)
- imported runtime memory (Wasm variants import page runtime, not separate memories)
- explicit selected-function, import, export, capability and layout plans
- central output writing (builders produce output records, build system writes)
- no compatibility adapters

The final design removes (deleted rather than retained through compatibility adapters):
- dispatcher-loop control flow as the durable backend shape
- `moth_start`
- per-module memories
- helper-export booleans
- the `i64` Int bridge architecture
- flat basic-block Wasm LIR
- whole-module JS or Wasm mode
- global `HtmlWasm` mode as durable architecture
- current runtime helper duplication

## Non-goals

- no Wasm-to-JavaScript Moth call
- no per-module Wasm memory
- no dispatcher-loop LIR
- no `i64` Int bridge
- no `moth_start` export
- no whole-module JS or Wasm validation mode
- no compatibility adapters for old paths
- no standalone Wasm output pipeline design beyond the HTML builder orchestration

## Risks and blockers

- the canonical module plan must deliver the graph-aware payload before this plan can consume it
- Wasm LIR redesign is a large structural change that must preserve all existing semantic contracts
- partition correctness depends on accurate per-function link facts from the compiler
- ABI and layout design must coordinate with the runtime memory model

## Implementation phases

Each phase must leave one coherent path. Reference `docs/build-system-design.md` "HTML project builder" and "Mixed-target planning and validation" for full contracts.

### Phase 1: Current owner and test baseline

Context: refresh all code anchors and establish a test baseline before the structural refactor.

- Confirm the `1298da468` post-TIR review and canonical module compilation are accepted.
- Confirm the backend consumes only neutral owned runtime template payloads and ordinary HIR string operations; no TIR identity, view, overlay or preparation state reaches link planning or lowering.
- Record `git rev-parse HEAD`, branch and `git status --short`.
- Inventory current `BackendBuilder::build_backend`, `HtmlProjectBuilder`, JS backend, Wasm backend, LIR, emission and runtime owners.
- Run baseline `just validate` and `just bench-check`. Record results.

### Phase 2: Graph-aware HTML link planning

Context: the HTML builder must consume entry assemblies and linked module calls rather than looping over a flat module list.

See `docs/build-system-design.md` "Entry and package link planning" and "Per-function reachable unions".

- Replace `BackendBuilder::build_backend(Vec<Module>, ...)` with consumption of `ProjectCompilation`.
- Build HTML link plans from entry assemblies and per-function link facts.
- Compute exact reachable function and runtime-fact unions for each entry.
- Keep backend-facing access narrow: module artefacts, generated sidecars, entry assemblies, package facade and runtime metadata.
- Remove source-path graph reconstruction from the builder.

### Phase 3: Target affinity, partition and compiler validation

Context: replace whole-module JS or Wasm mode with function-level partitioning.

See `docs/build-system-design.md` "Mixed-target planning and validation" for partition rules.

- Compute target affinity from semantic package and capability metadata.
- Apply partition rules: `start` is JavaScript-owned, DOM and browser JS force JavaScript, JS requirements propagate backwards, no Wasm-to-JS Moth call after propagation, JS-to-Wasm wrappers, remaining functions default to Wasm.
- Every decision records an explicit reason.
- Partitioning is entry-specific and independent of development or release mode.
- Run compiler target validation against the completed deterministic partition.
- Validate every function against its assigned target.
- Validate permitted cross-target edges.
- `check` runs the same sequence and stops before lowering.

### Phase 4: Selected JavaScript emission and companions

Context: JS backend needs a partition-selected emission mode, not all-functions or reachable-from-start.

- Lower only the selected JavaScript function set.
- Emit required runtime helpers only for selected functions.
- Render compile-time fragments into the document.
- Emit runtime fragment slots.
- Invoke active `start` once through the selected runtime path.
- Hydrate runtime fragments in source order.
- Generate JavaScript companion facades for selected module variants.

### Phase 5: Page runtime and shared memory

Context: each page owns one runtime instance and one shared memory.

See `docs/build-system-design.md` "Runtime and memory".

- Generate one page-local Wasm runtime instance per entry.
- Linked Wasm variants import the page runtime rather than owning separate memories.
- Project-level runtime bytes may be emitted once and instantiated separately for each page.
- Remove per-module memory section emission for user modules.
- Import runtime memory from the generated runtime module.

### Phase 6: Structured HIR view and structured Wasm LIR

Context: replace flat basic-block LIR with structured, emission-shaped IR.

See `docs/build-system-design.md` "Runtime and memory" for the LIR contract.

- Add a backend-neutral structured HIR view derived for structured lowerers (cached as derived data only).
- Replace `WasmLirBlock` flat block model with structured body-tree LIR.
- Remove `Jump` and `Branch` terminators from final LIR.
- Make Wasm LIR structured and backend-owned. It is not a second frontend semantic authority.
- Generate runtime helper functions through structured Wasm LIR like user functions.

### Phase 7: ABI, layouts and runtime helpers

Context: define the Wasm ABI type mapping and replace bridge instructions.

- Remove `I64` as Moth `Int` ABI path. Use `i32`.
- Remove `StringFromI64`. Replace with `StringFromI32` or generic numeric format helpers.
- Remove `Void` as a real ABI type. Represent no result as `results: []`.
- Define Wasm ABI type mapping for scalars, handles, strings, collections, structs, choices, options and errors.
- Define Wasm layout for structs: field offsets, alignment, construction, field access, mutation and ownership hooks.
- Define Wasm layout for choices: unit variants, payload variants, tag representation, payload storage, equality, matching and generic choices.
- Design runtime string model: allocation, UTF-8 layout, interpolation helpers, host string extraction and release hooks. Keep backend ownership here; cross-link profiling and string-building experiments from `post-tir-template-parser-optimization-plan.md` without moving output assembly into TIR.
- Remove helper-export boolean structs. Replace with runtime capability and helper import plans.

### Phase 8: External packages and cross-target glue

Context: external JavaScript glue must be demand-driven per entry.

See `docs/build-system-design.md` "External JavaScript".

- Build-level runtime emission deduplicates runtime assets, required module specifiers and shared provider runtime files.
- Entry-level glue generation emits only wrappers for external functions referenced by the selected JavaScript bundle, required import preambles and import-map entries.
- Direct builder packages and provider-created packages use the same binding identity and runtime asset model.

### Phase 9: Physical variants, manifests and output ownership

Context: partitioning is entry-specific and physical variants are deduplicated by a conceptual key.

See `docs/build-system-design.md` "Physical variants" and "Output ownership".

- Deduplicate physical variants by a key containing module identity, selected concrete function set, target assignment, ABI identity, layout identity, runtime capability requirements and relevant backend config fingerprint.
- Entries with the same key reuse one variant.
- One source function may be JavaScript in one entry variant and Wasm in another.
- Each selected module variant has a generated JavaScript companion facade. Wasm is emitted per selected module variant.
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
- Delete `StringFromI64` bridge instruction.
- Delete per-module memory section emission for user modules.
- Delete `WasmFunctionEmissionPolicy::AllFunctions` and `ReachableFromExports` from the final module path.
- Delete whole-module validation in `compile_one_module`.
- Delete `I64` as Moth `Int` ABI path.
- Delete helper-export boolean structs.
- Delete duplicated helper export types between HTML-Wasm and core Wasm.
- Search for `DispatcherLoop`, `StringFromI64`, `I64`, `moth_start`, `export_str_ptr` and old helper booleans. Confirm only intentional history remains.

### Phase 11: Complete backend-specific integration coverage and docs

Context: documentation, tests and progress matrix must reflect the final backend shape.

- Add Wasm validation and artefact assertions to canonical integration cases.
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
- `StringFromI64` bridge instruction
- per-module memory section emission for user modules
- `WasmLirBlock` flat block model with `Jump` and `Branch` terminators
- `I64` as Moth `Int` ABI path
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
- physical variant keys and reuse
- page-local runtime and shared memory
- generated JavaScript companions
- structured Wasm LIR emission
- imported runtime memory
- explicit selected-function, import, export, capability and layout plans
- central output writing
- no dispatcher-loop, `moth_start`, per-module memory, `i64` bridge or `StringFromI64` remains
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
- Wasm modules import page-local runtime memory rather than owning separate memories
- `moth_start`, `StringFromI64`, `i64` Int bridge and helper-export booleans are gone
- no compatibility adapter remains
- `check` runs the same planning and validation as `build`
