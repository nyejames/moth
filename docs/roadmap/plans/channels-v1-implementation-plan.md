# Channels V1 implementation plan

- **Status** Queued, accepted V1 design
- **Current slice** None
- **Blockers** Delivered post-retirement syntax and constrained-closure foundations, final memory compatibility brief, production memory safety foundations and repeated statement option-capture loops
- **Next action** Verify the activation evidence below, then review the Phase 0 decisions

Deliver function-owned cooperative tasks and host-integrated Channels on HTML-JS.
The [Channels reference](../../docs/async/channels.mtf) owns source semantics.
The [compiler](../../compiler-design-overview.md), [build system](../../build-system-design.md)
and [memory documentation](../../developer-docs/memory-management/overview.mtf)
own the handoffs. V1 includes a tested portable Wasm handoff. Collector-free
physical memory implementation and full Wasm execution follow on the roadmap.

## Before activation

- [ ] Record the actual checkout and focused baseline in the working notes.
  Follow the repository reading routes. Verify profile-independent const
  classification, the single migrated grammar and retired Wiring contracts,
  dense HIR, ordered native result slots, concrete generic
  targets and stable external binding classifications on the activation tree.
  Exercise recursive messages only when their ordinary choice representation exists.
- [ ] The memory brief identifies the bounded owners of child arguments, queue
  entries, pending operations, returns and host registrations. Require delivered
  production last-use, provenance, retained-edge, outlives and local/link
  topology checks on ordinary programs, real builder roots and a validated
  HostGC handoff. Rerun the live production provenance controls, including
  writable retention through a helper, a store before failure and disjoint
  copied siblings. An oracle result or an advisory drop site cannot discharge
  this gate. General physical memory lowering stays with its roadmap owner.
- [ ] Verify ordered Stage 5 operand capture before reusing call lowering.
  In the production borrow tracker's reproducer, a later argument rebinds a
  holder from two items to three. An earlier independent copy observes two.
  Diagnose an overlapping live shared operand at its authored site and accept
  a disjoint shared control. Cover ordinary and fallible calls.
  Fix capture in its existing producer if still outstanding. Dispatch and
  selection consume those captures. Stage 6 cannot recover their original order.
- [ ] Verify delivery of general repeated statement option capture before
  activation. Test a side-effecting ordinary optional producer
  first. Its loop owner supplies reevaluation, a fresh binding and nearest-loop
  exits. Bracket collection construction is separate. Collection-producing statement loops remain an open design question.

## Phase 0. Close the remaining design decisions

Record each decision at its permanent owner with its distinguishing example.
These are review inputs for the first implementer. The conservative choices
below remain proposals until the language, memory and runtime owners accept them.

| Owner | Decision and acceptance evidence |
| --- | --- |
| Language and memory | Confirm non-negative compile-time `I32` capacity, including zero, omitted, negative and unrepresentable cases. Set exact endpoint type positions, whether local aliasing is allowed and whether supplying the same endpoint to two formals acquires one or two obligations or is rejected. Start with constructor locals and scoped parameters only. Write expected results for forwarding, early close, partial argument evaluation and attempted escape through results, aggregates, messages or retained callback captures. |
| Memory | Start dispatch arguments with scalars, independent graphs and proven transfers. Admit a scoped immutable loan only with a demonstrated use case and a production lifetime proof that covers joins, cancellation and retained children. Ordinary cross-task mutable loans and independent transfers from declared regions are excluded. Record allowed message types, including nominal wrappers for optional payloads and the rejection of top-level optional payloads. |
| Language and compiler | Set operator binding and placement in call arguments and runtime templates, postfix and catch attachment, receive discard, direct receiver calls, mutable arguments and the eligibility of a `Never` target. Consume the migrated `if {` / `case ...:` / `else:` readiness grammar. Pin preparation order using guard/operand side effects and a preparation error. Review the baseline of evaluating each guard then its enabled operands in authored order, with non-suspending preparation and ordinary statement arm bodies. |
| Runtime and host | Write one turn-by-turn trace for immediate success, a waiting operation, child failure during join, cancellation racing commit and two simultaneously ready arms. Specify which operations yield, fairness between continuously ready arms/tasks and how a yielding guest gives the host a turn. For timer, browser event and asynchronous request completion, fix public names, exact types, host capability, registration lifetime, bounded ingress and overflow behaviour. |
| Compiler, memory and backend | Compare explicit resumable frames with scheduler-driven JS generators on nested calls, deep recursion, synchronous cost, retained bytes and cancellation. Validate a portable suspend/return/resume mechanism with authored Wasm. Optional stack switching can be compared where available. Record the chosen continuation and versioned host handoff, supported target edges and measurements. |

