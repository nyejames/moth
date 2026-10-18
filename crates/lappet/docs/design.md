# Lappet runtime and embedding design

> Accepted direction, not delivered runtime support. The initial crate contains
> documentation only. [The roadmap](../roadmap.md) owns implementation status and
> explicit design checkpoints. An open checkpoint is not permission to invent
> its contract during implementation.

Lappet is Moth's official native WebAssembly execution library, Rust host bridge
and future package infrastructure. It belongs to the Moth workspace. Every
user-facing command stays under `moth`. There is no `lappet` executable.

Evaluate each runtime decision against two environments: an embedded Rust game
engine and a native server. Browser applications use browser builders rather
than executing inside Lappet.

## Authority and ownership

| Owner | Contract |
|---|---|
| [Compiler](../../../docs/compiler-design-overview.md) | Language semantics, canonical identities, compilation and validated backend handoffs |
| [Build system](../../../docs/build-system-design.md) | Project graphs, builder selection, assemblies, lifecycle inputs and output ownership |
| [Builder SDK](../../../docs/src/developer-docs/project-builders/overview.mtf) | Rust-facing builder definitions and host-binding descriptors |
| [Memory lowering](../../../docs/src/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf) | Value access, lifetime topology, physical ABI and validated memory plans |
| [Wasm optimisation](../../../docs/src/developer-docs/wasm-optimisation/overview.mtf) | Optimisation goals, pass ownership and compiler/runtime measurement policy |
| [Package manager](package-manager.md) | Acquisition, managed-package eligibility and admission policy |

Moth compiles and validates. Lappet executes the resulting Wasm with the
specified imports and runtime contract. The builder owns application behaviour,
host packages, scheduling and lifecycle policy. Lappet supplies mechanisms for
those policies rather than another compiler, game loop or server framework.

Use compiler-produced types, call identities, result contracts and layout facts.
Do not reconstruct them from raw Wasm signatures, source spelling or package
names. Runtime validation of an artefact's imports and bytes is still required.
It checks the execution boundary rather than repeating semantic compilation.

## Users and workspace dependencies

A power user embeds the compiler and Lappet as Rust libraries, provides host
packages and controls the rest of the native application. A user choosing an
official bundled builder needs only the distributed Moth CLI and its supported
host platform. The CLI distribution supplies the required builder code and
libraries without asking that user to install Rust, Cargo, Node.js or another
runtime. Repository development tools have separate prerequisites.

A custom builder may use the compiler without Lappet. A small device can execute
Moth output with its own interpreter and selected runtime services. This does
not require the Moth compiler to run on the device or become `no_std`.

The repository-root `moth` Cargo package owns the compiler library.
`crates/moth-cli` owns the CLI package and its `moth` executable:

```text
moth-cli -> moth compiler library
         -> lappet
lappet   -> public compiler/builder contracts where needed
```

The workspace defaults to `moth-cli` for root Cargo commands. Its features
forward to the compiler's matching features. Explicit compiler and workspace
validation retains library coverage independently of those defaults.

The CLI owns argument parsing, terminal presentation and scaffolding. Compiler
and build services retain compilation, output planning and writing, diagnostic
contexts and compiler instrumentation. High-level service handoffs keep raw
frontend stages and output internals private. Developer execution services such
as Boracle and the integration runner retain their compiler-owned implementation.
Builder selection passes a concrete builder value into one constructor. Its
stored backend and internal trait module remain private.

Keep Wasmtime out of the compiler library's dependencies. Preserve established
public facades without forwarding libraries, and keep executable discovery,
installation and release packaging attached to the CLI package.

The package-management library must be usable without Wasmtime. Prefer narrow
feature isolation in this crate. Extract another crate only when a real
compilation or ownership boundary needs it. Using Lappet as a library does not
make a custom builder an official Moth environment.

## Runtime strategy

V1 uses Wasmtime with Cranelift as its only native compiler. Development
defaults to `OptLevel::None` for faster native compilation and release to
`OptLevel::Speed` for optimised native code. Confirm both settings with
benchmarks. Moth's compilation profile is independent of the native
optimisation level. Use supported configuration APIs, not a custom native
backend or patched engine. V1 does not expose a Winch strategy selector or
arbitrary Cranelift tuning.

Lappet performs no Wasm post-processing optimisation. The shared optimisation
authority owns work done before Wasm emission and work delegated to native
code generation. Native optimisation settings cannot change source types,
safety validation, error contracts or memory-model legality.

