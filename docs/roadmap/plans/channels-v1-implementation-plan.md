# Channels V1 implementation plan

> For agentic workers: implement in reviewed slices using the repository's
> AGENTS.md reading routes and validation policy. Use the superpowers
> subagent-driven-development or executing-plans workflow where available.

STATUS: queued, accepted V1 direction
CURRENT SLICE: none
BLOCKERS: final memory design closure and delivered memory safety foundations,
including the production borrow checker and complete supported-runtime topology
proof. Repeated statement option capture must be available before stream-loop
acceptance.
NEXT ACTION: refresh the activation baseline and close the design/evidence gate.

## Goal and authority

Deliver one host-integrated cooperative Channels model on HTML-JS, with
backend-neutral analysis, a concrete tested Wasm/runtime handoff and no
dependency on the full collector-free allocator implementation. The later
full Wasm backend must implement the same observable contracts, not an
alternative async language. The canonical source authority is
`docs/docs/async/channels.mtf`. Compiler, build, memory and native host
authorities own their respective handoffs. This plan is a work item, not
another semantic reference. Promote any approved checkpoint decisions to the
permanent owners before implementing them.

The roadmap orders final memory design closure, production memory safety
foundations, Channels, remaining memory-management work and full mixed Wasm.
Boracle research can begin from the accepted Channels semantics before any
Channels code lands. Do not activate runtime concurrency until the foundation
gate passes. Do not begin general collector-free lowering in this plan.

## Global constraints

- One source spelling per operation: prefix direct-call `>>`, infix send
  `>>`, prefix receive `<<`, statement `if:` selection and ordinary calls.
  No async annotations, future objects, alternate channel methods, detached
  tasks, scheduler objects, new prelude package or general consuming-call API.
- Compiler-owned directional channel contract and scoped endpoint obligations.
  Ordinary payload semantics remain reference-semantic, copy-explicit and
  move-inferred except mandatory channel transfer on successful commitment.
- Normal invocation completion joins; failure cancels and joins. No successful
  result publication before completion; cancellation and trap are distinct
  from recoverable failure and normal closure.
- All runtime borrow and lifetime legality is mandatory across profiles and
  targets. Required const evaluation stays separate and runtime operations
  are ineligible. Ordinary functions remain runtime functions in every profile.
- Reuse canonical exact types, result slots, stable call identities, generated
  convergence, source constraints, effect and lifetime summaries, dense
  stores and diagnostics. No donor-local IDs in public summaries.
- The compiler owns semantic effects and cleanup facts; build supplies roots,
  partition and capabilities; the runtime realises a validated execution
  contract. The backend cannot infer legality or change logical close timing.
- Keep the native host adapter and browser runtime isolated from compiler
  internals. No Wasmtime dependency in the compiler, no general foreign
  callback/re-entry ABI, no arbitrary host retention of guest references.
- Keep synchronous code cheap and async runtime reachability-driven. Do not
  add universal frame allocation, atomic REC, GC fallback on a capable
  full-control target, universal promises or per-call task creation.

## Prerequisite and activation evidence

- [ ] Read AGENTS.md, index.md, current progress, style/testing/validation,
  compiler and build architecture, the async, function, branching, loop,
  option/error, Wiring, constraint, IO and external-binding authorities, all
  routed memory authorities and Lappet runtime/host ABI contracts.
- [ ] Establish the actual activation HEAD and inventory parser and lexical
  reservations, generated summary convergence, HIR/CFG, borrow problem
  extraction, production checker, reference and operational Boracle solvers,
  JS lowerer, runtime helper demand and Wasm LIR/host boundaries. Update
  locations to the delivered code. Do not pin a speculative baseline.
- [ ] Verify completed profile-independent const/runtime semantics, dense
  HIR, ordered success/error slots, static generic targets, required concrete
  aggregate/message type support and explicit stable external binding
  classification. Recursive choice support is required where recursive
  message graphs are advertised; do not fake a carrier for unsupported types.
- [ ] Verify the accepted memory compatibility brief and delivered production
  last-use/alias, provenance, retention, outlives and mandatory local/link
  topology substrate on ordinary runtime programs. Validate real builder
  roots, transitive graph independence, success/error transfer facts, loan
  liveness and logical cleanup requirements. This is production architecture
  shared with the later memory work, not a temporary Channels checker.