Review these decisions before implementing their dependent slices. Promote the
chosen behaviour to the canonical references. Keep the evidence and its limits
distinct from a production or target-support claim.

## Phase 1. Parse once and lower to ordinary compiler owners

Paths below `src/compiler_frontend/` are relative to that directory.

- [ ] Reuse `CHANNEL_SEND` and `CHANNEL_RECEIVE` in `tokenizer/schema.rs`
  and their existing lexer productions. Dispatch prefix direct-call `>>`,
  infix send and prefix receive by grammatical position. Grouping and postfix
  operators use the expression parser. Preserve `value = << reader`. Reject
  declaration shorthand, invalid directions, indirect spawn and value-producing
  child targets at the authored span.
- [ ] Verify the syntax cutover retired the obsolete async-block path,
  deferred dispatch, diagnostic/render entries, `async_blocks` feature lane
  and reserved `yield`/task spellings, keeping independent checked-block
  coverage. `io.task.yield()` must resolve as an ordinary namespace call.
  Test ordinary identifiers and the rejection of retired task syntax.
- [ ] In `ast/statements/multi_bind.rs` the annotation belongs to the individual
  target and the RHS normally starts with `ExpectedType::Infer`. For the exact
  channel construction form, resolve `Channel of T` as contextual evidence
  before typing the initializer. Produce ordered reader and writer slots with
  exact carried types. Reuse the shared argument router in
  `ast/expressions/call_arguments.rs` for the named capacity. Preserve ordinary
  per-target annotations and builtin naming policies. Test wrong arity, omitted
  initializer, ordinary multibind and inferred generic endpoint parameters.
- [ ] Preserve the receiver and each written argument's evaluation position
  independently of its formal slot. `ast/expressions/call_validation.rs`
  normalises named arguments for the callee. Reuse captured operands in
  `hir/hir_expression/calls.rs` and the fallible path. An out-of-order named
  dispatch such as `>> worker(second = record(2), first = record(1))` records
  2 then 1. A failure in the latter argument publishes no child or new lease.
- [ ] Extend the delivered expressionless `if {` readiness statement from
  the syntax migration. Reuse the migrated line-initial `case` and colon
  arm-boundary owner, ordinary arm scopes and loop targets. Preserve the
  existing statement option-capture loop, including break, continue, early
  return and stream exhaustion. Reject mixed header forms, malformed guards,
  misplaced defaults and retired readiness spelling at authored spans.
- [ ] Use dense HIR operations, stable call targets and existing native
  success/error destinations in `hir/terminators.rs`. Make dispatch, wait,
  commit, release and invocation exit explicit. Suspension preserves the live
  operation. Only its winning edge defines results. Record preparation regions
  with source spans. Update successor traversal in `hir/utils.rs`, validation,
  remapping and generated sidecars together. Keep cancellation internal. Test
  required-const exclusions, inactive-source typing and reachable target
  rejection before executable support is enabled.

## Phase 2. Converge facts before verifying constraints

- [ ] Reuse the non-suspension proof already established for constrained
  closures. Extend `public_call_summary.rs` and the existing generated-function
  transaction in `module_compilation/generated/`. Preserve the ordering in
  `convergence.rs` and `constraint_convergence.rs`: establish escaping builtin
  failure, install private failure lanes, then compute effects on final HIR.
  Keep ordinary calls and dispatch edges distinct. A child with a synchronous
  body can still make its owner's implicit join suspend. Summarise cleanup and
  host waits. A selection with an immediate default has no communication wait
  of its own. Its winning body can still suspend.
