# Lappet host resource and context API

> Accepted design with implementation deferred. Lappet has no runtime,
> resource table or context code yet. [The roadmap](../roadmap.md) owns status.

This page owns how a Rust embedder gives a Lappet instance opaque host
resources and per-call host context, and how host functions reach them.

| Owner | Contract |
|---|---|
| [Native host ABI](../../../docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf) | The `i64` token carrier, where a token may appear and what the guest may do with it |
| [Host packages](../../../docs/developer-docs/project-builders/host-packages.mtf) | The `&T`, `&mut T` and context parameter shapes and their Moth meaning |
| [Host ABI adapter responsibilities](host-abi.md) | Token uniqueness, the checks before a Rust reference exists and the fault table |
| This page | The Rust API over those rules: ownership, lifecycle, access and failure reporting |

Rust names below are illustrative. The contracts are what is accepted.

## Strict first

V1 puts safety and correctness ahead of recovery. Every boundary violation
quarantines the whole instance, a stale token is never tolerated and no state
is repaired on the guest's behalf. This catches mistakes early while Moth and
Lappet stabilise. Long-running applications will later need more ways to
recover from unexpected state. That is future design with its own approved
contract, not something an implementation relaxes on its own.

## Constraints the design starts from

- A token is a nonzero `i64` from one process-wide generator, never reused. It
  reaches the guest only as an export argument. The guest stores it as a plain
  value and can't close it.
- Every use is checked against the instance's table, the registered type and
  the entry's lifecycle before a Rust reference exists.
- Host functions never call the guest, and V1 has no callbacks or re-entry.
  Host calls inside one guest entry therefore run one after another, never
  nested.
- Wasmtime requires store data to be `'static`. A host function reaches
  resources and context only through store data. So nothing borrowed from the
  embedder with a shorter lifetime can be visible to a host function without
  `unsafe`, which Lappet avoids.
- Both priority environments must work: a long-lived game world that creates
  and retires resources for hours, and a server that isolates requests.

Because of the fourth constraint, context can't be "borrowed for the call" in
the Rust lifetime sense. It is moved in for the call and moved back out.

## Two roles: identity and storage

The design gives resources and context different jobs.

- A **resource** is an identity Moth code holds and passes around. Its value is
  owned by the instance's table.
- **Context** is host state a call may touch that Moth code never names. Its
  value is owned by the embedder and lent to one guest entry at a time.

A game engine shouldn't move its world into Lappet to expose it. The intended
pattern keeps engine state in engine storage, reaches it through context and
uses small resource values as identities into it:

```rust
struct SpriteId(u32);                 // resource: an identity

struct Frame { sprites: SpriteStore } // context: the storage

fn set_position(frame: &mut Frame, sprite: &SpriteId, x: f32, y: f32) {
    frame.sprites.position[sprite.0 as usize] = (x, y);
}
```

A resource may also own real state, such as an open file, when the instance is
its natural owner.

## Resource table

Each instance owns one table. An entry is one token, one owned Rust value and
the value's registered type.

```rust
impl Instance {
    fn insert_resource<T: Resource>(&mut self, value: T)
        -> Result<ResourceToken<T>, ResourceError>;
    fn close_resource<T: Resource>(&mut self, token: ResourceToken<T>)
        -> Result<T, ResourceError>;
    fn resource<T: Resource>(&self, token: ResourceToken<T>)
        -> Result<&T, ResourceError>;
    fn resource_mut<T: Resource>(&mut self, token: ResourceToken<T>)
        -> Result<&mut T, ResourceError>;
    fn drain_resources(&mut self) -> DrainedResources;
}
```