- [ ] Verify the conservative validated HostGC memory-plan handoff for the
  initial JS target. Host GC implements physical storage only. Logical
  endpoint/registration release must already be representable independent
  of GC. No async execution if any needed proof is missing.
- [ ] Deliver the general repeated statement option-capture slice using its
  ordinary loop contract and CFG; coordinate its existing roadmap owner so
  collection-producing loops remain later and nothing implements it twice.
- [ ] Run the current focused production/Boracle differential and source
  integration baselines and record commands, results and relevant measured
  costs. Any unresolved safety gap blocks activation, rather than being
  accepted because JS happens to keep storage alive.

## Phase 0: Close semantic and architecture checkpoints

Before code changes, produce short durable decisions at the canonical owners,
with executable/reference traces where a choice affects semantics. Preserve
the accepted public direction. The following implementation-level details
need explicit review rather than guessing at lowering time.

- [ ] Finalize constructor capacity validation, canonical endpoint identities
  and exact allowed positions, generic evidence, endpoint aliasing, scoped
  parameter lease creation and release. Confirm the conservative baseline:
  local/parameter-only capabilities, independent per-invocation obligations,
  explicit close and lexical release, no arbitrary capability storage or
  endpoint-containing message. Establish whether local rebinding aliases
  one obligation or is rejected; never make alias count equal lease count.
  Verify producer/consumer, multiple producers, multiple consumers and helper
  forwarding. Test same endpoint supplied twice, parent closes its own
  endpoint after spawn, child closes early, error during argument preparation,
  last-reader teardown, no resurrection after final close.
- [ ] Fix ordinary task argument contract: fresh/independent graphs or
  proven transfers with no surviving conflicting aliases; reject unproved
  cross-task sharing, Wire/Route identity retention, ordinary mutable
  cross-task loans and declared-region escapes. Evaluate a bounded scoped
  immutable-loan rule against real use cases and the canonical single-owner
  topology. Only permit it if production and Boracle can prove it. Specify
  how sends of ordinary parameters induce caller-side mandatory outcome-
  sensitive transfer requirements without general move syntax. A wrapper
  cannot launder an aliased graph; no optional transfer used as proof.
- [ ] Fix message type domain. Support ordinary transferable scalars, strings
  and concrete legal aggregate/choice graphs, no unproven opaque resources,
  capabilities or independent declared-region members. Keep outer receive
  absence distinct from optional payload absence; reject the unsupported
  top-level optional payload rather than inventing nested options.
- [ ] Define send/receive precedence and placement (including templates,
  nested argument expressions and postfix handling), direct spawn call shape
  including receiver/mutable argument restrictions, exact branch binding
  syntax, guards and default placement. Fix selection preparation staging,
  all-disabled behavior, selected-send failure routing and cancellation races.
  The baseline is statement-only selection, one final else, one operation
  per arm, non-suspending guard and operand preparation, closed-reader
  readiness with none, recoverable selected-send failure. Report all-disabled
  no-default selection as proven deadlock, not ordinary absence.
- [ ] Fix task dispatch commit and scheduler turn order. Use explicit ready,
  running, waiting, joining, cancelling and terminal states; no recursive
  guest re-entry. Specify fair ready-task and ready-arm arbitration, FIFO
  buffered messages, fairness assumptions and host polling opportunities.
  A cooperative long-running function may starve the host; no implicit
  preemption promise. Define scheduler quiescence and reliable host-wakeup
  accounting. Decide when zero-wait operations do not yield.
- [ ] Define normal/error/cancel exit ordering for each dynamic invocation:
  stop admitting new children, stage result, retire unused endpoint
  obligations, preserve child-live storage, join/cancel descendants, dispose
  remaining registrations and locals, deliver one outcome. Cover returns
  inside loops and branches, postfix optional early return and implicit
  failure. Specify parent error vs child failure observation, cancellation
  during join, multiple failures, and error delivered at the owner boundary
  rather than an arbitrary interrupted call. Cancellation bypasses ordinary
  catches as a control outcome, not a new source result type. Traps invalidate
  the instance under existing fault policy.
