# Rust Project Builder SDK Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the opinionated Rust project-builder SDK, compiler-independent host runtime and checked Rust binding profile documented under `docs/src/developer-docs/project-builders/`.

**Architecture:** Keep `moth` as the only compiler and build-system authority. Add `moth-builder` as the high-level SDK layered directly on its validated services, plus compiler-independent `moth-host` and `moth-host-macros` crates for reusable build/runtime host-package declarations. External builders receive frozen capability definitions, owner-bound compiled views and named lowering/output services rather than compiler stores or arbitrary semantic callbacks.

**Tech Stack:** Rust 2024, the existing Moth compiler/build system, `proc_macro`, narrowly configured `syn` 2 + `quote` 1 for Rust signature parsing/code generation, existing Wasm encoding/parsing infrastructure and the repository's normal test/validation tooling.

**Spec:** `docs/src/developer-docs/project-builders/` together with `docs/full-stack-project-builder-design.md`, `docs/compiler-design-overview.md`, `docs/build-system-design.md` and the canonical package/directive references they route to.

## Global Constraints

- Place this plan after the mixed JavaScript/Wasm backend in the serial roadmap. Activate only after implicit failures/entry errors, native result slots, general directives/project configuration, explicit root-purpose metadata and the final Wasm host-import ABI required by this work are delivered. Consume those final contracts rather than adding temporary bridges.
- `moth` remains the single compiler/language implementation. `moth-builder` may depend on `moth`; `moth` must not depend on `moth-builder`.
- Crate direction is `moth-host-macros -> proc-macro support`, `moth-host -> moth-host-macros + shared lexical/numeric policy`, `moth -> moth-host`, `moth-builder -> moth + moth-host`.
- A deployed runtime can use `moth-host` without linking `moth-builder` or the compiler.
- Keep one current API shape. Do not retain `BackendBuilder`, `BuilderKind` or other superseded builder seams through compatibility wrappers after their callers migrate.
- Builder registries declare constrained data/capabilities. They never accept arbitrary AST/HIR/backend mutation callbacks.
- Rust host bindings generate structured descriptors/adapters, never generated Moth source or implicit global registration.
- Host-package Rust names are Moth leaf names. V1 has no rename facility and Rust module paths never imply Moth namespaces.
- The automatic host profile is synchronous, concrete and closed. Async Rust host functions remain deferred until Moth's canonical async semantics define the boundary.
- Host target implementations may differ physically but share one Moth semantic signature/access/error contract.
- `BuildArtifacts` returns records. The build system remains the sole writer/manifest/stale-cleanup owner.
- Builder/runtime errors caused by user source or registration use structured diagnostics. Invalid user input must not panic.
- Preserve first-party dependency rules and run the Slice review after every non-trivial task.

## Review Focus

- All four `NumericProfile` combinations: `Int`/`Float` parameters use only the documented lossless host-carrier compatibility and fixed-width types never gain implicit cross-width coercion.
- Host handles: wrong type, stale generation, reused slot and cross-runtime handles are rejected while `is`/`is not` compare handle identity without dereferencing the payload.
- Rust macro diagnostics: common unsupported forms (`usize`, `String` parameter, `&str` result, collections, generic/async/unsafe functions, nested options/results/tuples) point to a supported replacement when one exists.
- Runtime contract mismatch: changed name/signature/access/error/ABI/profile fails before application execution even when low-level carriers happen to match.
- CLI convergence: SDK and Moth binary share core command parsing/meaning; builder extensions cannot shadow top-level commands and remain under `builder`.

---

## Prerequisite gate

Before Task 1:

- [ ] Re-read `AGENTS.md`, the style guide, testing/validation guides and all project-builder docs.
- [ ] Confirm the prerequisite roadmap work named in Global Constraints is on the activation branch. If a prerequisite is missing, stop rather than creating a temporary SDK representation around it.
- [ ] Record the activation branch/head in working notes. Do not pin that SHA into this plan.
- [ ] Inventory the current owners of `BackendBuilder`, `ProjectBuilder`, `BuilderSurface`, `BuilderKind`, external binding metadata, Wasm host imports, CLI build/check/dev/new dispatch and output manifests.
- [ ] Search for every direct `BuilderSurface` field mutation and every external-package registration path. Decide which existing owner each new facade delegates to before adding a parallel helper.
- [ ] Run the branch-appropriate baseline checks from `validation.mtf` and record failures before changing code.

---

### Task 1: Add the three crate boundaries and dependency guards