Retain Cranelift's default backtracking register allocator. Wasmtime 49.0.3
marks its faster single-pass allocator not production-ready and outside normal
security-advisory support. Keep inlining disabled by default. Benchmark
`SpeedAndSize` and parallel module compilation before enabling either.
Verify debugger requirements against the selected Wasmtime release. Select
Cargo features explicitly and inspect their actual linked size.

V1 defers persistent compiled-code caches, AOT cache reuse and compiler-free
deployments. Reusing an already compiled module inside a running application is
allowed. Benchmarks distinguish that reuse from cold engine compilation and
record exactly which state is reused.

Publish support only for tested host and Wasm-feature combinations. The first
platform review evaluates Linux x86-64 and AArch64, macOS x86-64 and AArch64 and
Windows x86-64. These are validation candidates, not an implementation claim.
Validate Cranelift's Wasm feature support and host CPU on each advertised
platform. Reject unsupported requirements rather than selecting another
compiler. Winch can be reconsidered after upstream stability, compatibility
and measured performance justify it. Other engines and targets are later work.

## Rust bindings

Rust is the sole native host-binding interface in V1. Reuse Moth's manual typed
descriptors and its accepted `#[moth::host_function]` and `#[moth::host_type]`
conveniences. Both feed one checked registration path. Lappet does not introduce
another signature language, global plugin scan or general reflection system.

A builder describes its API once for compilation and installs the corresponding
Rust implementation for execution. Resolve and check imports before application
entry. Rust invokes only explicitly selected Moth callable exports through
checked wrappers. A public Moth package declaration is not automatically a host
export, network endpoint or a revived `moth_start` convention.

The starting profile contains exact scalars, strings, supported optionals, the
Moth error channel, explicit host context and opaque Rust-owned resources.
Byte buffers, records, choices, collections and ordered multiple results are
accepted priorities for the V1 binding design. Their concrete supported shapes,
conversions, physical packing and cleanup need approval before implementation.
This extends the existing restricted host profile deliberately rather than
claiming that every Rust type already has a Moth representation.

Keep these invariants throughout that work:

- Semantic types and result slots remain distinct from Wasm carriers and Rust
  layouts. `u8` is not automatically Moth `Byte`, and `repr(C)` does not make a
  Rust struct a Moth ABI type.
- Inputs have the documented non-retaining access contract. A call-scoped Rust
  view is allowed only while its backing storage remains valid. The adapter may
  copy. It promises values and lifetimes, not universal zero-copy access.
- The host retains no ordinary Moth reference after the call. Ordinary mutable
  Moth storage is not exposed through arbitrary Rust references or implicit
  copy-in/copy-out writeback.
- Returned ordinary values become independent Moth storage through the approved
  allocation and result ABI. Opaque resources remain host-owned identities.
- Validate ranges, finite floats, Unicode, lengths, handles and result shapes
  before exposing a successful value. Preserve declared errors separately from
  traps and boundary faults.

The ABI checkpoint must define allocation and cleanup control flow as well as
signatures. Runtime-internal marshalling helpers must not accidentally create
an unrestricted callback or guest re-entry facility.

## Trusted host libraries and Core implementations

A builder or an explicitly selected Lappet implementation can depend on Rust
libraries and vendor C libraries through Rust FFI. The Rust wrapper owns the C
ABI, allocator pairing and native safety obligations. Lappet sees only the
registered Rust-to-Moth contract.

These libraries run as trusted native host code, outside the Wasm sandbox.
Guest instruction limits do not make blocking native code interruptible or
prevent native memory corruption. The embedding author chooses which libraries
to expose. The Moth package manager does not fetch, compile or dynamically load
native host extensions as a consequence of adding a Moth dependency.

Lappet provides reusable native Core implementations. Canonical Core signatures,
access/effect facts and semantics remain compiler-owned. Reuse direct Wasm
lowering or compiler runtime helpers where they already implement an operation.
A scalar helper should not acquire a Rust host transition merely because this
crate exists. Compiler memory helpers are not ordinary foreign host bindings.

Trusted scalar-parametric Core registrations and file-backed JS/WAT authoring
remain under their Moth owners. Concrete types are resolved before target and
adapter selection. The generics work creates neither a generic foreign
container ABI nor permission to guess Dec or error-channel representations.
Lappet consumes delivered contracts without duplicating the provider or generic
specialisation machinery.