- [ ] Fix suspension and failure summary dependencies before HIR publication.
  Include spawn edges distinctly from ordinary calls, join, cancellation,
  select and host wait. Verify recursive SCCs, private inferred failure,
  exported and custom error boundaries, generic instantiation, selected
  inactive code, const eligibility and existing constraint verification.
  Optional `$no_suspend` is a proof of absence, not permission to suspend.
  Unknown provider evidence cannot prove absence. Include implicitly waiting
  function exit and error cleanup in this effect.
- [ ] Fix host execution/ingress contract for timers and real browser event
  delivery plus at least one asynchronous completion (for example fetch).
  Identify lifecycle owner, bounded queue overflow/coalescing/error policy,
  registration retention, host cancellation, stale generation rejection,
  timer duration exact type, and API placement under existing io or focused
  builder packages. A browser callback cannot synchronously wait for queue
  space. Host-owned pending data must be bounded and independent. Do not
  expose a raw promise, continuation, callback or arbitrary route invocation.
- [ ] Compare compact explicit continuation frames/state machine, JS
  generators driven by a common scheduler and any supported Wasm stack
  switching path. Use small authored JS/Wasm and host fixtures to measure
  resumable call chains, deep recursion, ordinary call overhead, suspended
  local storage, task/queue memory and cancellation. Baseline must not
  require optional Wasm stack-switching proposals or force a universal
  dispatcher for synchronous Wasm. Fix the minimum portable continuation
  handoff and target eligibility before selecting representation. Record
  why alternatives were rejected. Prototype mechanics is not production
  proof or a target support claim.
- [ ] Obtain explicit architecture review of these decisions, including
  memory/borrow and host/Wasm owners. Update permanent authorities and
  remove superseded rules before proceeding.

## Phase 1: Compiler surface and neutral operations

Owners: moth-lexical and frontend lexical mapping; retained preparation;
AST expression, statement, call and type owners; Stage 5 dense HIR and
validation; canonical language and diagnostics owners.

- [ ] Add tests for constructor contextual two-result typing, named capacity,
  zero and positive bounds, wrong arity/type/direction, omitted initializer,
  endpoint misuse and nested payload restrictions. Parse/type check via
  existing native result and receiving-context owners. Do not add tuple
  carriers or bidirectional channel values.
- [ ] Add positive/negative tests for prefix direct-call dispatch, infix
  send, unary receive with grouping, call/return arguments, explicit
  declarations/updates, rejected legacy sugar and standalone receive.
  Validate exact source locations, shadowing and ordinary argument routing.
  Preserve receiver/callee/argument order and once-only evaluation.
- [ ] Lower direct child dispatch and channel construction to validated
  semantic operations with resolved targets and explicit native result
  slots. A child target has no ordinary success values, only zero-success
  or builtin Error! contract. Reject custom unadapted errors and invalid
  first-class targets. No source async function type.
- [ ] Represent send, receive, selection, wait, join and logical endpoint
  release with explicit CFG outcomes and stable operation identities;
  no nested effectful expression left in a backend value. Distinguish
  suspended operation from normal success/error continuation and internal
  cancellation. Receive success has one optional slot; failed send does not
  define successful destinations. Reuse Invoke-style failure edges and
  ordinary option/loop/branch destinations rather than a semantic Result.
- [ ] Define selection arms and guards in the shared branch/call owners.
  Use explicit arm-scope identities and selected-edge destinations.
  Reject mixed Bool/match/select forms, duplicate/default placement and
  malformed guards. A guard false path contributes no registered operation.
  Require arm CFG to preserve break/continue/return and nearest loop rules.
- [ ] Add source validation for required const functions, constants,
  const templates, config and other required-const contexts; no new
  const effect interpreter. Check the actual delivered const classifier.
- [ ] Retain inactive-source typing and evidence checks but remove executable
  async demand only at the existing mandatory semantic-selection boundary.
  Add remap, generated sidecar, HIR verifier and diagnostics tests.
- [ ] Until the runtime and safety gates are met, reject reachable
  unsupported execution with structured target diagnostics. Check-only
  success must not imply a target can execute an incomplete feature.