**Files:**
- Modify: `Cargo.toml`, `justfile`
- Modify: validation/dependency-audit crate inventories under `xtask/src/`
- Create: `crates/moth-host/Cargo.toml`, `crates/moth-host/src/lib.rs`
- Create: `crates/moth-host-macros/Cargo.toml`, `crates/moth-host-macros/src/lib.rs`
- Create: `crates/moth-builder/Cargo.toml`, `crates/moth-builder/src/lib.rs`

**Interfaces:**
- Produces workspace crates `moth-host`, `moth-host-macros` and `moth-builder`.
- `moth-host` re-exports the proc macros. `moth-builder` depends on `moth` and `moth-host`; `moth` may depend on `moth-host` but never `moth-builder`.

- [ ] **Step 1: Extend dependency-audit tests first.** Add expectations for the accepted direction and a rejection check that would fail on `moth -> moth-builder` or `moth-host -> moth`.
- [ ] **Step 2: Run the focused dependency/audit test and confirm it fails because the new members do not exist.**
- [ ] **Step 3: Add the three minimal crate manifests and roots.** Add `syn = "2"` and `quote = "1"` only to the proc-macro crate with the narrowest needed features.
- [ ] **Step 4: Add each crate to formatting, Clippy, test and feature-lane inventories explicitly.**
- [ ] **Step 5: Run `cargo check -p moth-host -p moth-host-macros -p moth-builder`, the dependency audit and lane-coverage check.**
- [ ] **Step 6: Slice review, then commit the crate/dependency boundary.**

---

### Task 2: Implement the closed host-signature descriptors and proc-macro diagnostics

**Files:**
- Create: `crates/moth-host/src/{value_type,error,byte,binding}.rs`
- Create: `crates/moth-host-macros/src/{host_function,host_type,diagnostics}.rs`
- Create: `crates/moth-host-macros/tests/ui.rs` and focused compile-pass/fail fixtures

**Interfaces:**
- `moth_host::Byte`
- `moth_host::Error`
- `HostValueType`, `HostParameterDescriptor`, `HostReturnShape`
- `#[moth_host::host_function]`, `#[moth_host::host_type]`
- `moth_host::host_binding!(path)`

- [ ] **Step 1: Write compile-pass tests for every accepted mapping in `host-packages.mtf`, including zero/multiple returns, `Result<(...)>`, one optional layer, `Byte`, `&str` input and `String` output.**
- [ ] **Step 2: Write compile-fail cases for every common unsupported form named by the spec. Assert the diagnostic names the incompatible Rust form and a concrete replacement where one exists.**
- [ ] **Step 3: Run the macro UI suite and verify the new cases fail before implementation.**
- [ ] **Step 4: Implement the descriptor vocabulary without importing compiler `DataType`, `TypeId` or backend ABI enums into `moth-host`.**
- [ ] **Step 5: Implement `#[moth_host::host_function]`. Preserve the original free function, reject methods/generics/async/unsafe/foreign ABI and consume only `#[moth(host)]` plus `#[moth(exclusive)]`. Permit at most one host parameter, first, typed `&Env` or `&mut Env`.**
- [ ] **Step 6: Implement `#[moth_host::host_type]` for fieldless marker structs only and generate typed-handle identity equality.**
- [ ] **Step 7: Implement explicit `host_binding!` lookup with no inventory/global registration and no rename syntax.**
- [ ] **Step 8: Run macro unit/UI/doc tests.**
- [ ] **Step 9: Slice review, then commit.**

---

### Task 3: Implement host packages, environments and canonical opaque-handle storage

**Files:**
- Create: `crates/moth-host/src/{package,namespace,environment,resources,registry,contract}.rs`
- Create: `crates/moth-host/tests/{package_contract,resources}.rs`

**Interfaces:**
- `HostPackage<Env>::new`, `HostPackage::stateless`
- `HostPackage::{namespace,function,host_type,constant,semantic}`
- `HostEnvironment::{resources,resources_mut}`
- `HostResources::{insert,get,get_mut,remove}`
- `HostBindingIdentity`, `HostContractFingerprint`
- `HostRegistry<Env>`

- [ ] **Step 1: Test explicit namespace composition, exact Rust/Moth leaf-name identity, duplicate/case-conflicting names and absence of rename support.**
- [ ] **Step 2: Test wrong marker, wrong payload, stale generation after slot reuse, cross-runtime injection and identity equality before/after staleness.**
- [ ] **Step 3: Test that build-time semantic and runtime projections come from the same package declaration and fingerprint identically.**
- [ ] **Step 4: Implement deterministic package/namespace registration.**
- [ ] **Step 5: Implement per-host-type typed resource tables with reusable slots plus generation/runtime identity. Do not use Rust object addresses as identity.**
- [ ] **Step 6: Enforce `HostPackage<Env>` environment typing while allowing stateless bindings, and erase `Env` only in the semantic projection.**
- [ ] **Step 7: Implement explicit scalar constants. Exclude strings, handles, collections and runtime-derived values.**
- [ ] **Step 8: Run `cargo test -p moth-host` and macro tests.**
- [ ] **Step 9: Slice review, then commit.**