- [ ] Verify optional `$no_suspend` on functions and methods and the implicit
  requirement on every selection guard and enabled operand. Reuse
  `hir/constraint_facts/analysis.rs` (`analyze_statement_region`) and
  `hir/constraint_verification.rs` for a bounded causal witness. Arm bodies
  follow ordinary call rules. Provisional facts and unknown foreign effects
  cannot prove absence. Test recursive, imported, re-exported and generated
  targets, including a provider whose only interface change is suspension.
  A suspending call in a proven dead handler adds no suspension or scheduler
  demand. Change only the imported callee's failure summary to make the handler
  reachable and verify that both results change.
  Keep witnesses/local HIR IDs out of public identity. Preserve existing
  `$infallible`, numeric constraints and profile-independent verdicts.
  A closure invocation must still be proven non-suspending through its
  helpers and implicit joins. Test a provider becoming suspendable through
  Channels and reject it at the existing callback constraint boundary.
  Use the canonical callback retention and access checks. Do not treat
  captured environments as automatically transferable payloads.
- [ ] Extend the failed-send operation through `builtins/error_codes.rs` and
  `hir/private_failure_lane.rs`. Deliver an escaping child's original builtin
  Error at the owning invocation's completion, including an arbitrary declared
  code. Classify the join as the implicit failure origin. Keep other host calls
  on their declared failure paths. Test a caught owner call, exported boundary,
  explicit custom-error mapping and a child failure during a staged return.

## Phase 3. Prove transfer and produce cleanup facts

- [ ] Extend the production `analysis/borrow_checker/` and its memory-foundation
  facts. A committed send transfers a whole independent allocation graph's
  cleanup responsibility. The lifetime owner remains fixed. Shared children
  inside the graph retain their identity. A surviving outside alias or a
  declared-region escape prevents transfer. A projection cannot detach part
  of a family. Transfer of the whole family needs a proven final use and
  a legal lifetime owner.
  `FunctionReturnAliasSummary::Fresh` only excludes parameter roots. Reuse
  ordinary `copy`. Use `copy_preserves_shared_children_runtime` as the
  synchronous control. Send a copy with a repeated child in the compiled
  channel case and verify independence from the original. Preserve valid
  task-local borrows across suspension when no conflicting observer exists.
- [ ] Extend `PublicCallParameterSummary` and its transition validator with
  mandatory send commitment per parameter **on each reachable callable exit**.
  Existing `MayConsume` models optional transfer. A caller regains access
  only when every path to its exit is uncommitted. Test a direct precommit
  failure, a helper that sends then fails, a helper that catches a failed send
  and returns success, a conditional send and a later child failure at join.
  Carry these facts through forwarded aliases and imported/generic wrappers.
  Widen mixed histories conservatively. Extend the existing summary and
  publication/invalidation paths once.
- [ ] Track scoped endpoint obligations separately from payload aliases.
  `.close()` consumes the *current* receiver obligation. An ordinary helper
  closes only its own parameter lease. Lower explicit close and automatic
  scope exit to the same release effect exactly once. Distinguish use after
  close from a live reader draining an exhausted channel. Protect earlier
  staged arguments/selection candidates against conflicting later mutations
  or closes. Prove bounded parent, child, queue and host lifetimes on every
  exit, including optional early return, cancellation and a pending join.
- [ ] Publish per-operation liveness and release facts for locals, pending
  operands, queued messages, selected-arm temporaries, staged returns and
  registrations. Use the memory planner's validated JS handoff. GC timing,
  non-lexical last use and advisory drop sites cannot move observable close.
  Production Stage 6 consumes validated HIR and writes side tables.
- [ ] Extend `analysis/borrow_checker/problem/` and Boracle's reference,
  operational, differential and reduction paths. Separate static sites from
  dynamic task, channel and wait occurrences. Run the same source through
  production and the normalized problem. Record the production verdict,
  reference verdict and bounded execution or counterexample separately.
  The differential runner compares Boracle implementations. Retain known
  disagreements, unsupported cases and exhausted bounds as such. Require
  evidence for both accepted and rejected cases before runtime activation.

## Phase 4. Run structured tasks through a single completion protocol

- [ ] Implement the Phase 0 continuation with a resume point, live locals,
  existing result/error slots and planned cleanup. Dispatch creates a child
  only at commit. Ordinary suspendable calls stay on the current task.
  Proven synchronous bodies retain direct calls. Register `io.task.yield()`
  and `io.task.cancel_children()` via `src/builder_surface/core_packages/io.rs`
  and the existing binding/capability path.