- [ ] Run targeted parser, AST/HIR and integration cases; run the slice
  review and relevant docs gate.

## Phase 2: Suspension, failure and constraint convergence

Owners: existing callable summaries, external provider registry,
module_compilation/generated convergence, hir/constraint_facts and
constraint_verification, canonical call/interface/link fact owners.

- [ ] Test direct, transitive, recursive, cross-module, re-exported and
  generic calls, including a non-suspending child whose parent may join.
  Add a stable inferred suspension fact with unknown/provisional state
  distinct from proven absence. Use one monotone finite convergence,
  independent of profile/target. Keep local suspension sites keyed to HIR.
- [ ] Extend existing implicit-failure analysis for sends and escaping
  child failures. Retain first-failure primary cause with bounded source
  witness. Spawn argument failures remain in the caller's ordinary expression
  lane; child error belongs to owning invocation completion. Preserve
  explicit Error! handling, closed export and custom-error requirements,
  ordinary catch eligibility and native success/error destination rules.
- [ ] Install private failure lanes, converge summaries, prune only
  unreachable handlers and refresh dependent borrow/lifetime/link facts
  before immutable publication. No hidden third error slot.
- [ ] Add `$no_suspend` through existing compiler-owned directive attachment
  and constraint verifier. Verify function/method/generic contracts and
  transitive calls, implicit join and unknown host evidence. Reject any
  violation with bounded causal call/operation witness. Ordinary functions
  need no positive annotation; no user-extensible effect row.
- [ ] Ensure `$infallible` checks escaping channel/child errors and semantic
  traps under its existing guarantee, separately from suspension and
  external cancellation. No weakening of fast/safe numeric rules.
- [ ] Publish complete canonical summary effects and fingerprint changes
  through the existing public interface and generated-sidecar owners.
  Reuse stable call identity and active link facts for child target reachability
  and host requirements. An unchanged public summary must not cause
  needless semantic recompilation.
- [ ] Test source acceptance and validated HIR equivalence in debug and
  release, including a pruned handler and a previously synchronous provider
  gaining suspension. Run focused constraint/failure/generic cases and the
  module publication/architecture checks.

## Phase 3: Borrow, transfer, lifetimes and Boracle

Owners: production Stage 6, normalized Boracle problem extraction,
reference and operational oracle, lifetime/retention analysis and link
topology. Consume the delivered common memory foundations.

- [ ] Extend the normalized problem with task dispatch, suspend/resume,
  channel commit, loser withdrawal, close, join and cancellation events.
  Model dynamically distinct invocation/endpoint occurrences and wait
  records rather than conflating static HIR IDs with runtime objects.
  Reuse existing places, overlap, alias, future-use and outcome facts.
- [ ] Extend reference and operational oracle traces for task/endpoint
  events and bounded scheduler choices. Exercise exact success, error,
  cancellation and alternative-arm effects. Keep inconclusive bounded
  exploration distinct from proof and retain reduced counterexamples.
  Production uses conservative static facts, not runtime search or
  an interpreter check in the emitted program.
- [ ] Prove mandatory graph transfer on committed sends: no surviving
  source/retained alias, no unsupported family projection detachment,
  no lateral region promotion, no direct cycle or declared-region escape.
  Failure and losing branches retain responsibility and source access.
  Preserve scalar value semantics, fresh/copy handling and interprocedural
  send-of-parameter consumption facts.
- [ ] Prove child argument legality, bounded task and queue owners, suspension
  live storage, endpoint obligation aliases/retention and deterministic
  release legality. Ensure a parent reaching return cannot free a child-live
  allocation; an exclusive access cannot be used by another task while
  suspended. Do not reject a task-local borrow solely because it is live
  across suspension when no conflicting observer is possible.
- [ ] Publish validated suspension liveness and cleanup requirements with
  one owner: task-local values, argument staging, pending send values,
  select candidates, staged returns, retained resources, exit outcomes.
  Logical endpoint and registration releases are semantic events separate
  from physical reclamation. Feed the ordinary compiler-owned memory
  planning boundary; JS may select HostGC for payload storage only after
  mandatory proof.