---

### Task 4: Project host descriptors into the canonical Moth external-package model

**Files:**
- Modify: `src/compiler_frontend/external_packages/{abi,definitions,registry}.rs`
- Modify: `src/compiler_frontend/ast/expressions/call_validation.rs`
- Modify: `src/compiler_frontend/datatypes/environment.rs`
- Modify: focused external-package/backend validation tests
- Modify: root `Cargo.toml` for `moth -> moth-host`

**Interfaces:**
- One compiler-owned conversion service registers `HostPackageSemantic` into the existing Builder/ExternalBinding registry.
- `ExternalParameter` gains only the native `Int`/`Float` receiving-compatibility fact required by the spec.
- Opaque external host types advertise identity equality only.

- [ ] **Step 1: Add compiler tests for fixed-width mappings, Byte, Option, zero/multiple result slots and built-in Error channels.**
- [ ] **Step 2: Add the four-profile numeric matrix and explicit negative cases proving no fixed-width cross-width coercion.**
- [ ] **Step 3: Add opaque equality tests plus ordering/arithmetic/map-key rejection.**
- [ ] **Step 4: Implement the one host-descriptor conversion service over the existing external package registry.**
- [ ] **Step 5: Extend external ABI/signature vocabulary only for missing fixed scalars/Byte required by this profile. Keep Core native `Int`/`Float` distinct.**
- [ ] **Step 6: Implement native numeric receiving compatibility in ordinary call validation after the `NumericProfile` is fixed. Do not add a general numeric coercion.**
- [ ] **Step 7: Wire host-handle equality into the existing runtime-equality capability and lower it by handle identity. Leave hashing absent.**
- [ ] **Step 8: Run focused compiler and integration tests.**
- [ ] **Step 9: Slice review, then commit.**

---

### Task 5: Add closed physical implementations and artefact/runtime contract validation

**Files:**
- Modify/create host implementation descriptor modules in `crates/moth-host/src/`
- Modify: `src/compiler_frontend/external_packages/definitions.rs`
- Modify: `src/backends/external_package_validation.rs`
- Modify: existing Wasm host-import/emission owners under `src/backends/wasm/`
- Modify: existing HTML JavaScript runtime/provider owners only where they project into the shared identity
- Create: `crates/moth-host/src/wasm_contract.rs`
- Create: `crates/moth-host/tests/wasm_contract.rs`

**Interfaces:**
- Closed implementation categories, initially Rust/Wasm host import and registered JavaScript module export.
- Wasm custom section `moth.host.v1`.
- `HostRegistry::validate_wasm(bytes)`.

- [ ] **Step 1: Test one semantic binding with target-specific implementations and pre-lowering rejection when implementation/capability is missing.**
- [ ] **Step 2: Add Wasm artefact assertions and `moth-host` round-trip tests for `moth.host.v1`.**
- [ ] **Step 3: Test missing/renamed/changed bindings, access/error changes, host ABI mismatch and numeric-profile mismatch before execution.**
- [ ] **Step 4: Implement the closed implementation descriptors. Adapt the existing JS package/runtime path rather than creating a second one.**
- [ ] **Step 5: Emit only reachable host requirements into the versioned Wasm custom section using stable identities/fingerprints.**
- [ ] **Step 6: Implement bounded compiler-independent section parsing/validation in `moth-host` with `wasmparser`; malformed/duplicate sections are host artefact errors, never panics. Do not add a second Wasm byte parser.**
- [ ] **Step 7: Implement Rust dispatch adapters. Contain unwinding panics at the outer dispatch boundary and report host faults rather than `Error!`.**
- [ ] **Step 8: Keep Wasm-engine selection outside `moth-host`; expose validated import adapters that an embedding runtime can install.**
- [ ] **Step 9: Run focused host/Wasm/JS tests, Slice review and commit.**

---

### Task 6: Replace closed builder identity and publish the frozen definition bridge

**Files:**
- Modify: `src/build_system/output/{policy,manifest}.rs`
- Modify: `src/build_system/build.rs`
- Modify: `src/builder_surface/definition.rs`
- Create: `src/build_system/builder_api/{mod,identity,registration}.rs`
- Create: `crates/moth-builder/src/{definition,diagnostics,source_package}.rs`
- Create: `crates/moth-builder/tests/definition.rs`