- **Ownership.** The table owns each value. A resource type is any `'static`
  Rust type registered with the runtime that meets the instance kind's thread
  rule under [Instance kinds and threads](#instance-kinds-and-threads).
  Registration is what later ties the Rust type to its canonical Moth type.
- **Typed tokens.** `ResourceToken<T>` is a `Copy` wrapper over the nonzero
  token that remembers `T` at compile time. Embedder code can't pass a
  `Window` token where a `Texture` is expected. The raw `i64` exists only at
  the ABI, where the runtime check still runs because a guest can send any
  bits.
- **One token, one entry.** V1 has no way to mint a second token for an
  existing entry. Conflict tracking therefore keys on the entry. Two resource
  values that share state internally, such as two `Arc` clones, are two entries
  and the sharing is the embedder's own safe Rust.
- **Close returns the value.** Closing removes the entry and hands the value
  back, so the embedder runs its own observable teardown at a point it chose.
  Rust drop timing decides nothing Moth can observe.
- **Table changes need the instance.** Insert, close and drain take
  `&mut Instance`. A guest call also holds the instance exclusively. So the
  table can't change while guest code runs, and a host function can't create
  or close a resource. That is the V1 rule that the guest can't close, enforced
  by the borrow checker.
- **Failures here are errors, not faults.** An unregistered type, a full table,
  an exhausted generator or a token that isn't live returns `ResourceError` to
  the embedder. No guest code is involved, so the instance isn't quarantined.

### Closed entries are removed

A closed entry leaves the table at once. The table doesn't keep a closed
marker.

Tokens are never reused, so a removed token can never name a later entry. A
marker would only let the fault report say "closed" instead of "not live", and
a long-lived game world would pay for every resource it ever closed until
teardown. The fault report can still distinguish a token at or above the
generator's next value, which was never issued, from a lower one, which is
closed or belongs to another instance.

### Faults keep the table

Quarantine blocks guest calls only. On a faulted instance the embedder can
still close, inspect and drain resources, so a server recovers its handles and
a game saves what it can. Dropping an instance drops remaining values in
ascending token order, which is creation order.

## Access during a host call

A host function reaches a resource only through the token the guest passed.
The adapter resolves every resource parameter before the function body runs.

1. Look the token up in this instance's table. Not live is a boundary fault.
2. Check the entry's type against the parameter's registered type. A mismatch
   is a boundary fault.
3. Acquire shared access for `&T` or exclusive access for `&mut T`. A conflict
   is a boundary fault.
4. Only then construct the Rust reference and call the function.
5. Release every access when the function returns or panics.

Because host calls never nest, a conflict can arise only inside one call's own
parameters: the same token passed twice with at least one exclusive use.
Passing one token as two shared parameters is legal.

Each entry carries a runtime shared-or-exclusive borrow state, the same
discipline as the standard library's `RefCell`, and the guard is that borrow
narrowed to `T`. This needs no `unsafe` and no table-wide lock. The rule stays
correct unchanged if a later profile allows nested host calls.

## Passing tokens to exports

A token enters the guest as an export argument. Before entering the guest,
Lappet checks that each token argument is live in this instance and has the
type the export declares. A failure is an error returned to the embedder and
the guest isn't entered, so the instance stays usable. The same mistake made by
the guest later is a boundary fault.

A guest may keep a token after the embedder closes its entry. Using it then is
a boundary fault and quarantines the instance. V1 keeps this strict. An
embedder that retires a resource while Moth code may still hold its token
first tells the Moth program through an ordinary export, or keeps the entry and
marks its own value retired.

## Host context

The embedder owns a context holder and passes it to each guest entry call.

```rust
let mut context = HostContext::new(Frame { sprites });

instance.call(&update, arguments, &mut context)?;

let frame: &mut Frame = context.get_mut::<Frame>()?;
```

- **Moved in, moved out.** For the call Lappet moves the boxed value from the
  holder into the instance's context slot and moves it back before returning.
  It comes back on success, on a declared Moth error and on every fault,
  including a caught host panic. The holder allocates once, not per call.
- **Never retained.** The slot is empty between calls. One instance serves
  successive requests with different context and can't leak one request's
  state into the next.
- **One value per call.** The holder carries one value of one type, or none. A
  host function names the context type it needs. A missing value or another
  type is a host fault and the function isn't called. An application that needs
  several pieces of state passes one struct that holds them.
- **Scope is the embedder's choice.** Passing the same holder to every call
  gives application scope. A holder per request gives request scope. There is
  one mechanism.
- **Exclusive access is automatic.** One slot and sequential host calls mean a
  `&mut` context can't conflict with anything.

## What a host function receives

The adapters, manual today and macro-generated later, hand a function its
resolved arguments. They reach resources and context through one view of the
running call:

```rust
impl HostCall<'_> {
    fn resource<T: Resource>(&self, token: i64) -> Result<Shared<'_, T>, Fault>;
    fn resource_mut<T: Resource>(&self, token: i64) -> Result<Exclusive<'_, T>, Fault>;
    fn context<C: 'static>(&self) -> Result<Shared<'_, C>, Fault>;
    fn context_mut<C: 'static>(&self) -> Result<Exclusive<'_, C>, Fault>;
}
```

The view offers no insert, no close and no way to call the guest. A returned
`Fault` ends the call before the function body runs.

## Instance kinds and threads

An embedder chooses one of two instance kinds when it creates an instance.
Neither kind is ever `Sync`: one thread uses an instance at a time.

| Kind | Moves between threads | Resource and context values |
|---|---|---|
| Sendable instance, the default | Yes. The instance is `Send` | Must be `Send` |
| Thread-bound instance | No. The instance is neither `Send` nor `Sync` | Any `'static` value |

- A **sendable instance** suits a server that hands instances to worker
  threads. Its table and context accept only `Send` values.
- A **thread-bound instance** suits a game whose state must stay on one
  thread, such as a window handle or a graphics context. It accepts values
  that aren't `Send`, and the Rust compiler stops it leaving its thread.
- The kind is a compile-time property of the instance type, not a runtime
  flag. A value that isn't `Send` can't be inserted into a sendable instance
  or passed to it as context. Nothing is checked at run time and no thread
  identity is recorded.
- Both kinds share one implementation of the table, the access rules and the
  fault handling. They differ only in the thread bound on stored values. Don't
  build two tables.
- One compiled module serves both kinds. Compiled code and host function
  implementations are shared and thread-safe either way. Only instance state
  differs.
- Tokens come from the same process-wide generator for both kinds, so a token
  from a thread-bound instance is never live in another instance.
- A context holder follows the same rule. A sendable instance takes a holder
  of a `Send` value. A thread-bound instance takes either.

Confirm against the pinned Wasmtime release that a store whose data isn't
`Send` supports every call Lappet makes on it.

## Failure summary

| Situation | Outcome | Instance afterwards |
|---|---|---|
| Embedder inserts an unregistered type, the table is full or the generator is exhausted | `ResourceError` | Usable |
| Embedder closes or inspects a token that isn't live or has another type | `ResourceError` | Usable |
| Embedder passes a bad token as an export argument | Error before guest entry | Usable |
| Guest passes a token that isn't live, has the wrong type or conflicts | Boundary fault | Quarantined, resources recoverable |
| Host function needs context that is missing or has another type | Host fault | Quarantined, context returned |
| Host function panics while holding access | Host fault, access released | Quarantined, context returned |

## Extensions the design leaves room for

None of these is built in V1. Each is listed because the V1 shape must not
block it.

- **Resource results and guest-side close.** The ABI already fixes their
  carrier. They need a host function to change the table during a call, which
  V1 forbids by construction. A later profile adds table access to the
  running-call view. Removing closed entries and keying conflicts on entries
  both stay valid.
- **Optional resources.** A layout exists in the ABI. The API needs only an
  `Option<ResourceToken<T>>` argument and parameter shape.
- **Bulk close for a lifecycle.** Tokens increase over time, so "close
  everything created since this mark" needs only a remembered token value. It
  suits a frame or request lifecycle on a long-lived instance. Add it when a
  builder needs it.
- **Several tokens for one entry.** If a later profile lets the host hand out
  an alias, conflict tracking already keys on the entry, not the token.
- **Nested host calls.** Per-entry borrow state already handles them.
- **A more forgiving fault model.** Recovering an instance after a stale token
  or another contained violation needs an approved contract and, for export
  calls, a reset helper in a later ABI version. V1's strictness is a starting
  point, not a permanent rule.