- [ ] Test aliased fields, projection sends, later use after success,
  failed-send reuse, path joins, loop re-entry, copied values, recursive
  choice graphs where supported, task-local mutation, incompatible access,
  invalid escapes, endpoint close/alias misuse, cancellation of suspended
  nested calls, declared-region restrictions and inter-module summaries.
- [ ] Compare production/reference/operational results for bounded adversarial
  fixtures. Review topology and mandatory transfer separately from optional
  drop precision. No GC, dynamic uniqueness scan, parallel checker or
  advisory drop sites as legality fallback. Run targeted and `just validate`
  at this hard pre-runtime integration gate.

## Phase 4: Scheduler, task frames and completion

Owners: backend-neutral execution handoff and selected backend runtime.
Use the Phase 0 evidence; one scheduler protocol with compact task IDs,
wait IDs and explicit state, no public task object API.

- [ ] Write deterministic scheduler/model tests for runnable ordering,
  nested ordinary calls, recursion, child dispatch, yield, implicit join
  and terminal outcomes, including a child that completes before parent
  exit and a parent that stages a multiple-result return.
- [ ] Implement the selected continuation representation using only locals
  live across a suspend edge, explicit resume point, native result/error
  destinations and planned cleanup facts. Synchronous calls use their
  existing path. Do not turn every function invocation into a task or
  allocate a heap frame for a proven non-suspending body.
- [ ] Register child ownership at dispatch commit, not before evaluating
  fallible arguments. Join all descendants transitively through their
  owners. Release parent endpoint obligations at the validated exit
  boundary before joining when legal; preserve child-live storage and
  staged result obligations. Never publish success before join.
- [ ] Deliver child/parent recoverable failure and explicit cancellation
  through one cleanup protocol. Remove waits exactly once, resume or
  unwind active continuation frames, cancel descendants and deliver primary
  error only after cleanup. Fault/host termination uses existing quarantine,
  not catch or a retry. A late completion token must be harmless.
- [ ] Add `io.task.yield()` and `io.task.cancel_children()` through the
  existing Core IO registration/capability path. No second prelude or
  scheduler customization surface.
- [ ] Add stress traces for nested scopes, thousands of completed children
  with bounded retained bookkeeping, cancellation during join, task-local
  state and failure while returning. Verify cleanup and scheduler identity
  reuse, no stale wakeup and no synchronous-path overhead regressions.

## Phase 5: Channel queue and endpoint runtime

- [ ] Test rendezvous, FIFO buffered messages, bounded capacity, empty/full
  wait, one delivery per message, multiple senders/receivers, endpoint
  forwarding, explicit close, last-sender drain and last-reader failure.
- [ ] Implement one channel state owner with bounded queue and waiter
  storage. Use compact IDs and efficient ready/wait queues. One commit
  changes pending payload responsibility and channel state. A direct
  handoff need not allocate a queue node. Avoid polling or wake-all storms.
- [ ] Track only logical endpoint obligations, not every ordinary alias.
  Release each lease exactly once through compiler-planned release sites.
  Last writer wakes receivers after buffered data is drained; last reader
  releases undeliverable payloads and fails pending sends without consuming
  their values. No backend-GC finalizer defines closure.
- [ ] Preserve order and ownership on failure, cancellation before/after
  commit and receiver teardown. A host or scheduler wakeup cannot recommit
  a completed operation. Track bounded reusable waiter records and channel
  storage; counters must not wrap or underflow.
- [ ] Integrate compiled producer/consumer, multiple producer worker-pool,
  copy-and-send and repeated-option stream cases. Verify exact messages
  and error codes, not emitted JavaScript snippets. Include output/error
  propagation through ordinary imported and generic wrappers.
- [ ] Run scheduler and channel state-model tests plus targeted source
  integration tests in debug and release.

## Phase 6: Atomic selection and liveness diagnostics

- [ ] Add tests for send/receive competition, simultaneous readiness,
  duplicate endpoint arms, false guards, default, all-disabled selection,
  closed readers, failed selected send, same value offered to two writers,
  fresh losing values and cancellation before/after arbitration.