**Interfaces:**
- Stable `BuilderId`
- `BuilderApiVersion::V1`
- `BuilderDefinition::new(id, api_version)` and `finish()`
- Low-level Moth registration service that instantiates one per-build internal surface/session from the frozen definition

- [ ] **Step 1: Write output-manifest tests replacing `BuilderKind` assumptions with stable builder identity and cross-builder ownership rejection.**
- [ ] **Step 2: Write definition tests for invalid IDs, duplicate packages/directives/root purposes/config surfaces and compiler-owned name overrides.**
- [ ] **Step 3: Replace `BuilderKind` with `BuilderId` through output ownership and HTML builder manifests, then delete the enum/test-only variants.**
- [ ] **Step 4: Split static capability declaration from mutable provider/cache/resolution state currently bundled in `BuilderSurface`.**
- [ ] **Step 5: Implement the high-level definition wrapper over compiler-owned registration types. Do not duplicate compiler schema/type rules.**
- [ ] **Step 6: Fold template directives into the one definition surface rather than retaining a separate style-directive hook.**
- [ ] **Step 7: Replace the misleading `BackendBuilder` seam with the smallest compiler-owned builder client/service boundary required by the SDK. Migrate HTML/test callers in the same task and delete the old trait/name.**
- [ ] **Step 8: Run output/bootstrap/HTML/definition tests, Slice review and commit.**

---

### Task 7: Expose config, root-purpose, source-kind, package and provider helpers

**Files:**
- Create: `crates/moth-builder/src/{config,directives,root_purpose,providers}.rs`
- Modify: the delivered general-directive/root-purpose registration owners
- Modify: `src/builder_surface/{source_file_kind_registry,source_package_registry}.rs`
- Modify: `src/builder_surface/external_import_providers/**` only through a public facade
- Add focused definition/compiler integration tests

**Interfaces:**
- Exactly one `BuilderDefinition::config(name, configure)` surface.
- `RootPurpose` descriptors over compiler-owned directive schema types.
- `.moth_templates()` / `.markdown()` opt into compiler-known adapters.
- Public provider context exposes logical paths, package/runtime-asset services and structured diagnostics, not interners.

- [ ] **Step 1: Test the one-config-surface rule, defaults and compiler-owned directive collisions.**
- [ ] **Step 2: Test root-purpose parsing/placement/folding, retained metadata, assembly selection and unpurposed runtime-work rejection without builder scans.**
- [ ] **Step 3: Implement config/schema helpers over the delivered canonical config/directive owner.**
- [ ] **Step 4: Implement root-purpose descriptors and typed retained metadata.**
- [ ] **Step 5: Implement source-package/source-kind helpers with automatic Builder provenance and no custom-parser hook.**
- [ ] **Step 6: Wrap provider context so external providers use named services instead of mutable compiler registries/interners.**
- [ ] **Step 7: Wire `HostPackage<Env>` semantic projections into `BuilderDefinition::host_package` without making definitions generic over `Env`.**
- [ ] **Step 8: Run focused and end-to-end tests, Slice review and commit.**

---

### Task 8: Publish compiled-project views, lowering services and output records

**Files:**
- Create: `src/build_system/builder_api/{compiled_project,build_context,artifacts}.rs`
- Create: `crates/moth-builder/src/{project,build_context,artifacts}.rs`
- Modify: `src/build_system/build.rs`
- Modify: named backend entry points under `src/backends/js/` and `src/backends/wasm/`
- Modify: resource/output owners only where needed
- Add tests under `src/build_system/tests/` and `crates/moth-builder/tests/`

**Interfaces:**
- `CompiledProject::{entries,roots,package,require_package}`
- `EntryTemplatePlan`
- `BuildContext::{compile_wasm,compile_javascript}`
- `BuildArtifacts::{new,add}`

- [ ] **Step 1: Test safe assembly selection and Rust visibility boundaries. Do not expose/mutate compiler stores through the public facade.**
- [ ] **Step 2: Test const/runtime fragment plans and success-only start publication.**
- [ ] **Step 3: Implement owner-bound projections over existing project/entry/package/compiler metadata without cloning HIR/type stores.**
- [ ] **Step 4: Remove HTML-specific entry-selection assumptions from generic `ProjectCompilation` now that purpose facts own selection.**
- [ ] **Step 5: Implement named lowering services that own target validation, lifecycle instantiation, memory planning and backend invocation.**
- [ ] **Step 6: Expose structural-string/resource services without rendered-string rescans.**
- [ ] **Step 7: Replace generic output fields that encode HTML-only concepts with builder-neutral artefact/launch metadata while preserving HTML behaviour.**
- [ ] **Step 8: Run build/backend/HTML/SDK tests, Slice review and commit.**