- [ ] On normal exit stop admitting children to that invocation, stage results,
  release its retiring endpoint obligations and join. Keep storage observed
  by children live. On escaping failure latch the first observed Error, cancel
  siblings/descendants as appropriate and join before delivery. Wake a parent
  blocked in a nested call and unwind to the owner. Explicit cancellation
  withdraws waits and bypasses ordinary catches. Traps use existing fault
  handling. Reclaim completed child records while the parent remains live.
- [ ] Drive deterministic traces for recursion, multiple staged results,
  child-first and parent-first failure, cancellation during join and repeated
  child creation. A parent that falls through holding the sole writer releases
  it before joining a child draining the stream. A parent draining an open
  stream inside its body must explicitly release its own writer or arrange
  another end condition.

## Phase 5. Deliver one queue and commit operation

- [ ] Implement one channel state with directional lease counts, FIFO buffered
  messages and pending offers/receives. Reuse the same readiness and commit
  operations for direct communication and Phase 6. Rendezvous needs no buffer
  slot. Capacity bounds accepted entries. Waiting senders retain their
  uncommitted payloads. Size waiter storage by live registrations and release
  it on withdrawal. Guard count overflow and repeated completion.
- [ ] Test zero/full capacity, one delivery with multiple producers/consumers,
  explicit release plus scope exit, last-writer drain followed by repeated
  absence and last-reader teardown. Teardown disposes queued values and
  fails blocked sends with their payloads intact. After a successful handoff,
  cancellation cannot reverse ownership. Run the compiled producer/consumer
  through a forwarded endpoint and a general repeated-option loop. Verify
  messages, absence and failure codes in debug and release.

## Phase 6. Select atomically and diagnose global deadlock

- [ ] Stage guards and enabled operands once under the approved order. A
  preparation error disposes fresh temporaries and prevents registration.
  Earlier user effects stand. Use one wait identity for all enabled arms.
  Atomically commit one compatible arm (both parties for rendezvous), remove
  losing offers and wake each participating continuation once. Never match
  two arms of the same selection. The chosen failed send takes its error
  edge. The success body is unreachable on that outcome.
- [ ] Test duplicate endpoints, the same graph offered to two writers, a fresh
  losing temporary, two selections meeting, default, EOF readiness and
  competing cancel/commit. Only the winner transfers. A late wake cannot
  recommit. Exercise fairness with several continuously ready arms and
  document the approved rotation or equivalent deterministic test policy.
  Compare a contested selection in the scheduler model and a compiled Moth
  program using the same event sequence and allowed winners.
- [ ] An all-false, no-default selection has no communication wakeup. Keep it
  cancellable. Diagnose global deadlock only with no runnable task, deliverable
  completion or live host source capable of waking the instance. Test a parent
  cancelling the disabled child, the same wait in a truly quiescent instance,
  a full queue, a circular join/receive and a stale host token. Report bounded
  wait, channel, parent and source identities through the instance fault lane.
  Elapsed time is solely a harness fail-safe.

## Phase 7. Integrate HTML entries, browser IO and the test harness

- [ ] `src/projects/html_project/js_path.rs` lowers linked modules separately
  into isolated JS scopes. Aggregate validated demand across the entry,
  imported functions and reachable generated sidecars. Create **one scheduler
  per execution instance** outside those module wrappers. Keep the per-module
  helpers in `src/backends/js/emitter.rs` stateless with respect to that
  instance. Test an imported child waking its parent, two independent page
  instances and a synchronous-only bundle with no scheduler or async helpers.
- [ ] Keep exactly-once entry activation and publish staged fragments only
  when the invocation and descendants succeed. On failure retain static HTML,
  report once and preserve explicit IO already performed. Drive timer, browser
  event and real asynchronous completion through scoped registrations with
  instance and wait generations. Copy/validate bounded host-owned data before
  scheduling, observe the agreed count/byte and overflow policy and release
  subscriptions on cancellation or teardown. A callback enqueues a wakeup.
  It cannot synchronously wait for guest capacity or re-enter running guest code.
  Forced page destruction invalidates the instance and host resources without
  promising guest cleanup. Reject reachable unclassified async bindings at
  target validation. Ordinary synchronous foreign bindings keep their contracts.
- [ ] Extend classic and ES-module execution in
  `src/compiler_tests/integration_test_runner/assertions/rendered_output.rs`.
  Both currently finish after one `setImmediate`. Observe entry settlement or
  a typed fault before serialising output. Reuse the bounded Node subprocess
  and the existing Wasm completion-observation pattern. Self-test delayed
  success, late failure, expected deadlock and an actual harness timeout.