- [ ] Register one selection identity with all eligible channel waits.
  Atomic arbitration commits at most one arm, unregisters losers and
  wakes exactly one continuation. Resolve rendezvous between selections
  without self-matching or double commit. No temporary child per arm.
- [ ] Use deterministic test scheduling and fair arbitration under
  repeated readiness; record fairness policy and assumptions. Do not
  claim probabilistic fairness or bounded progress outside those rules.
- [ ] Track bounded waiting reasons (send/receive/select/join/host), source
  identity and parent relationships. Diagnose proven global quiescent
  deadlock only when no runnable or deliverable internal work and no
  registered host completion can wake the instance. A live host wait is
  not proof of deadlock. Never use a fixed elapsed-time heuristic.
- [ ] Test mutual receive, blocked send on full queue, parent join awaiting
  its own unsent shutdown, endpoint release resolving a wait, host wait
  preventing false deadlock, and stale cancelled host wait not preventing
  a true deadlock. Keep partial-deadlock and protocol analysis deferred.
- [ ] Define fault classification and dev/host reporting via existing
  instance-owned runtime diagnostic path. Bound reports and preserve
  spawn/wait sites without retaining completed task graphs. Validate
  scheduler/selection model against compiled traces.

## Phase 7: Real host integration and HTML-JS

Owners: Core IO, focused builder bindings, JS runtime/import demand,
entry assembly and dev reporting, host ABI/adapter authorities.

- [ ] Implement the approved timer and event/completion APIs and scoped
  registrations from Phase 0. Host callbacks enqueue independently
  validated data or completion tokens and arrange a scheduler wakeup;
  never re-enter an active Moth stack or expose a raw callback. Validate
  exact result/error types before publishing values. A host event burst
  observes its documented bounded ingress policy, including full queue.
- [ ] Drive resumable entry execution through the builder-owned page
  lifecycle. Preserve exactly-once activation, existing staged runtime
  fragment publication only at successful entry completion, static HTML
  on error and explicit IO already performed. A long-lived entry waiting
  on events does not publish staged fragments early. Explicit live DOM
  operations need their own approved builder API, not implicit template
  reactivity. Unrelated host scripts stay outside the fault wrapper.
- [ ] Define host teardown, registration ownership, cancellation/unsubscribe,
  late completion invalidation and stale generation behavior. Browser
  destruction may terminate execution without running arbitrary guest
  cleanup; host-owned resources and instance disposal still end their
  obligations. No claimed graceful cleanup for a forcibly killed process.
- [ ] Use existing target capabilities and stable external call metadata
  to reject unclassified async boundaries before lowering. Ordinary foreign
  synchronous value-only bindings remain closed. Keep host async adapters
  separate from arbitrary external JS/WAT/native functions.
- [ ] Execute real event-loop tests: timer suspends while another task
  runs; actual browser event wakes a waiting receive; asynchronous
  completion succeeds and fails; cancellation while pending; page teardown
  and late wakeups; bounded ingress under burst; runtime error reported
  once. Use fake deterministic host scenarios for races plus real host
  tests for scheduling and ingress. Test synchronous-only artefacts do
  not include the scheduler or unused host helpers.
- [ ] Run doc and integration gates, selected builder tests and
  `just validate`. Record browser/host coverage and unsupported surfaces.

## Phase 8: Wasm and memory-planner handoff

This phase establishes a tested portable execution contract, not full Wasm
Channels lowering ahead of its prerequisites.

- [ ] Publish the accepted continuation handoff, resume/dispatch operation,
  native result/error slot transport, suspension-live storage and ownership
  transition requirements in compiler and memory authorities. Specify
  target-validated frame alignment/layout inputs, semantic commit order,
  instance/wait identity, host import completion/cancellation and cleanup
  responsibilities. Physical encoding belongs to the later validated
  memory/target plan. No source semantic type or new public fingerprint
  domain for physical frame state.
- [ ] Validate small core Wasm and Lappet mechanism fixtures for suspend
  back to host, resume, typed result delivery, cancellation, no re-entry,
  bounded frame storage and stale host token rejection. Use pinned runtime
  APIs and portable Wasm features, compare any optional feature with the
  portable route. Do not treat authored Wasm fixture success as Moth backend
  delivery.