---

### Task 9: Add the canonical CLI driver and low-level embedding API

**Files:**
- Refactor: `src/projects/cli.rs` into shared parser/driver owners
- Create as needed: `src/projects/driver/**`
- Modify: `src/projects/check.rs`, `src/projects/dev_server/**`
- Create: `crates/moth-builder/src/{driver,commands,request}.rs`
- Add parser/driver tests under both owning modules

**Interfaces:**
- `moth_builder::run(builder) -> Result<()>`
- Low-level build/check/dev request APIs using the same typed input/profile/config path
- Core commands retain one parser/meaning; builder commands exist only under `builder`

- [ ] **Step 1: Write parser parity tests for core commands and identical typed errors.**
- [ ] **Step 2: Test that extensions cannot register/override top-level core commands.**
- [ ] **Step 3: Extract command parsing/materialised input ownership from HTML-specific dispatch while preserving one `--input` path.**
- [ ] **Step 4: Implement `moth_builder::run` as a thin adapter over the canonical Moth project-tool driver, not a copied CLI.**
- [ ] **Step 5: Implement low-level request entry points for process-owning embedders through the same compiler/build services.**
- [ ] **Step 6: Generalise `dev` at the existing executor/watch seam: Moth owns config/build/watch/rebuild, the builder supplies application-specific runtime/session behaviour.**
- [ ] **Step 7: Generalise `new` through a builder scaffold hook under the canonical command shape.**
- [ ] **Step 8: Add scoped `builder` command dispatch.**
- [ ] **Step 9: Run CLI/build/check/dev/scaffold/SDK tests, Slice review and commit.**

---

### Task 10: Prove an external builder and compiler-free runtime consumer end to end

**Files:**
- Create focused public-API fixtures under `crates/moth-builder/tests/`
- Create runtime-only tests under `crates/moth-host/tests/`
- Add Moth integration cases under `tests/cases/` only where normal source-visible behaviour is the owner

- [ ] **Step 1: Build a realistic custom builder fixture with one source package, host package, config surface and root purpose using only public SDK APIs.**
- [ ] **Step 2: Exercise shared/exclusive opaque handles, identity equality, one optional, multiple returns, built-in Error and native numeric parameter compatibility through the real compiler path.**
- [ ] **Step 3: Prove a runtime-only consumer validates/dispatches a prebuilt Wasm host contract without `moth` or `moth-builder` in its dependency graph.**
- [ ] **Step 4: Cover stale/cross-runtime handles, contract mismatch and Rust panic in their documented error/fault lanes.**
- [ ] **Step 5: Run the custom builder under all four numeric profiles and applicable JS/Wasm target lanes.**
- [ ] **Step 6: Run existing HTML builder coverage, Slice review and commit.**

---

### Task 11: Final authority/docs/progress migration and merge validation

**Files:**
- Update: `docs/build-system-design.md`
- Update: `docs/compiler-design-overview.md`
- Update: `docs/src/docs/packages/external-binding-contracts.mtf`
- Update: `docs/src/developer-docs/project-builders/*.mtf`
- Update: both progress matrices only where support changed
- Update: `index.md`, validation inventories and stale audit-log rows
- Delete this plan and its roadmap bullet in the completion commit

- [ ] **Step 1: Re-read permanent authorities and move every durable implementation-time clarification to its correct owner. Remove stale references to `BackendBuilder`, `BuilderKind`, `host context` terminology or duplicate signature metadata.**
- [ ] **Step 2: Update external binding docs with the final Rust host profile while keeping JS/WIT contracts distinct.**
- [ ] **Step 3: Update tutorial examples to the exact shipped APIs with no implementation-status prose.**
- [ ] **Step 4: Update progress rows to actual support. Keep async Rust host functions as a distinct Deferred surface until Moth async owns the boundary.**
- [ ] **Step 5: Update `index.md`, crate/lane inventories and stale audit-log rows. Regenerate docs through the compiler, never by editing `docs/release/**`.**
- [ ] **Step 6: Run focused package/SDK/integration checks and `just validate` at the final integration checkpoint.**
- [ ] **Step 7: Perform the full Slice review across new crates, compiler adapter, build facade, CLI driver and HTML paths. Search again for duplicate/legacy builder surfaces and compiler-internal leaks.**
- [ ] **Step 8: Run `just validate-full` on the final integrated tree and record exact evidence.**
- [ ] **Step 9: In the same completion commit, delete this plan and its roadmap bullet. Preserve durable contracts in permanent docs and Git history.**
