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
STATUS: activated; scope and prelude review accepted; current surface audited
CURRENT_SLICE: Phase 1 - snapshot input contract and host-buffer hardening (not started)
BLOCKERS: Phase 1 needs its listed contract decisions settled with the user before implementation;
the ordered event queue needs a final Moth event value that crosses the binding boundary; io.set_title
needs the config and HTML entry cutover; scheduling, timers and callbacks need structured async and a
host-ingress contract
NEXT_ACTION: settle the Phase 1 decisions, publish them in io.mtf, then implement and cover them
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
| Pointer | `input.pointer_down`, `input.pointer_pressed`, `input.pointer_released`, `input.pointer_x`, `input.pointer_y`, `input.last_pointer_pressed`, `input.last_pointer_released` | shared `Input` plus `String` button `-> Bool`, `Input -> Float` or `Input -> String?` |

`register_core_prelude` (`src/builder_surface/core_packages/prelude.rs`) registers bare `io` as a
compile-time namespace alias. `io.set_title` is accepted queued design with no registration,
lowering, capability plumbing or tests.

## Implementation notes

### Console lowering

`src/backends/js/package_bindings/core/io.rs` emits one shared `__moth_io_write` plus each
referenced console helper. `print` and `line` both call `console.log` with identical bodies, so
HTML-JS does not realise the documented trailing-newline distinction. `debug`, `warn` and `error`
fall back to `console.log` when the specific console method is missing.

### Input host state

`src/backends/js/package_bindings/core/io_input.js` is one helper blob, emitted whole when any input
helper is reachable and never pulled in by console-only use. `__moth_io_input_new` fails unless
`window`, `document`, `AbortController` and `window.PointerEvent` all exist. The handle holds held,
pressed and released sets for keys and buttons, the pointer position, four `last_*` values and a
`pending` array of edge records.

Host-owned listeners, all passive and all removed through one `AbortController` signal:

- `window` `keydown` records a press only when the key is not already held, so key repeat produces no
  new edge. `keyup` always records a release.
- `window` `pointerdown`, `pointerup` and `pointermove` update `clientX`/`clientY`. Down and up
  record button edges for buttons 0, 1 and 2 only. Other buttons update position but record nothing.
- `window` `pointercancel` releases held buttons. `window` `blur` and a hidden `document`
  `visibilitychange` release every held key and button, so input does not stick down across focus
  loss and the synthetic releases surface as ordinary released edges.
- `preventDefault` is never called. A unit test pins its absence.

`update` clears the edge sets and `last_*` values, drains `pending` in arrival order into them and
empties it. A press and release between two updates therefore reports both edges while `key_down`
is false, and each `last_*` value is the final edge of its kind. `close` aborts the listeners, marks
the handle closed and resets every field. A second `close` and any `update` after close are no-ops.
Reads on a closed handle return `false`, `0.0` or `none`.

Key normalisation maps `" "` to `"Space"` and lowercases single ASCII `A`-`Z`, both on ingest and on
every query. Every other key string passes through unchanged. Button queries are not normalised.

### Snapshot and live state are mixed

Edges and `last_*` values change only at `update`. Held state and pointer position are written
directly by the listeners, so `key_down`, `pointer_down`, `pointer_x` and `pointer_y` report the
latest host state whether or not `update` ran. Within one synchronous Moth invocation nothing
changes, because the browser cannot run listeners while Moth code runs. Across invocations without
an `update`, edge and held queries can disagree.

### Unbounded buffer

`pending` grows by one record per edge until the next `update`, with no bound or coalescing. A
program that keeps the handle open but stops calling `update` retains every edge the host delivers.

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

### Phase 1 - snapshot input contract and host-buffer hardening

Publish the existing snapshot model as a precise contract, then make the implementation match it.
Settle each decision with the user before implementation. The proposals follow the audit.

| Decision | Proposal |
|---|---|
| Observation point | `update` is the only point where observable input state changes. Held state and pointer position join the snapshot instead of reading live host state. |
| Edges between polls | A press and release between two updates reports both edges and not held. Each `last_*` value is the final edge of its kind since the previous update. |
| Buffer bound | Coalesce at ingest into per-key and per-button pending edge flags plus the four pending `last_*` values. Storage is then bounded by distinct keys and buttons rather than event count, and no raw event record is kept. An ordered queue, if later accepted, owns its own explicit bound. |
| Focus loss | Losing host focus or visibility releases every held key and button and reports those releases as ordinary edges. |
| Teardown | `close` stops observation immediately. A second `close` and `update` after close do nothing. Reads after close return `false`, `0.0` and `none`. |
| Pointer coordinates | Logical pixels from the top-left corner of the host surface's visible area. `0.0` until the first pointer event. HTML-JS keeps `clientX`/`clientY`. |
| Key names | Decide the portable normalisation rule. The reference promises lowercase for single alphabetic keys, but only ASCII is lowercased. Logical key strings also stick when the produced character changes between down and up, such as `1` pressed and `!` released after Shift changes. The fix may need the physical-key distinction from Phase 2. |
| Failure code | Replace the hand-built code 500 in `__moth_io_input_new` with a compiler-owned `BuiltinErrorCode` through the canonical error constructor, as Time did. |
| `print` and `line` | Decide how HTML-JS realises the documented distinction or narrow the contract. Both currently emit one `console.log` entry and the runtime harness joins entries with newlines. |

