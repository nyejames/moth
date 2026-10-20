# Lappet agent rules

This file extends the repository-root `AGENTS.md` for work under
`crates/lappet/`. Read both files in the current isolated worktree. The root
style, testing, validation and Slice review rules still apply. Resolve all
paths below from the worktree root.

## Reading routes

Classify the task before reading more documentation. Read the applicable
sections of `crates/lappet/docs/design.md` and the current state and relevant
checkpoint in `crates/lappet/roadmap.md` before implementation. An open design
checkpoint blocks implementation of that contract. Read full authorities for
architectural reviews or changes that cross ownership boundaries.

- **Wasmtime, execution and embedding:** read
  `crates/lappet/docs/wasmtime-integration.md`, then the runtime strategy,
  instances and failure sections of the Lappet design. Verify APIs against the
  exact selected Wasmtime release, not upstream main.
- **Host bindings, ABI and exported calls:** read the Lappet Rust bindings
  section, `docs/src/developer-docs/project-builders/host-packages.mtf` and
  `docs/src/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf`.
  Once approved, `docs/src/developer-docs/memory-management/native-host-abi/native-host-abi.mtf`
  owns the physical boundary and `crates/lappet/docs/host-abi.md`
  owns Lappet's adapter, staging, resource-table and fault handling. Neither
  overrides compiler memory semantics.
- **Native Core implementations:** read the relevant canonical Core package
  reference under `docs/src/docs/packages/core/` and Lappet's official Core
  contract. Check browser/native parity and documented host capability limits.
- **Package manager:** read `crates/lappet/docs/package-manager.md` and the
  relevant dependency declaration, catalog and graph sections of
  `docs/build-system-design.md`. The manager must consume compiler-owned
  compatibility facts, not build a second semantic resolver.
- **CLI split or compiler/build integration:** use the task-reading routes in
  the root `AGENTS.md` for both `docs/compiler-design-overview.md` and
  `docs/build-system-design.md`. Inspect the actual producer and consumer APIs.
- **Wasm optimisation or benchmarks:** read
  `docs/src/developer-docs/wasm-optimisation/wasm-optimisation.mtf`.
- **Moth source and language semantics:** use
  `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` for orientation and
  `docs/src/docs/progress/@page.moth` for implemented support. Read the relevant
  canonical language references when correctness depends on exact semantics.
  The cheatsheet is not mandatory for Rust-only runtime or manager tasks.
- **Tests and validation:** read the relevant sections of
  `docs/src/developer-docs/style-guide/testing.mtf` and
  `docs/src/developer-docs/style-guide/validation.mtf`.

If a routed reference is absent or stale, report the drift and consult the
current owner or version-pinned upstream source. Never invent a missing
contract. Re-route when the task expands.

## Authority and architecture

The explicit task and the owning canonical documents outrank this file.
Compiler and build authorities own Moth semantics, type identities, package
graphs, validated memory plans and output planning. Lappet owns native Wasm
execution, Rust host adapters and package acquisition/admission. The builder
owns capabilities, application lifecycle, scheduling and trusted host libraries.
The roadmap tracks delivery and gates, not alternative semantics.

- V1 executes core Wasm through Wasmtime Cranelift only. Development defaults
  to `OptLevel::None` and release to `OptLevel::Speed`, independently of
  Moth's compilation profile. Winch and other engines are deferred.
- Execution is synchronous and builder-scheduled. No implicit guest callbacks,
  general re-entry, async runtime or persistent AOT cache.
- Rust is the only V1 native host-binding language. Trusted C libraries are
  accessed through Rust. Foreign WIT components, arbitrary WAT packages and
  JavaScript execution are outside Lappet V1.
- Consume compiler-validated binding and layout facts. Do not infer language
  semantics from raw Wasm values, reconstruct compiler analyses or assume Rust
  layouts match Moth's ABI.
- Keep ordinary Moth storage independent of retained host state. Validate
  guest memory bounds, value conversions, resource type/owner/lifetime and
  overlapping mutable access before constructing Rust references.
- Distinguish declared Moth errors, Wasm traps, binding faults and host
  operational failures. Never silently grant filesystem, network or WASI access.
- Core interfaces are compiler-owned. Lappet provides reusable native
  implementations, not new Core semantics. Using Lappet does not certify a
  custom builder for Core-dependent managed packages.
- Package management must remain usable without Wasmtime. Managed V1 packages
  are Moth-source only; preserve no-core/Core-dependent admission and
  fail-closed assessment without automatically installing Rust/C host code.

## Working and testing rules

Keep implementation in an isolated Lappet worktree. Follow the accepted
checkpoint and implement the smallest complete slice. Inspect nearby owners,
tests and existing utilities before adding a wrapper, registry, type or helper.
Avoid unsafe memory access and speculative runtime abstractions. Keep the
game-engine and native-server embedding cases in view.

Start runtime tests with small core Wasm fixtures where useful. Exercise
Cranelift's development and release optimisation levels, malformed imports,
bounds, traps, cleanup, stale handles and lifecycle isolation. Fixtures establish runtime mechanics, not Moth compiler
ABI correctness. Add real Moth integration coverage only once the required
compiler lowerings are delivered. Test package graph and admission decisions
without pulling in Wasmtime.

Use focused tests during a slice and the root validation policy at integration
checkpoints. Measure compilation, instantiation, execution, host-call cost and
memory separately. Review each completed slice for duplicated owners, unused
abstractions, failure handling and missing tests. Use available planning,
debugging and code-review skills when they help without overriding design
gates. Update Lappet's roadmap with delivered status and evidence. Do not edit
other permanent documentation unless requested or an accepted plan requires it.

After compaction, re-read the root and Lappet agent files and reload the
relevant routed authorities.