- [ ] Use deterministic fake-host traces for race permutations and a focused
  real-browser test for a timer while another task repeatedly yields, a DOM
  event waking a receiver, request success/failure against a local endpoint,
  ingress bursts and teardown followed by late completion. Do not infer
  browser scheduling from the existing fake input event target. Keep snapshot
  input behaviour and unrelated page scripts intact.

## Phase 8. Verify the portable Wasm handoff

- [ ] Specify a versioned suspend/resume boundary with result/error destinations,
  frame-layout inputs, validated live storage, commit order, instance generation,
  wait identity and cancellation. Build supplies lifecycle roots, target
  assignment and capabilities. Compiler validation checks reachable cross-target
  edges before physical planning. Gate unsupported mixed JS/Wasm calls with a
  target diagnostic. Physical frame data stays outside public semantic identity.
- [ ] In `crates/lappet/src/runtime/instance.rs`, fallible exports reserve
  statuses 0 and 1 for success and declared error. Host-produced staged text must
  be consumed within its call. Exercise the new boundary with small authored
  core Wasm fixtures and the pinned runtime. Return control to the host with
  no live staging ticket, borrowed guest span, resource guard or call context.
  Resume using independent data. Reject stale or cross-instance tokens and
  cancel without guest re-entry. Repeat suspend/resume/cancel and verify bounded
  frame storage that returns to baseline on teardown. Preserve fault quarantine and the
  synchronous ABI.
- [ ] Hand the validated frame, queue, pending payload, lease and registration
  requirements to remaining physical memory work. Exercise close, cancel,
  join and teardown in collector-free verification. Keep REC non-atomic.
  Full Wasm execution needs parity on the same Moth scenarios after its
  backend lands. An authored fixture establishes the mechanism only.

## Phase 9. Verification and slice review

Use one primary test owner per contract. Keep unit tests for grammar, typed
IR edges, invalid states and bounded scheduler traces. Put observable programs
under `tests/cases/` and register them in `tests/cases/manifest.toml`. Give
Phase 5 one compiled `channels_worker_pool` case for forwarding, EOF and join.
Keep compile-time rejection and host-entry faults in cases that can observe
those outcomes. Extend existing fixtures first. Use the same source for backend
and profile comparisons. Run the focused check with the owning slice.

| Checkpoint | Command and required evidence |
| --- | --- |
| Surface and summaries | `cargo test -p moth-lexical`, `cargo test -p moth --lib constraint_summary` and `moth tests --case core_io_namespace_binding_success --terse` as an existing namespace control. Check source spans, selected edges, bounded witnesses and changed versus unchanged interface reuse. |
| Memory safety | Run production source cases alongside `just boracle`. Run `just boracle-campaign` for changed generated differential coverage. Record verdicts, bounds and reduced failures. Run `just validate` before enabling runtime execution. |
| Runtime and host | Once registered, run `moth tests --case channels_worker_pool --terse`, harness self-tests, the real-browser lane and `cargo test -p lappet --features runtime` for the native fixtures. State browser, target, profile and exact commands. Verify synchronous runtime erasure in the artefact. |
| Performance | Compare like-for-like workloads for ordinary call/compile cost, output size, summary convergence, live frame bytes, allocations, queue/waiter peaks, transfer throughput, selection withdrawal, host wake latency and cancellation. Include worker pools and event bursts. Use non-recording measurements and keep raw data local. |
| Documentation and integration | Run `moth check docs --terse` and `moth build docs --release`, inspect changed pages and run `just validate` at the integration boundary. Run `just validate-full` on the final integrated tree before landing on main. |

- [ ] Perform the repository Slice review at each coherent handoff. Before
  closeout, obtain independent correctness and redundancy reviews. Verify
  all accepted HTML-JS behaviour, mandatory production proof and the Wasm
  mechanism evidence. Remove displaced parsers, summary passes, unused
  runtime demand and redundant tests where the new owner covers the contract.
- [ ] Update the progress matrix only for delivered language/target support.
  Move durable decisions to the permanent owners, reconcile the remaining
  memory and Wasm work with the measured implementation and remove this plan
  and its roadmap entry in the completion commit.
