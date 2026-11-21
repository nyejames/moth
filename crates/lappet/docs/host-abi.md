# Lappet host ABI adapter responsibilities

> The runtime and host boundary tables are in the tree. The adapter is not.
> This page remains the accepted design, and [the roadmap](../roadmap.md)
> owns delivery status.

This document owns Lappet's side of the V1 native host boundary: installing
Rust implementations, checking and converting boundary values, the staging and
resource tables, host context and fault handling. It does not define the ABI.

| Owner | Contract |
|---|---|
| [Native host ABI](../../../docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf) | Carriers, frames, validation direction, staged-text and call-scope protocols, resource-token carriage, import/export identity and the host manifest |
| [Host packages](../../../docs/developer-docs/project-builders/host-packages.mtf) | The Rust signature profile and the Moth meaning of each shape |
| [Memory lowering](../../../docs/developer-docs/memory-management/runtime-and-backend-lowering/runtime-and-backend-lowering.mtf) | The restricted host-binding profile and `ValidatedMemoryPlan` |
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
- every required context type and resource type is registered with this runtime

Any mismatch rejects the instance before guest code runs. Lappet never installs
an unrequested import, a default stub or a trapping placeholder. An implementation
for a binding the artefact doesn't import is unused, not an error.

`moth-host` describes required opaque resource parameters with their canonical
semantic type identity and shared or exclusive access mode. It also records a
host-only context requirement and its access mode separately from Moth parameter
slots. The compiler's existing descriptor conversion table and preflight remain
the single validation path. Rust-local `TypeId` checks the actual resource or
context object at runtime but is not an artefact identity, manifest type or
substitute for compiler-produced semantic identity. Neither Lappet nor a macro
adds a parallel type registry.

## Converting values

The adapter reads every argument through the descriptor's canonical type, never
through its Wasm carrier alone. Validation completes before Rust runs. A failed
check is a boundary fault and the Rust function isn't called.

| Rust shape | Adapter duty |
|---|---|
| `bool`, `char` | Check `0`/`1` and Unicode scalar validity |
| `i8`..`i64`, `u8`..`u64` | Check the exact range of the declared type, then convert |
| `moth::host::Byte` | Check `0` through `255`. Never accept a `U8` descriptor for `Byte` or the reverse |
| `f32`, `f64` | Check finiteness of import arguments and export arguments and results. Pass import results through raw, because the compiler's external float guard decides their delivery |
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

- Ticket values come from one per-instance `u64` counter starting at 1. They
  are nonzero, unique within the instance and never reused while it lives.
  They encode no host address.
- By default an entry holds at most 16 MiB and one instance holds at most 64 MiB
  of live staged text. `InstanceLimits::max_staged_entry_bytes` and
  `InstanceLimits::max_staged_live_bytes` set these limits for an instance.
  The native ABI's maximum text length of `i32::MAX` bytes applies regardless
  of how high either configurable limit is set. Exceeding a limit is a host
  fault, not a Moth error.
- A take removes its entry. A failed take is a boundary fault.
- Lappet tracks live tickets so it can fault when the guest enters another
  import, or the outermost entry returns, with a ticket outstanding.
- Fault handling and teardown clear the table.

## Resource table

Each instance owns a table of its opaque host resources. Rust code creates and
closes resources through the
[host resource and context API](host-resources.md), which owns the Rust shape,
ownership and failure reporting.

- Tokens come from one process-wide `u64` counter starting at 1, so they are
  nonzero, unique across every runtime and instance in the process and never
  reused. A token from another runtime is therefore never live in this table.
  Exhausting the generator is an error returned from insertion, not a host
  fault, because no guest call is running.
- By default an instance holds at most 1,048,576 live entries, set by
  `InstanceLimits::max_live_resources`. The next insert returns a
  `ResourceError` and leaves the table unchanged.
- V1 mints one token per entry. Every use from the guest checks the instance's
  table, the registered canonical resource type and that the entry is live
  before a Rust reference exists. Stale, closed, wrong-type and other-instance
  tokens are boundary faults.