- [ ] Define mixed JS/Wasm affinity/edge restrictions for suspendable calls,
  ownership of task/channel runtime and cross-target lifecycle. A JS-owned
  scheduler may drive guest continuation only through a validated ABI.
  Unsupported reachable cases receive target diagnostics until implemented;
  synchronous reachable code retains existing behavior.
- [ ] Review actual JS runtime with memory authorities. Identify frame,
  pending-send, queue, lease, wakeup and staged-result allocation families,
  retained edges, semantic cleanup events, logical close frontiers and
  exact count obligations. Keep non-atomic REC and one validated memory
  plan per variant. Never make endpoint lease accounting ordinary REC.
- [ ] Require subsequent physical memory implementation to consume these
  facts, including all cancel/join/close/teardown edges, and exercise
  Channels in collector-free verification. Require full Wasm plan to
  implement tested parity, not only synchronous numeric or queue smoke tests.

## Phase 9: Performance, review and closeout

- [ ] Measure synchronous frontend and output baselines, compile-time
  summary convergence, code size, allocations per task and per suspend,
  suspended-frame bytes, queue/waiter peak storage, rendezvous and buffered
  throughput, select arbitration/withdrawal, host wakeup latency and
  cancellation/cleanup cost. Include long-running worker pools and bounded
  event bursts. Compare identical configurations with recorded workload
  parameters; no invented thresholds or benchmark history edits.
- [ ] Review each semantic/runtime owner for duplicate parser rules, parallel
  summaries, carrier/continuation bloat, stale paths, lost-wakeup and
  double-release risks, profiling-conditioned legality, unsafe host
  retention, missing tests and unnecessary helper demand. Use the
  repository Slice review; independent correctness and redundancy reviews
  are required before final activation/closeout.
- [ ] Verify every accepted surface and every negative contract against
  canonical source tests, production/Boracle proof, scheduler model and
  real HTML-JS execution. Check typed runtime errors vs traps vs cancelled
  scopes, source diagnostic distinctions and cross-profile parity. No
  tests that silently hang: harness owns bounded execution and identifies
  expected deadlock vs test timeout.
- [ ] Update progress rows for actually delivered capabilities, diagnostics
  and targets. Update canonical async, compiler, memory, IO, external and
  builder references where checkpoints settled details. Remove obsolete
  async syntax/reservations and old plan references. Keep unsupported
  Wasm paths honestly gated. Do not edit generated docs directly.
- [ ] Run `moth check docs --terse` and `moth build docs --release` (or
  documented Cargo equivalents), inspect changed pages, run `just validate`
  at broad integration and `just validate-full` for final integrated
  merge readiness per current validation policy. Record exact commands,
  failures and measured evidence. Revalidate after integration changes.
- [ ] Review the remaining memory plan against actual Channels results
  before it activates, then update the full Wasm plan to consume the
  accepted handoffs. Transfer durable requirements to canonical authorities
  and retire this plan and its roadmap entry in the completion commit.

## Required review focus

- Parent retains an endpoint needed to unblock its child: close ordering
  and resource liveness, not only happy-path join.
- A sent graph is consumed only at commit: pending, failed, cancelled and
  losing paths conserve every responsibility and alias fact.
- A sibling fails during a nested suspend or staged return: correct owner
  receives the original error after all descendants clean up.
- Host event arrives concurrently with registration teardown or page reload:
  no stale wakeup, unbounded queue or guest re-entry.
- Transitive suspend/failure changes through a generic/imported function:
  correct stable summary invalidation, constraint verification and no
  profile-dependent acceptance.

## Completion boundary

V1 is complete for HTML-JS only when the full accepted Channels surface,
mandatory static safety, real host integration, failure/cancellation and
global deadlock diagnostics have executable evidence. The common compiler
and memory handoff plus validated Wasm/native mechanism evidence must be
ready for full backend implementation. Actual Wasm Channels support remains
deferred until full backend parity and collector-free memory-plan verification
pass. Fine-grained task groups, richer partial deadlock and protocol analysis,
parallel tasks and new async abstractions are outside this work.
