# Core IO package plan

## Role and authority

`@core/io` is Moth's broad, high-level host IO package and the only Core package exposed through the
prelude. It is a host-capability facade: each operation describes what the program wants from its
environment, and the active backend chooses the most appropriate host facility to realise it. The
package has Core origin, ExternalBinding backing and the bare `io` prelude alias.

Canonical source semantics belong to `docs/src/docs/packages/core/io/io.mtf` and prelude policy
belongs to `docs/src/docs/packages/core/prelude/prelude.mtf`. The queued title helper's capability
rules also appear in the HTML builder section of `docs/build-system-design.md`. This plan records
the accepted scope rationale, implementation knowledge, audit findings and sequencing. It cannot
accept public API or semantics.

### Current-state capsule

```text
STATUS: activated; scope and prelude review accepted; Phase 1 snapshot input hardening complete
CURRENT_SLICE: Phase 2 - portable snapshot additions (candidate selection not started)
BLOCKERS: Phase 2 needs its candidates chosen with the user; the ordered event queue needs a final
Moth event value that crosses the binding boundary; io.set_title needs the config and HTML entry
cutover; scheduling, timers and callbacks need structured async and a host-ingress contract
NEXT_ACTION: choose the Phase 2 snapshot additions with the user, then publish and implement them
```

## Current surface

Registered in `src/builder_surface/core_packages/io.rs`: twenty functions and one opaque type. Every
function has a JavaScript runtime-helper lowering and `wasm: None`. Stable identities and helper
names live in `ExternalFunctionId` (`src/compiler_frontend/external_packages/ids.rs`), so registry
names and emitted helper names cannot drift.

| Group | Symbols | Signature shape |
|---|---|---|
| Console | `print`, `line`, `debug`, `warn`, `error` | one shared `StringContent` parameter, no result, no error channel |
| Handle | `input.Input` | opaque `Handle`, `ExternalTypeId(1)` |
| Lifecycle | `input.new`, `input.update`, `input.close` | `new -> Input, Error!` is the only fallible function; `update` and `close` take `~Input` |
| Keyboard | `input.key_down`, `input.key_pressed`, `input.key_released`, `input.last_key_pressed`, `input.last_key_released` | shared `Input` plus `String` key `-> Bool`, or `Input -> String?` |
| Pointer | `input.pointer_down`, `input.pointer_pressed`, `input.pointer_released`, `input.pointer_x`, `input.pointer_y`, `input.last_pointer_pressed`, `input.last_pointer_released` | shared `Input` plus `String` button `-> Bool`, `Input -> F64` or `Input -> String?` |

F64 coordinates are the accepted exact numeric contract, pending implementation
cutover from profile-based Float. Preserve coordinate units, pointer semantics
and the existing Wasm rejection. Shared finite-result validation and its
documented fatal/builtin Error! delivery remain independent from authored
safety-catch eligibility.

`register_core_prelude` (`src/builder_surface/core_packages/prelude.rs`) registers bare `io` as a
compile-time namespace alias. `io.set_title` is accepted queued design with no registration,
lowering, capability plumbing or tests.

## Implementation notes

### Console lowering

`src/backends/js/package_bindings/core/io.rs` emits one shared `__moth_io_write` plus each
referenced console helper. `print` and `line` both emit one `console.log` record because browser
consoles are record-oriented. Appending `"\n"` would only render a blank line, and buffering `print`
calls into one entry would delay output. `debug`, `warn` and `error` fall back to `console.log` when
the specific console method is missing.

### Input host state

`src/backends/js/package_bindings/core/io_input.js` is one helper blob, emitted whole when any input
helper is reachable and never pulled in by console-only use. `io.rs` interpolates the
`BuiltinErrorCode::Unsupported` result into it through the shared `error_result_source` formatter in
`src/backends/js/runtime/errors.rs`, which collections and Time also use. `__moth_io_input_new`
returns that error unless `window`, `document`, `AbortController` and `window.PointerEvent` all exist.

The handle has three lanes:

- **Live state** written by listeners: a `liveKeys` map from key identity to the logical name
  recorded at keydown, `liveButtons` and the live pointer position.
- **Pending transitions** since the previous update: pressed and released sets for keys and buttons
  plus the four `last_*` values. Repeats coalesce into the same set entry, so storage depends on
  distinct keys and buttons rather than event count.
- **Published snapshot**: held key and button sets, the pointer position and the transition record
  published by the last update. Every read observes only this lane.

`update` rebuilds the held sets from live state, copies the pointer position and swaps the pending
and published transition records, so publication allocates nothing. `close` aborts the listeners,
sets `closed` and clears all three lanes. `update` on a closed handle returns early, so a listener
that still fires after close can't publish anything.

Key identity is `code:` plus `event.code` when the host supplies a usable code, otherwise `key:` plus
the normalised name. A keyup releases the identity recorded at keydown and therefore the name recorded
then, so modifier changes can't strand a key. A keyup without a usable code releases every held key
with the same name. A logical name is pressed when its first identity goes down and released when
its last goes up, so both Shift keys share one `"Shift"` state. Keyup or pointerup without a recorded
press is not a transition. Normalisation maps `" "` to `"Space"` and lowercases single ASCII `A`-`Z`
on ingest and on every query. Button queries are not normalised.