- A call-scoped guard grants shared or exclusive access. It tracks the
  underlying object and rejects any conflict, exclusive with exclusive or
  shared with exclusive, including two tokens for one object. Host calls never
  nest in V1, so a conflict can arise only within one host call's own
  parameters.
- Closing a resource removes its entry at once and returns its value to the
  embedder. The table keeps no closed marker, because a token is never reused.
- The table changes only between guest calls. A host function can't create or
  close a resource.
- Quarantine blocks guest calls only. The embedder can still close, inspect
  and drain the resources of a faulted instance. Teardown drops remaining
  entries in creation order. Rust drop timing never decides observable Moth
  behaviour.
- A token the embedder passes as an export argument is checked before guest
  entry. A token that isn't live or has the wrong type is an error returned to
  the embedder, the guest isn't entered and the instance stays usable.

The guest can't close a resource or tell Lappet it dropped a token. A
long-lived instance, such as a game world, must close resources it no longer
needs through the host resource API, or its table grows until teardown. A
guest that uses a token after its entry closed takes a boundary fault, so the
embedder coordinates each close with the Moth program. A server can create an
instance per request, so request resources can't leak into another request
through a shared table.

## Host context

A host function may mark at most one parameter as host context. Context isn't a
Moth parameter and has no Wasm carrier or descriptor slot. The embedder supplies
context for each guest entry call, so one instance can serve successive requests
with different context. The value is moved into the instance for the call and
returned to the embedder afterwards on every outcome, including a fault. The
instance retains none between calls. Lappet checks its Rust type before the
host call. A missing or mismatched context is a host fault and the function
isn't called. A mutable context receives exclusive access for the call.

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

V1 is deliberately strict. Every violation above quarantines the whole
instance, with no partial recovery, so that mistakes surface early while Moth
and Lappet stabilise. A more forgiving model for long-running applications is
later design and needs its own approved contract.

A fault ends the guest call and the faulting instance is quarantined. Lappet
then calls no guest helper, clears the staging table and discards values
already copied from the call. The V1 helper set has no reset helper, so until
a later ABI version adds one, a faulted instance is dropped. Host IO already
performed is never rolled back.

## Calling Moth exports

Lappet exposes a selected export only through a handle validated against the
compiler's manifest. The Rust call interface accepts checked values for the
manifest's ordered parameter types and returns checked values for its ordered
success slots. A Rust tuple may group multiple returned slots. A declared
builtin Moth `Error!` keeps its message and code and is separate from the
runtime error channel. No raw Wasm function, carrier-level call API or second
semantic signature declaration becomes part of the public embedding API.

A selected call enters only when no guest call is active on the instance and
follows the native host ABI's full call-scope protocol. A compiler-produced
host-callable artefact must have no implicit Wasm start function. Lappet checks
that requirement during manifest validation, before instantiation. The builder
owns when each selected function runs.

Reject invalid or oversized Rust-supplied export arguments before guest entry.
These are caller-facing `ExportArgument` errors and do not quarantine an
otherwise usable instance. Once guest execution begins, invalid guest output,
malformed ABI values and broken resource/staging invariants take the defined
boundary/host fault lane and quarantine the instance. A previously quarantined
instance rejects all attempted guest calls before token preflight, context
installation or epoch-deadline changes.

Returned Moth strings are copied into independently owned Rust storage before
scope close. Apply an explicit per-result byte bound before allocating the copy,
in addition to checking the ABI's `i32::MAX` length ceiling, span validity and
UTF-8. Excessive or malformed guest output is a boundary fault and quarantines
the instance. The adapter implementation checkpoint must approve a concrete
default and any configuration surface for the smaller operational bound. It
must not copy unbounded guest memory while that limit remains undecided.

The current fixture-only scope driver accepts one raw string and reports status
`1` without reading the builtin error record. Replace that path with the
manifest-driven adapter when it is delivered, rather than exposing or
maintaining a second public calling convention. Report malformed status,
span length, span bounds and UTF-8 as the appropriate distinct boundary facts.