## Official Core and custom environments

Official executable builders provide the complete documented Core catalogue
through official native or browser implementations. Host-dependent operations
retain their canonical capability gates or declared unsupported outcomes.
An unavailable window title cannot become a successful silent no-op.

Documented approximation, nondeterminism and host differences belong in the
individual Core contracts and conformance tests. Builders cannot privately
weaken exact numeric, error, ownership or access semantics. The LSP and check
are tooling overlays over a selected builder's surface, not separate runtimes.

Custom builders may provide any subset of Core for local application code,
including none. Reserved Core identities and preluded `io` remain reserved.
Recognition does not imply that an executable implementation is available.
Compiler-internal helpers needed by advertised language features remain required,
even in an environment without public Core APIs.

Official package-manager admission is stricter than local compilation. Custom
builders can acquire only no-core managed packages, even if they use Lappet and
implement every Core API. Official Core eligibility comes from Moth-maintained
builder certification, never a self-declared flag or matching signatures. The
package-manager design owns this policy and its future trust checkpoint.

The HTML builder remains the initial browser implementation owner. Keep Core
semantics outside it and separate reusable browser implementations from HTML
routing and document assembly. Extract browser support into another crate when
another real builder needs it, not as speculative V1 structure.

## Instances, resources and scheduling

An execution instance owns its store, Moth memory and lifetime-scoped host state.
Compiled code may be shared without sharing mutable instance state. Resources
have a registered type, runtime owner and lifecycle. Reject stale, wrong-type,
foreign-instance and conflicting-access handles before constructing Rust
references. Two different handles do not prove two distinct mutable resources.
Observable close and teardown follow explicit contracts, not incidental Rust
or guest finalisation timing.

V1 calls are synchronous. The builder supplies lifecycle roots to compiler
validation and chooses when Moth runs. A game can keep one gameplay instance per
world. A server can use application or request isolation without implicit global
state sharing. A lifecycle root does not automatically require one new Wasm
instance, but reuse must preserve isolation and cleanup.

Arbitrary guest callbacks and re-entry into the same active instance are outside
V1. The instance checkpoint defines fault disposition, cleanup and whether an
instance can be used again after a trap. Neither retries nor transactional
rollback of earlier host IO are implied by a Moth error.

Moth channels and asynchronous execution will need coordinated compiler,
builder and runtime work later. V1 adds no event loop, task scheduler, async
host-signature convention or background callback delivery.

## Capabilities and failures

Install only the imports the builder explicitly provides. No ambient filesystem,
network or WASI capability is granted by default. Keep resource limits, execution
interruption and native host-call limits explicit. Select concrete defaults and
fault handling at the runtime policy checkpoint.

Preserve normal Moth errors, guest traps, adapter violations, operational host
failures and compiler diagnostics as distinct outcomes. A failed native contract
cannot manufacture an undeclared Moth error channel. A package's install-time
security assessment does not remove the need for runtime isolation.

HTTP, cryptography and other rich server helpers are later design. They may
remain builder-owned or become reusable Rust companion libraries. V1 establishes
the host mechanism without making these APIs mandatory Lappet packages.

## Foreign code and later extensions

V1 executes Moth core Wasm and registered Rust host imports. It does not load WIT
components, generate Canonical ABI adapters or execute C libraries compiled to
foreign Wasm. The earlier value-only WIT profile remains a possible first future
component contract, not a permanent ban on richer interoperability.

JS and WAT file-backed bindings require an explicit builder and target provider.
Lappet has no JavaScript execution. The scalar WAT provider work does not imply
that arbitrary external WAT/Wasm packages are available in Lappet. Its trusted
Core helpers and test fixtures are separate from managed foreign-package support.

## Validation and design gates

Begin runtime experiments with small core Wasm fixtures and public Wasmtime APIs.
They can establish a host adapter or runtime mechanism while Moth's Wasm backend
is incomplete. They do not prove the eventual Moth ABI or complete Core support.
End-to-end delivery waits for validated compiler export/import, layout and memory
handoffs, then uses real Moth projects in both priority environments.

The roadmap requires separate approval of workspace integration, host ABI,
instance/failure policy, platform features, official Core conformance and
package-manager contracts. Freeze each at its owning checkpoint. Keep current
status and validation evidence there instead of treating design prose as a
passing implementation test.