Host-owned listeners, all passive and all removed through one `AbortController` signal:

- `window` `keydown` and `keyup` drive key transitions as above.
- `window` `pointerdown`, `pointerup` and `pointermove` update the live `clientX`/`clientY`. Down and
  up drive transitions for buttons 0, 1 and 2 only.
- `window` `pointercancel` releases held buttons. `window` `blur` and a hidden `document`
  `visibilitychange` release every held key and button. The releases publish at the next update and
  the pointer position is kept.
- `preventDefault` is never called.

The browser can't run listeners while Moth code runs. Chorded mouse buttons pressed while another is
already down arrive as `pointermove` with a changed `buttons` mask rather than `pointerdown`, so they
record no transition today.

### Wasm rejection

HTML-Wasm rejects reachable Core IO calls before lowering through
`validate_hir_external_package_support` (`MOTH-RULE-0058`, rendered with the leaf path such as
`input.new`). `resolve_host_function_id` in the Wasm `hir_to_lir` import owner is an unconditional
backstop. Unreachable IO calls create no requirement.

## Design rationale worth preserving

- **Facade, not platform mirror.** `io` is broad, opinionated and easy to reach rather than the most
  flexible API for each capability. Its operations describe intent, such as setting the title of the
  current user-facing host surface. Backends choose the realisation but never redefine the contract,
  and a host without the capability rejects reachable calls through target validation.
- **Overlap at the convenient edge only.** `io` may offer the common operation of a domain that a
  focused package owns in detail, but it never duplicates that package's complete API or
  reimplements its semantics. Shared behaviour uses one compiler or runtime owner. `@core/time` owns
  time values, so `io` is not a second time library. Targeted browser input belongs to future
  `@web/*` packages. A future filesystem package owns paths and metadata while `io` may expose a few
  common high-level file operations. A focused package existing is not a reason to make common IO
  inconvenient.
- **Prelude for convenience.** `io` is preluded because ordinary programs need common host
  interaction immediately, not because it is semantically special. That justifies a broader package
  than other Core packages, not an unstructured one. Related capabilities share a namespace such as
  `io.input`. Preluding another Core package is a separate language-wide decision.
- **Pull-driven events.** Host listeners write into an opaque handle and Moth reads it through
  ordinary synchronous calls. Moth never receives a host callback. The handle has explicit teardown,
  and host garbage collection does not define observable resource lifetime.
- **Event storage is not execution scheduling.** Polling suits an existing execution opportunity:
  an update loop, an engine tick, a future frame callback or another host-driven invocation. A host
  event that arrives after Moth returns stays unobserved until something runs Moth again. A
  synchronous polling loop blocks the browser event loop that would deliver the event.
- **Final shape or defer for events.** An ordered event queue must return ordinary Moth event
  values. It must not ship as an opaque `Event` handle because the binding ABI cannot yet return
  the right choice or record shape.
- **Async owns suspension.** Sleep, timeouts, intervals, event waits, fetch, persistent network
  events and callbacks into Moth need a runtime that suspends work, returns to the host and resumes
  later. The intended route is host event, runtime host adapter, ordinary Moth value, channel,
  structured scheduler and waiting coroutine. Structured async and channels must first establish
  scheduler, suspension, bounded task lifetime and value transfer. A later host-ingress contract must
  then define how a host callback queues a value, wakes the scheduler, handles re-entry and tears
  down its registration.
- **No callback ABI by imitation.** Moth does not adopt JavaScript's callback architecture as its
  asynchronous model. Design a callback ABI only if channels and structured scheduling demonstrably
  cannot express an important capability.

## Current work

### Phase 2 - portable snapshot additions

Choose the additions with the user before publishing or implementing any of them. Candidates:
pointer movement deltas, wheel or scroll accumulation, modifier-key state, focus or active-state
queries, a physical key or scancode API, typed key and button values and further pointer information
that maps cleanly across hosts. Each needs a portable contract, stays global and application-oriented
and joins the existing snapshot published by `update`.

## Known gaps and next extensions

### Coverage limits

The integration runtime harness has no `window`, so it executes only the `new` failure path. Input
semantics are owned by the Node scenarios in `src/backends/js/tests/io_input_runtime.rs`, which
install a fake `window`, `document`, `EventTarget` and `AbortController` and run the emitted helpers.
A minimal window host in the harness would let a rich Moth input scenario own them end to end.

Chorded mouse buttons that arrive only through `pointermove` record no transition.

### Next extensions, in order

Candidates only. Nothing here is accepted API.

1. **Ordered event value investigation.** Define the final Moth event value first, likely a choice
   with typed key and button payloads. Registered binding signatures currently carry scalars,
   `String`, `Char`, `Bool`, options and opaque handles, with no choice, record or collection
   results. If the value cannot cross cleanly, keep the ordered queue deferred and continue snapshot
   polling.