Coverage for this phase: the runtime harness has no `window`, so an integration case can execute the
`new` failure path and observe its code. Edge, focus-loss, teardown and coordinate semantics need a
driver that dispatches synthetic events, like the Node driver in
`src/backends/js/tests/runtime_helpers.rs`, unless the harness gains a minimal window host.

## Known gaps and next extensions

### Audit findings

| # | Finding | Evidence |
|---|---|---|
| 1 | `print` and `line` emit identical `console.log` bodies. | `src/backends/js/package_bindings/core/io.rs` |
| 2 | `io.input.new` fails with the unowned code 500. | `io_input.js`, pinned by `runtime_helpers.rs` |
| 3 | `pending` is unbounded and uncoalesced. | `io_input.js` |
| 4 | Held state and pointer position are live while edges are snapshot. | `io_input.js` |
| 5 | Only ASCII `A`-`Z` is lowercased, and logical keys can stick when the produced character changes. | `__moth_io_input_normalize_key` |
| 6 | The reference omits pointer coordinate space, initial position, focus-loss release, ignored extra buttons and post-close reads. | `io.mtf` against `io_input.js` |

### Canonical facts with no test owner

No integration case executes input semantics. Edge reporting, held versus edge state, `last_*`
ordering, focus-loss release, teardown, key normalisation, button mapping, coordinates and the `new`
failure path through Moth recovery are unowned. Integration cases assert emitted helper text only.
There is no `line` console case.

### Next extensions, in order

Candidates only. Nothing here is accepted API.

1. **Portable snapshot additions.** Choose from pointer movement deltas, wheel or scroll accumulation,
   modifier-key state, focus or active-state queries, improved pressed and released state, portable
   physical or logical key distinctions and further pointer information that maps cleanly across
   hosts. Each needs a portable contract and stays global and application-oriented.
2. **Ordered event value investigation.** Define the final Moth event value first, likely a choice
   with typed key and button payloads. Registered binding signatures currently carry scalars,
   `String`, `Char`, `Bool`, options and opaque handles, with no choice, record or collection
   results. If the value cannot cross cleanly, keep the ordered queue deferred and continue snapshot
   polling.
3. **Ordered pull queue.** Only after the value crosses cleanly: a bounded host queue drained by an
   explicit poll into ordinary Moth values, with stated overflow and coalescing rules.
4. **Queued `io.set_title`.** Its implementation is owned by the HTML page directives and runtime
   title work and needs the config and HTML entry cutover first. Its meaning is host-neutral and
   HTML-JS realises it as the live document title.
5. **Small synchronous host conveniences.** Review each individually rather than adding a broad
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
| Console helper emission per output lane | `tests/cases/core_io_console_print_success`, `_debug_success`, `_warn_success`, `_error_success` |
| `line` emission and `@core/io as` alias binding | `tests/cases/core_io_dependency_alias_success` |
| Bare prelude namespace, non-callable namespace, type-clause misuse | `tests/cases/core_io_namespace_binding_success`, `io_callable_rejected`, `core_io_type_dependency_rejected` |
| String-only console boundary and template coercion | `tests/cases/io_coerce_to_string`, `io_rejects_struct_value`, `io_rejects_collection_value`, `io_rejects_option_value`, `io_rejects_multiple_return_value`, `io_rejects_result_value` |
| Input helper emission, handle passing and optional reads | `tests/cases/core_io_input_new_success`, `core_io_input_update_close_success`, `core_io_input_reads_success`, `core_io_input_signature_success`, `core_io_input_last_key_optional_success` |
| Mutable-access, argument-type and type-as-value rejection | `tests/cases/core_io_input_update_missing_mutable_rejected`, `core_io_input_non_string_key_rejected`, `core_io_input_type_as_value_rejected` |
| Reachable HTML-Wasm rejection and unreachable acceptance | `tests/cases/core_io_input_wasm_rejected`, `core_io_unreachable_io_wrapper_wasm_ignored` |
| Helper reachability, no `preventDefault`, failure carrier code | `src/backends/js/tests/prelude.rs`, `src/backends/js/tests/runtime_helpers.rs` |

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
