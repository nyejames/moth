# Lappet host ABI adapter responsibilities

> Proposed design pending approval. Lappet has no runtime, adapter or
> registration code yet. [The roadmap](../roadmap.md) owns status.

This document owns Lappet's side of the V1 native host boundary: installing
Rust implementations, checking and converting boundary values, the staging and
resource tables, host context and fault handling. It does not define the ABI.

| Owner | Contract |
|---|---|
| [Native host ABI](../../../docs/src/developer-docs/memory-management/native-host-abi/native-host-abi.mtf) | Carriers, frames, validation direction, staged-text and call-scope protocols, resource-token carriage, import/export identity and the host manifest |
| [Host packages](../../../docs/src/developer-docs/project-builders/host-packages.mtf) | The Rust signature profile and the Moth meaning of each shape |
| [Memory lowering](../../../docs/src/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf) | The restricted host-binding profile and `ValidatedMemoryPlan` |
| [Runtime design](design.md) | Instances, scheduling, capabilities and failure categories |

When this document and a compiler authority disagree, the compiler authority
wins and the drift is a bug here.

## Installing implementations

A builder describes a host binding once. The compiler registers that descriptor
in its external-package registry and later records the reachable bindings in
the artefact's host manifest. Lappet installs a Rust implementation against the
same descriptor's canonical package and symbol identity. Lappet has no second
signature language, type registry or name-based discovery.

Before guest start, Lappet checks that:

- every manifest import has exactly one implementation with the same canonical
  identity, parameter types, success slots and error channel
- the module's actual imports and exports match the manifest's physical names
  and Wasm signatures
- the ABI version, memory and helper set match the profile Lappet implements
- every required context type and resource type is available to this instance

Any mismatch rejects the instance before guest code runs. Lappet never installs
an unrequested import, a default stub or a trapping placeholder. An implementation
for a binding the artefact doesn't import is unused, not an error.

## Converting values

The adapter reads every argument through the descriptor's canonical type, never
through its Wasm carrier alone. Validation completes before Rust runs. A failed
check is a boundary fault and the Rust function isn't called.

| Rust shape | Adapter duty |
|---|---|
| `bool`, `char` | Check `0`/`1` and Unicode scalar validity |
| `i8`..`i64`, `u8`..`u64` | Check the exact range of the declared type, then convert |
| `moth::host::Byte` | Check `0` through `255`. Never accept a `U8` descriptor for `Byte` or the reverse |
| `f32`, `f64` | Pass import results through raw. The compiler's external float guard decides their delivery |
| `&str` | Check span bounds, length and UTF-8, then borrow or copy for this call only |
| `String` result | Check the length limit, stage the bytes and write an `AbiStagedText` |
| `Option<T>` | Check the tag and absent-payload zeroing. Keep absence distinct from failure |
| `()` | Write no result |
| `Result<T, moth::host::Error>` | `Ok` writes the success slot and returns status `0`. `Err` stages the message, writes the code, including `0`, and returns status `1` |
| `&T`, `&mut T` resource | Resolve the token and acquire shared or exclusive access |

Every other result is checked against its canonical type before it's written,
so an out-of-range Rust value is a host fault rather than a Moth value. Guest
code trusts what the adapter writes, so these checks are the only ones.

A `&str` borrows guest memory only when the runtime can hold that view together
with the instance's context and resource tables without aliasing. Otherwise the
adapter copies into call-owned storage. Callers can't observe the difference.

## Staging table

Each instance owns one staging table for Rust-produced text. The ticket
protocol itself is defined by the native host ABI.

- Ticket values are nonzero, unique within the instance and never reused while
  it lives. They encode no host address.
- Entries have per-entry and per-instance byte limits, chosen with the runtime
  resource policy in milestone 3. Exceeding a limit is a host fault, not a Moth
  error.
- A take removes its entry. A failed take is a boundary fault.
- Lappet tracks live tickets so it can fault when the guest enters another
  import, or the outermost entry returns, with a ticket outstanding.
- Fault handling and teardown clear the table.

## Resource table

Each instance owns a table of its opaque host resources. Rust code creates and
closes resources through the host resource API, which is milestone 3 design.

- Tokens come from one generator per host runtime, so they are unique across
  that runtime's live instances and never reused. Exhausting the generator is a
  host fault.
- Every use checks the instance's table, the registered canonical resource type
  and the entry's lifecycle before a Rust reference exists. Stale, closed,
  wrong-type and other-instance tokens are boundary faults.
- A call-scoped guard grants shared or exclusive access. It tracks the
  underlying object and rejects any conflict, exclusive with exclusive or
  shared with exclusive, including two tokens for one object.
- Closing a resource marks its entry closed. Teardown drops every entry. Rust
  drop timing never decides observable Moth behaviour.

The guest can't close a resource or tell Lappet it dropped a token. A
long-lived instance, such as a game world, must close resources it no longer
needs through the host resource API, or its table grows until teardown. A
server can create an instance per request, so request resources can't leak into
another request through a shared table.

## Host context

A host function may mark at most one parameter as host context. Context isn't a
Moth parameter and has no Wasm carrier or descriptor slot. The embedder supplies
context for each guest entry call, so one instance can serve successive requests
with different context. Lappet checks its Rust type before the host call and
rejects a missing or mismatched context without calling the function. A mutable
context receives exclusive access for the call.

## Faults

| Outcome | Produced by | Moth sees |
|---|---|---|
| Declared error | `Err(moth::host::Error)` | Status `1` on the error edge |
| Boundary fault | Invalid carrier, span, tag, token, frame, ticket or result | Trap |
| Host fault | Panic, limit exhaustion or broken adapter invariant | Trap |
| Guest trap | Moth or Wasm execution, including a failed helper | Trap |

No fault becomes a Moth error. The adapter catches an unwinding Rust panic at
the call boundary and reports a host fault. A host built with `panic = "abort"`
aborts the process instead, and Lappet gives no Moth-level guarantee.

A fault ends the guest call and the faulting instance is quarantined. Lappet
then calls no guest helper, clears the staging table and discards values
already copied from the call. The milestone 3 runtime policy decides whether a
quarantined instance may be reused. Until it does, a faulted instance is
dropped. Host IO already performed is never rolled back.

## Calling Moth exports

Lappet calls only exports listed in the manifest, and only when no guest call is
active on that instance. Each call follows the native host ABI's call-scope
protocol and its fault rules. The Rust-facing call API, including several
ordered success slots, is milestone 3 design.

## Tests deferred to milestone 3

These need emitted Wasm or a Wasmtime store and stay blocked until the compiler
prerequisites on the ABI page exist.

1. `U8` and `Byte` descriptors with identical carriers reject each other's
   implementations and out-of-range values.
2. `none`, `Some("")`, status `1` with code `0` and a boundary fault stay
   distinguishable from Rust.
3. A two-slot export result keeps slot order, and the error edge never reads
   success fields.
4. A Rust `String` result is copied into guest storage. A failed take, or a live
   ticket at another import or at entry return, faults without leaking the entry.
5. Stale, closed, other-instance, wrong-type and conflicting tokens are rejected
   before the Rust function runs.
6. Missing, extra and mismatched imports fail before guest start under both
   `OptLevel::None` and `OptLevel::Speed`.
7. A trapping scope helper fails the whole export call and no further guest call
   runs.