Lappet also implements the reserved `assertion_failed` helper once the
compiler freezes its signature. It reports a guest trap.

## Deferred tests

Each test needs a Wasmtime store. Every fixture-level test now has coverage.
Its marker names the test that covers it, or what it still waits for.

- **Covered:** the named test under `crates/lappet/src/runtime/tests/`
  exercises the fixture-level part under both optimisation levels.
- **Adapter:** needs the value conversions of the host adapters, which read
  the shared descriptors in `moth-host`. The adapters arrive with the
  real-application gate because nothing calls them until compiled Moth imports
  a host function.
- **Compiler:** needs Wasm emitted by the Moth backend, so it waits for the
  compiler prerequisites on the ABI page and the real-application gate.

Every covered test is repeated against compiled Moth when that gate opens.

1. `U8` and `Byte` descriptors with identical carriers reject each other's
   implementations and out-of-range values. The descriptor half is **covered
   by** `crates/moth-host/src/tests/preflight_tests.rs::byte_and_u8_are_distinct_identities`.
   Rejecting an out-of-range value is **Adapter.**
2. `none`, `Some("")`, status `1` with code `0` and a boundary fault stay
   distinguishable from Rust. **Compiler.** Waits for host-import lowering to
   `moth-host-v1` names, fallible-call lowering through `out_frame` and
   `status`, builtin `Error` lowering and the `ValidatedMemoryPlan` producer
   for boundary string, frame and scope allocations.
3. A two-slot export result keeps slot order, and the error edge never reads
   success fields. **Compiler.** Waits for compiled Moth output and the
   builder-selected export wrappers recorded under
   [Waiting on main](../roadmap.md#waiting-on-main).
4. A Rust `String` result is copied into guest storage. A failed take, or a live
   ticket at another import or at entry return, faults without leaking the entry.
   **Covered by** `staged_text_tests.rs::staged_text_is_copied_and_the_ticket_is_consumed`,
   `staged_text_tests.rs::a_bad_take_quarantines_and_clears_the_table`,
   `staged_text_tests.rs::taking_one_ticket_twice_is_a_boundary_fault`,
   `staged_text_tests.rs::a_live_ticket_at_another_import_is_a_boundary_fault`
   and `staged_text_tests.rs::a_live_ticket_at_entry_return_is_a_boundary_fault`.
   The `String` conversion is **Adapter.**
5. Stale, closed, other-instance, wrong-type and conflicting tokens are rejected
   before the Rust function runs. **Covered by**
   `access_tests.rs::a_token_that_is_not_live_is_rejected_before_the_host_body`,
   `access_tests.rs::a_token_from_another_instance_is_not_live`,
   `access_tests.rs::a_wrong_type_is_rejected_before_the_host_body`,
   `access_tests.rs::two_shared_borrows_coexist_and_an_exclusive_borrow_conflicts`,
   `access_tests.rs::a_guard_is_released_after_the_call_and_after_a_panic` and
   `resource_tests.rs::zero_is_not_a_token_and_a_closed_token_is_not_live`.
6. Missing, extra and mismatched imports fail before guest start under both
   `OptLevel::None` and `OptLevel::Speed`. **Covered by**
   `instance_tests.rs::import_the_host_does_not_provide_is_a_missing_import`,
   `instance_tests.rs::an_extra_physical_import_fails_before_guest_start` and
   `instance_tests.rs::import_with_another_physical_signature_is_a_mismatch`.
   Matching against the manifest is **Compiler** and waits for the host
   manifest and builder-selected export wrappers.
7. A trapping scope helper fails the whole export call and no further guest call
   runs. **Covered by**
   `scope_tests.rs::a_trapping_scope_step_skips_close_and_makes_no_further_guest_call`
   and
   `scope_tests.rs::a_trapping_scope_close_fails_the_call_and_discards_the_copied_result`.