2. **Ordered pull queue.** Only after the value crosses cleanly: a host queue drained by an explicit
   poll into ordinary Moth values, with its own representation, ordering and overflow contract. The
   snapshot's coalesced transitions are not a queue and impose no size limit to inherit.
3. **Queued `io.set_title`.** Its implementation is owned by the HTML page directives and runtime
   title work and needs the config and HTML entry cutover first. Its meaning is host-neutral and
   HTML-JS realises it as the live document title.
4. **Small synchronous host conveniences.** Review each individually rather than adding a broad
   speculative IO surface.

## Longer-term candidates

Behind structured async, channels and the host-ingress contract: general callbacks into Moth,
listener APIs that run Moth later, timers, intervals, sleep, delay, event waits, network and fetch
completion, WebSocket and server-sent events, asynchronous file selection, asynchronous image
loading and any API whose result must wake suspended Moth work. Event-time browser decisions such as
arbitrary `preventDefault` or propagation control also wait unless they can be configured completely
at registration.

Outside this package: targeted DOM sources, propagation phases, default suppression and composition
events belong to `@web/*`. Complete filesystem and networking APIs belong to focused packages. A Wasm
lowering set needs its own target decision.

## Previous blockers and rejected approaches

- Reject an opaque `Event` handle as a stand-in for the final event value.
- Reject synchronous-looking `sleep`, timeout, interval, event-wait, fetch or callback forms before
  async V1.
- Reject a general callback ABI added to imitate JavaScript.
- Reject synchronous polling loops as a way to wait for input.
- Reject copying a focused package's complete API, or reimplementing its semantics, into `io`.
- Reject preluding further Core packages through package-level decisions.
- Reject exposing platform-specific implementation details in source-visible semantics.

## Validation and integration coverage

Primary owners:

| Contract | Owner |
|---|---|
| `print` and `line` content, order and one record per call | `tests/cases/core_io_console_print_line_records` |
| Remaining console helper emission per output lane | `tests/cases/core_io_console_debug_success`, `_warn_success`, `_error_success` |
| `line` emission and `@core/io as` alias binding | `tests/cases/core_io_dependency_alias_success` |
| Bare prelude namespace, non-callable namespace, type-clause misuse | `tests/cases/core_io_namespace_binding_success`, `io_callable_rejected`, `core_io_type_dependency_rejected` |
| String-only console boundary and template coercion | `tests/cases/io_coerce_to_string`, `io_rejects_struct_value`, `io_rejects_collection_value`, `io_rejects_option_value`, `io_rejects_multiple_return_value`, `io_rejects_result_value` |
| `io.input.new` returns `Unsupported` code 1 through Moth recovery | `tests/cases/core_io_input_new_unsupported_host` |
| Input helper reachability, handle passing and optional reads | `tests/cases/core_io_input_signature_success`, `core_io_input_reads_success`, `core_io_input_last_key_optional_success`, `src/backends/js/tests/prelude.rs` |
| Snapshot publication, transitions, repeat suppression, coalescing, `last_*` reads, key identity and naming, focus and visibility release, teardown, passive listeners and bounded storage | `src/backends/js/tests/io_input_runtime.rs` |
| Mutable-access, argument-type and type-as-value rejection | `tests/cases/core_io_input_update_missing_mutable_rejected`, `core_io_input_non_string_key_rejected`, `core_io_input_type_as_value_rejected` |
| Reachable HTML-Wasm rejection and unreachable acceptance | `tests/cases/core_io_input_wasm_rejected`, `core_io_unreachable_io_wrapper_wasm_ignored` |
| Unsigned code record under both Int widths | `src/backends/js/tests/runtime_helpers.rs` |

Every IO implementation phase closes with the programme's full phase gate plus console, input,
lifecycle and recovery coverage appropriate to that phase. Runtime behaviour uses runtime-output
assertions. Artifact assertions remain only where helper reachability is the contract.

## History

### V0 - console and global input polling

The package shipped five console helpers and the opaque `Input` handle with fifteen global keyboard
and pointer polling functions, JavaScript-only with pre-lowering HTML-Wasm rejection.

### Scope review

The dedicated scope and prelude review fixed `io` as the broad host-capability facade and the only
preluded Core package, set the overlap rule with focused packages, kept event work pull-driven and
moved scheduling, timers and callbacks behind async V1.

### Phase 1 - snapshot input hardening

The audit of the shipped helpers found live held state and pointer position mixed with snapshot
edges, an unbounded raw event array, logical keys that stuck when a modifier changed the produced
character, the unowned failure code 500 and an HTML-JS `print`/`line` distinction the reference
promised but the browser console cannot show. Phase 1 made `update` the sole publication boundary,
replaced the event array with coalesced transition sets, released keys by the identity recorded at
keydown, released held input on focus or visibility loss, made `close` permanently inert, defined
logical host-client pointer coordinates, switched the failure to `BuiltinErrorCode::Unsupported`
and redefined `print` and `line` by line termination with record-oriented hosts allowed one entry per
call. Executable fake-host scenarios replaced emitted-text assertions for input semantics.
