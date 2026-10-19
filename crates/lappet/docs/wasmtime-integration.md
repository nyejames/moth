# Wasmtime integration reference

This is Lappet's source map for **core Wasm execution**, not a Wasmtime
manual. [Lappet's design](design.md) owns the accepted V1 boundaries and
[its roadmap](../roadmap.md) owns implementation gates. This page records
relevant upstream mechanics and limitations. It does not approve a concrete
Moth ABI or claim that the runtime is implemented.

**Research snapshot:** Wasmtime Rust crate **49.0.3**, released 2026-10-14.
Read the [v49.0.3 source](https://github.com/bytecodealliance/wasmtime/tree/v49.0.3)
and versioned rustdoc, not an unversioned development API page. This is
a **reference version**, not a selected Cargo dependency. Pin the version at
the roadmap's engine checkpoint and recheck source details when upgrading.

## The small API surface Lappet actually needs

| Wasmtime | Purpose |
|---|---|
| `Config`, `Strategy`, `Engine` | Choose compiler strategy, feature set and engine-level options |
| `Module` | Validate and compile binary core Wasm for one engine |
| `Linker<T>`, `InstancePre<T>` | Register Rust host functions, check imports and prepare repeated instantiation |
| `Store<T>`, `Instance` | Own guest state, memories and a per-instance Rust context |
| `Caller<'_, T>` | Access host state and caller exports inside imported functions |
| `Func`, `TypedFunc` | Install/call functions using physical Wasm signatures |
| `Memory` | Validate, read and write guest memory |
| `StoreLimits`, `ResourceLimiter` | Limit selected guest allocations |
| `Error`, `Trap`, `WasmBacktrace` | Attribute Wasm execution and host-call failures |

The normal flow is **engine -> module -> linker import checks -> store-owned
instance -> typed call**. Moth owns the semantic signature and physical ABI
definition before this sequence. Lappet does not rediscover them from Wasm.

## Engine configuration and feature selection

Primary references: [Config](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Config.html),
[Strategy](https://docs.rs/wasmtime/49.0.3/wasmtime/enum.Strategy.html),
[crate features](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/Cargo.toml#L125-L175)
and [config source](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/config.rs).

- Build a `Config`, select `Strategy::Cranelift` and pass it to
  `Engine::new`. Use `Config::cranelift_opt_level(OptLevel::None)` for
  development and `OptLevel::Speed` for release, pending benchmarks.
  Inconsistent options can fail during engine creation. `Strategy::Auto`
  currently selects Cranelift but is not an explicit V1 strategy policy.
- Candidate minimal Cargo features are `std`, `runtime` and `cranelift`,
  with `default-features = false`. **This is not a locked manifest.**
  Defaults include unrelated features. Measure `cargo tree -e features`,
  clean Rust build cost and linked size before adding a feature.
- In v49.0.3 `OptLevel::None` disables most optimisation to favour
  compilation time. `OptLevel::Speed` is the optimising default.
  Keep `RegallocAlgorithm::Backtracking`. The upstream
  `SinglePass` allocator is not production-ready and lacks normal
  security-advisory coverage. `compiler_inlining` defaults to
  `Inlining::No`. Consider `SpeedAndSize` and parallel compilation
  only after independent measurements. Read the
  [configuration source](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/config.rs#L1538-L1587)
  and [single-pass warning](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/config.rs#L3699-L3725).
- Lappet does not run a Wasm post-optimiser or duplicate Moth's semantic
  passes. The [shared optimisation authority](../../../docs/src/developer-docs/wasm-optimisation/wasm-optimisation.mtf)
  owns that boundary.
- Validate the compiler's Wasm proposal requirements against Cranelift
  and the actual host CPU. Multi-value is enabled by default and Memory64
  is disabled by default in v49.0.3. Do not enable GC, threads or relaxed
  SIMD merely because Wasmtime can implement them. No fallback compiler.

- Use the live
  [compiler feature matrix](https://docs.wasmtime.dev/stability-tiers.html#compiler-support)
  and [platform tiers](https://docs.wasmtime.dev/stability-tiers.html#current-tier-status)
  for investigation, then verify the corresponding release. Lappet's
  supported host OS/CPU combinations require its own real CI evidence.

## Module, linker and store ownership

Primary references: [Module](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Module.html),
[Store](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Store.html),
[Linker](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Linker.html),
[store identity in source](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/runtime/store.rs#L160-L200)
and [linker sharing rules](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/runtime/linker.rs#L51-L82).

- An `Engine` and its compiled `Module` can be shared where useful.
  Each `Store<T>` owns its guests, memories and mutable execution state.
  Dropping the store frees its guest objects. Wasmtime does not collect
  otherwise unused instances within a still-live store, so select explicit
  lifecycles for a game world or isolated server execution.
- Wasm objects have a **store identity**. Passing an instance, memory or
  function with the wrong store can **panic** rather than return a typed
  Lappet error. Linkers and modules are also associated with an engine.
  Use ownership-safe wrapper APIs instead of exposing arbitrary handles.
- `Linker<T>` does **not** own `T`. It specifies the host state type
  stored in `Store<T>`. Host functions installed with `func_wrap` and
  `func_new` are store-independent and reusable across stores. Defining
  store-owned globals, memories or functions in the linker prevents that
  general reuse.
- Host closures registered with `func_wrap` are `Fn + Send + Sync +
  'static`. Put changing application state in `Store<T>` and access it
  through `Caller`. Different stores can execute shared host functions
  concurrently. Do not make instance state accidentally global.
- `Linker::instantiate_pre` resolves and checks imports once, producing
  reusable `InstancePre<T>`. Missing or mismatched imports fail at
  preflight. Actual **instantiation runs the Wasm start function** and can
  trap or invoke host imports before an explicit Lappet entry call. Guest
  initialization has observable effects.
- Keep duplicate-import shadowing disabled. Avoid
  `define_unknown_imports_as_traps` and
  `define_unknown_imports_as_default_values`: a missing host capability
  or invalid ABI must be rejected, not silently installed as a stub.

### API orientation only

This sketch uses **ordinary core Wasm**, not the future Moth host ABI.
`wasm_bytes` must import `host.read_value` as `() -> i32` and export
`run` with that physical signature.

```rust
use wasmtime::{Caller, Config, Engine, Linker, Module, Store, Strategy};

struct HostState {
    value: i32,
}

fn invoke(wasm_bytes: &[u8]) -> wasmtime::Result<i32> {
    let mut config = Config::new();
    config.strategy(Strategy::Cranelift);
    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, wasm_bytes)?;

    let mut linker = Linker::new(&engine);
    linker.func_wrap("host", "read_value", |caller: Caller<'_, HostState>| {
        caller.data().value
    })?;
    let prepared = linker.instantiate_pre(&module)?;

    let mut store = Store::new(&engine, HostState { value: 42 });
    let instance = prepared.instantiate(&mut store)?;
    let run = instance.get_typed_func::<(), i32>(&mut store, "run")?;
    run.call(&mut store, ())
}
```

Do not install imports by guessed source names or treat `i32` as a
complete Moth numeric signature. Real Lappet adapters consume checked
compiler descriptors. A complete, executable embedding example remains
a separate roadmap deliverable.

## Host calls and linear memory

Primary references: [Caller](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Caller.html),
[Func](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Func.html),
[TypedFunc](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.TypedFunc.html),
[Instance](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Instance.html),
[Memory](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.Memory.html)
and [memory safety source](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/runtime/memory.rs#L33-L178).

- `get_typed_func` checks the **physical Wasm call signature**, not the
  Moth type, result-channel or ownership contracts. The dynamic `Func::call`
  API can use `Val` arrays when a signature cannot be statically expressed,
  but that does not justify a second runtime semantic type registry.
- `Caller<'_, T>` provides store state and caller exports inside a host
  import. `get_export("memory")` can return `None` when that export
  does not exist or the call has no Wasm caller. Memory/allocator access
  must be part of the approved compiler-produced ABI, not a convention
  inferred by a Rust adapter.
- `Memory::read` and `Memory::write` are bounds-checked copying APIs.
  Failed out-of-bounds writes do not partially modify memory. Validate
  pointer sign and conversion, pointer+length overflow, UTF-8 and
  Moth-specific value constraints separately. Wasmtime's bounds check
  does not prove that a byte sequence is a valid Moth value.
- `Memory::data` and `data_mut` borrow the associated store while their
  slice lives. Such slices cannot safely survive a guest call or memory
  growth. `Memory::data_and_store_mut` permits disjoint simultaneous
  access to store state and a memory slice when justified. Read the linked
  upstream aliasing discussion before proposing zero-copy adapters.
- Avoid raw guest pointers and long-lived borrowed views. Calls into the
  guest can mutate memory and growth can invalidate pointers. Ordinary
  host arguments cannot retain Moth storage after the call. Returned
  ordinary values must use the approved allocation, result and cleanup ABI.
- Host functions may return a Rust `Result` whose error stops guest
  execution. A Wasmtime host-call failure is **not** automatically a
  Moth `Error!` value. Preserve declared Moth errors, guest traps,
  host operational failures and malformed binding faults separately.
  Neither retry nor rollback of earlier host side effects is automatic.
- General guest callbacks, arbitrary re-entry and async host functions
  remain outside V1. A future compiler-approved allocator interaction
  must explicitly specify ordering and mutable store borrowing.

## Resource controls and failure handling

Primary references: [ResourceLimiter](https://docs.rs/wasmtime/49.0.3/wasmtime/trait.ResourceLimiter.html),
[StoreLimitsBuilder](https://docs.rs/wasmtime/49.0.3/wasmtime/struct.StoreLimitsBuilder.html),
[limiter source](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/runtime/limits.rs#L10-L95),
[Config fuel/epoch source](https://github.com/bytecodealliance/wasmtime/blob/v49.0.3/crates/wasmtime/src/config.rs#L627-L791)
and [Trap](https://docs.rs/wasmtime/49.0.3/wasmtime/enum.Trap.html).

- A new store places no independent byte ceiling on linear memories.
  The default instance, table and memory count ceilings are **10,000**.
  Install `Store::limiter` with `StoreLimits` or a reviewed
  `ResourceLimiter` before loading untrusted guests. The
  `memory_size` setting is **per linear memory**, not total guest RSS.
- Limiters do not cover every Wasmtime allocation, Rust host allocation
  or C library. Denying guest `memory.grow` ordinarily returns `-1`.
  `trap_on_grow_failure` can alter that behavior, which must remain
  consistent with Moth's selected memory/allocation contract.
- `max_wasm_stack` bounds the Wasm stack, not native host recursion
  or the Rust thread stack.
- `consume_fuel` instruments guest work for deterministic fuel
  exhaustion. A store starts with zero fuel when enabled and needs a value
  supplied with `Store::set_fuel`. Benchmark the instrumentation under
  both Cranelift optimisation levels.
- `Config::epoch_interruption` provides lower-overhead external
  deadline checks with Cranelift in v49.0.3. Configure store deadlines and
  increment the engine epoch explicitly. Neither mechanism is a Lappet
  default until its resource-policy checkpoint decides semantics and cost.
- Neither epochs nor fuel interrupts a blocking Rust/C host call.
  Epoch checks have bulk-memory-operation granularity limitations.
  The game and server builders need explicit host IO and timeout policy,
  separately from guest execution budgets.
- Classify failures by phase: engine configuration, invalid module bytes,
  unsatisfied imports, instantiation/start trap, exported call trap,
  host adapter error and resource failure. Wasmtime errors may carry
  `Trap` or `WasmBacktrace` context, but that does not define Moth's
  error channel. Post-trap instance reuse is an **unresolved Lappet gate**.

## Features Lappet V1 does not need

| Subsystem | Why excluded |
|---|---|
| `wasmtime::component`, WIT, Canonical ABI | Foreign Wasm components and resource bridging are later work |
| `wasmtime-wasi` | No ambient OS services. Builders explicitly register trusted Rust imports |
| `async`, stack switching, Wasm threads | V1 is synchronous and builder-scheduled. Channels need a separate future design |
| Persistent cache, AOT and `Module::serialize/deserialize` | Deferred for predictable cold compilation benchmarks. In-memory module reuse is allowed |
| Winch, Pulley, other engines or custom interpreter layer | V1 uses Wasmtime Cranelift only. Revisit Winch after stability and performance evidence |
| GC integration and relaxed SIMD | Not part of the accepted V1 common feature profile |
| DWARF/native debugger machinery, profiler, `call-hook` | Only add on a demonstrated debugging/profiling requirement |
| Pooling-allocator/MPK tuning, raw Cranelift flags | Performance/security knobs require independent evidence |
| `wat` in production | Compile binary Moth Wasm. Parse WAT test fixtures in test tooling if needed |

No generic WASI service layer, HTTP server framework, automatic native
dependency loader or Wasm-level optimisation pipeline belongs in Lappet V1.
Trusted Rust/C host extensions remain a deliberate builder decision.
A small custom interpreter for embedded devices is a **different builder's
runtime**, not a requirement to make Wasmtime work on that device.

## Before implementing or broadening this reference

At the engine checkpoint, choose a concrete release and feature graph.
Confirm Cranelift proposal and OS/CPU support, compare development/release
native optimisation settings and establish resource limits, trap disposition
and host-call policies.

Prove `Engine::new` validation, import rejection, start-function traps,
typed exports, missing memory, invalid buffer spans, guest growth failures,
resource limits and store isolation using small core Wasm fixtures. Measure
separately clean Rust build, engine setup, `Module::new`,
`instantiate_pre`, instantiation, repeated guest calls, scalar host
calls, buffer copies and resident memory. Test long-lived game instances
and separate concurrent server instances. An authored Wasm fixture does
not establish the eventual Moth ABI.

Consult the linked **versioned source** when an API contract is subtle.
Record Lappet-specific findings that change an implementation choice,
not copies of the entire upstream manual. Extend this document only when
the implementation encounters a new owned concern or its approved
checkpoint changes.
