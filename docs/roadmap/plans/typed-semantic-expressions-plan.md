# Typed semantic expressions and dense compiler foundations

## Status

```text
STATUS: ACTIVE on expression-refactor (Phase 5 reactivated by the user)
CURRENT_SLICE: Phase 5. P5a call-site descriptor consolidation and shared generated registry accepted.
BLOCKERS: Fresh Windows proof remains pending.
NEXT_ACTION: P5b direct type resolution, removal of display-only type trees and range-domain reuse.
```

The main squash checkpoint retains expression-refactor source `534aea1a5`,
including the P2 isolation correction after reviewed checkpoint `20fddc3c4`.
Historical review `1343c64ec` and R2.4 qualification of compiler source
`6b855aab9` retain their original scope. Those gates and timing records do not
retarget the main squash tree. Windows-specific proof remains pending.

## Purpose and authority

Replace the growing, recursively owned `Expression` record with compact typed
semantic nodes addressed by IDs. Preserve Moth's architecture: AST construction
resolves types, applies contextual coercion and folds constants. TIR stays
AST-local and HIR remains the first backend-facing semantic IR. Bounded numeric
construction now completes at the owning body root before semantic publication.
This work changes representation and ownership, not that delivered timing.

The plan also completes the dense HIR, bounded Wiring and native result/Core
constant-evaluation foundations that would otherwise extend and then replace
the same owners. Remove Reactivity V1 before migrating expression storage.
Implement the accepted Wire/Route foundation on the final expression and result
contracts. Do not migrate obsolete reactivity or add an interim model to keep
its tests green.

`AGENTS.md`, the compiler/build authorities, canonical language references and
memory design own semantics. This plan owns delivery and investigation. Where
an experiment reveals an unresolved representation choice, the implementing
agent may select and refine a candidate under Phase 2's evidence rules. It may
not change language semantics, move stage responsibilities or relax validation
to make a candidate win.

### Obligation transfer

This plan supersedes three bounded work items in full. Their deletion records
consolidation, not implementation completion.

| Transferred work | New owner and completion evidence |
|---|---|
| HIR dense expression/place storage and capacity foundations | Phases 0, 2 and 3, including every durable carrier, consumer, freeze boundary, capacity statistic and performance gate |
| Wiring V1 cleanup and foundations | Reactivity retirement and public documentation in Phase 1, final parameter/identity/route semantics in Phase 6, stale-model audit in Phase 8 |
| Native result slots and Core constant evaluation | One result contract in Phases 0 and 2, integrated expression/HIR/ABI cutover in Phase 4, evaluator delivery in Phase 7 |
| Diagnostic arity and propagation-context work that depends on native results | Phase 4 owns single-target multi-result rejection, multiple-success postfix `?` handling and the narrow enclosing-function/entry facts needed for propagation diagnostics |
| Expression cloning and constant-adapter work overlapping post-TIR optimisation | Phases 4 and 5 own representation and direct handoffs, while independent cache, invalidation, source-span text and scheduling research stays deferred |

The old HIR plan's whole-AST exclusion no longer excludes semantic expression
storage. It still excludes an indiscriminate flattening of parser syntax,
statement collections and unrelated borrow facts. The old permission to lower
new AST results through a temporary tuple/carrier HIR bridge does not apply.
The former Wiring-before-native-results ordering also no longer applies.

### Capability checkpoints

Each checkpoint requires its named phases, affected validation and a merged
implementation before another branch may consume it. A phase checked off on an
unmerged branch does not deliver a prerequisite.

| Checkpoint | Delivers | Does not imply |
|---|---|---|
| Reactivity retirement | Phase 1 removes V1 compiler/runtime support and updates its documentation and coverage | Wire/Route execution or observation |
| Dense HIR | Phase 3 publishes dense expressions, flat places and capacity policy | Native result channels or expression-store migration |
| Typed expressions and native results | Phase 4 publishes the final semantic node/slot/channel contracts and all mechanical consumers | New borrow precision, never-return semantics or full Wasm support |
| Wiring foundation | Phase 6 completes closed `of`, Wire identity/forwarding and contextual constructor routes on those contracts | Subscriptions, route application, channels, async or UI event delivery |
| Core constant evaluation | Phase 7 completes trusted registration, shared folding and the five Text operations | Arbitrary source-function evaluation or expanded packages |

The package programme remains active in parallel. Eligible work consumes its
specific merged capabilities without waiting for unrelated final polish. Boracle
research stays paused until its explicit activation gate and the native-result
checkpoint are satisfied. Diagnostic layout Phase 4 still requires explicit
reactivation after the delivered foundations it names, a fresh inventory and
preservation of its locked decisions. This plan does not reactivate it silently.

## Reading and execution rules

At activation and after compaction, follow `AGENTS.md` and reload routed
material. Read the following authorities in full for this cross-stage work:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/compiler-data-layout-design.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- the scope, invocation, performance and completion sections selected by
  `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/developer-docs/language/overview.mtf` and its exact references for
  numbers, calls, results, options, errors, assertions, templates, generics,
  constants and Wiring
- `docs/src/developer-docs/memory-management/overview.mtf` and its routed borrow,
  lifetime, retained-edge and backend handoff references
- both progress matrices, `docs/roadmap/roadmap.md` and
  `benchmarks/frontend-optimization-results.md`

Read the canonical Core Text and external-function contracts before Phase 7.
Use current source paths as an owner map, not as instructions to preserve old
module shapes. Move contracts with their owner and delete replaced helpers.
Update permanent documentation in the phase that changes the contract. Keep
implementation status in the progress matrices and roadmap.

Execute the numbered phases in order. Within a phase, split work only where a
reviewer can assess a coherent deliverable. Each accepted checkpoint uses one
implementation path. A large local edit may be incomplete while being developed,
but do not commit a compatibility adapter, disabled validation or fake passing
fixture as an accepted checkpoint. Rebase the exact work packages against the
activation tree before touching their owners.

## Fixed design direction

### Semantic responsibilities

| Responsibility | Final owner and invariant |
|---|---|
| Parsing and contextual construction | Short-lived parser/builder state and body-owned numeric construction own incomplete operands, literal recipes and receiving witnesses until root completion. Completed semantic nodes do not impersonate declaration shells or unresolved placeholders. |
| Declarations and symbols | Dedicated records own identity, visibility, declared type and callable/constructor descriptors. A declaration without an initialiser has an explicit absence, not a fake expression. |
| Typed expressions | Compact IDs address stage-owned semantic nodes with resolved result shape, exact source location and the operation facts required by their consumers. Fixed children use IDs, variable payloads use typed ranges. |
| Results and failure | One authoritative zero/single/multiple success shape per producer, with a separate typed error channel. Optional absence and divergence remain distinct concepts. |
| Constants | Existing constant owners hold content. Occurrences retain source, type, contextual/config provenance and other occurrence facts without copying the content graph. |
| Places | A place owner describes the root and ordered projections. An indexed projection refers to its index expression by ID. Places are not nested general expression copies. |
| Failure and diagnostic facts | One authoritative owner retains required operation, boundary and witness facts. Sparse/cold records do not make every common node carry their largest payload. |
| TIR | Immutable AST-local roots/views retain their exact semantic overlay and owning expression domain until the accepted view materialises. No TIR identity crosses the HIR handoff. |
| HIR and analyses | HIR owns validated dense semantic data. Borrow, lifetime, link and other analyses consume it and publish their existing side tables. |

Keep structural syntax where it serves parsing. This plan does not create an
extra untyped AST pass, postpone type resolution to HIR or turn constant folding
into a later optimiser. Compacting semantic expressions should reduce work in
the type system and folder, not merely make one `size_of` result smaller.

### Representation constraints

- Use typed compact IDs and checked typed ranges with an explicit issuing owner.
  A store index owns node identity without a duplicated per-record self-ID.
  Source, expression, constant, TIR, type and HIR IDs remain different domains.
- Decide the expression store's lifetime and issuing scope before fixing the ID
  API. Do not assume body-local IDs can enter a module-owned TIR store. A raw
  local ID never becomes a cross-module, generated or public semantic identity.
- Fixed-arity children fit inline as IDs. A variable child range indexes a
  contiguous list of child IDs, not an assumed contiguous run of child nodes.
  Unrelated side-store element types remain distinct.
- Use typed `Vec` construction and fixed dense ownership at a stable handoff.
  Freeze topology only after the last owner that may add or remove structure.
  Owning scalar remapping may happen before final publication. Published
  read-only consumers receive no structural mutation API.
- Shared completed nodes are immutable. Substitution or folding returns the
  same ID when unchanged and an isolated replacement root/view when changed.
  Do not mutate a shared node through an ID or clone an authored graph to avoid
  defining ownership.
- Store common zero/single result shapes without a per-expression allocation.
  Multiple results have at least two ordered types. Every single-value consumer
  checks arity. `TypeId(0)` is a valid built-in type, not an absence sentinel.
- Keep operation selection, numeric domain, coercion/conversion boundaries and
  failure facts after semantic resolution. HIR lowering consumes those facts
  without recovering source policy from an RPN tree or the result type alone.
- Use safe Rust and small typed codec/accessor owners. No pointer tagging,
  generic allocator framework, lifetime-heavy arena library, node-level `Arc`,
  global hash-consing or a map per field. Preserve existing shared owners where
  their lifetime and semantics justify them.
- No recursive compatibility tree, second constant-value language, evaluator
  callback framework, global semantic registry or unused future-only variants.
  New indirection must remove work, establish an invariant or own a real lifetime.
- Compact capacity exhaustion from authored input produces a structured source
  limit diagnostic before truncation or allocation arithmetic overflow. Internal
  malformed IDs remain compiler invariant failures. Estimate errors only change
  allocation cost and never semantics.

## Phase 0 - accepted activation and restart baseline

**Complete.** The numeric restart fixed the owner inventory and result, TIR,
failure and service handoffs. The [benchmark report](../../../benchmarks/frontend-optimization-results.md)
owns activation, restart, candidate and validation evidence.

### Contracts retained after the numeric restart

- `ast/numeric_construction.rs` and `numeric_known_shape.rs` own body-local
  origins, recipes, receiving witnesses, symbolic shapes and continuations.
  `complete_numeric_body` resolves them before publication and releases them on
  success and failure. Independent functions and concrete generic bodies have
  separate domains. Permanent stores receive completed typed nodes.
- `ast/templates/tir/numeric_intake.rs` queues intake by body domain in the
  module-shared TIR owner. One root's completion leaves sibling intake intact.
  Completed TIR holds typed values. Drained intake alone leaves capacity live.
- Retained calls link numeric origins through declared parameter/result shapes,
  then resume concrete inference and completion. Request reservation/publication
  stays separate from numeric selection. Preserve inactive requests and the
  immediate expected-result boundary.
- Compact nodes retain canonical types, D15 domains, conversion boundaries and
  failure facts after body completion. Cast sources resolve independently of
  targets. Historical eager-literal layouts do not describe this contract.
- Authored safety eligibility, actual implicit/typed failure and deferred
  private-call eligibility have separate owners. Type-check handlers and their
  constraints before pruning. Retain sparse failure snapshots rather than the
  construction witness graph. Base/generated summaries converge after AST
  release, then lane installation/pruning refreshes final CFG and link facts.

### Remaining owner and lifetime map

Paths beginning `ast/` or `datatypes/` are relative to
`src/compiler_frontend/`. Refresh the owning source before changing a boundary.

| Owner | Contract used by remaining work |
|---|---|
| `ast/expressions/`, declaration records and `ast/module_ast/environment/` | Parser scratch stays transient. Completed expressions use the phase-owned builder/store. Canonical declarations own absence, pending state, signatures, receiver identity and defaults. Preserve resolved-lookup hiding, deferred validation and the bare-identifier constness exemption. |
| `ast/statements/functions.rs`, `ast/type_resolution/`, call/constructor preparation | Phase 5 consolidates borrowed resolved descriptors and default IDs, including diagnostic function-type/signature copies. Struct defaults survive projection. Choice payload fields stay default-free. |
| `ast/templates/tir/`, runtime handoff and static-if specialisation | AST-local views retain their expression domain and accepted overlay. Structural children inherit it, nested values use their complete context and no TIR identity crosses HIR. |
| `ast/module_ast/finalization/`, generic body rules and materialisation | Validation bodies serve static selection, terminality and failure-boundary checks, then drop before AST publication. Retain canonical `StableBodySyntax` and stable semantic inputs for later instantiation. Preparation survives emitted-AST lowering until generated convergence and consuming `freeze`. |
| `ast/const_values/` | `ConstValueStore` owns content under local `ConstValueId`. Public projection uses `PublicFoldedValue`. Phase 5 removes unnecessary resolution/materialisation graph round trips while preserving occurrence metadata and the shared scalar/optional/aggregate vocabulary. |
| `module_compilation/service.rs`, generated compilation and sidecar build | Lowering consumes AST and moves constant ownership into HIR. Live `Preparing` and frozen `Published` provider routes both use requester preparation and converge sidecar summaries before transaction publication. Frozen `ModuleMaterialisationContext` retains stable closure, artefacts, identity and path/string tables under `Arc`. |
| Single-file, config and direct-template services | Single-file uses the module service. Config returns owned folded values, records and spans, with declaration names in the caller's `StringTable`. Direct templates return `OwnedFoldedString` content retaining caller `StringId`s alongside resources, warnings and source snapshots. Their local type/constant/TIR owners end at projection/extraction, with captured source ownership retained through build-owned validation. |
| HIR, public/generated projection, borrow/Boracle and JS/Wasm | Consume native dense facts directly. Remap local domains at their owning handoff. Stable semantic projections cross modules, rather than raw TypeId, ExprId, ConstValueId, TIR or HIR indexes. |

Preparation retains canonical syntax and stable semantic projections, not shared
validation graphs or a cloned `ConstValueStore`. P4.5 proves compaction, detached
instantiation and owned-result consumption. Phase 5 owns descriptor/default
consolidation and direct constant handoffs.

## Phase 1 - accepted Reactivity V1 retirement

**Complete.** Compiler, analyses, Boracle, backends, HTML runtime and tooling
share one path without V1 reactive syntax, observation or scheduling. Ordinary
directives, templates/resources, fragments, defaults, alias/borrow semantics,
lazy assertion messages, entry activation and dependency invalidation retain
their contracts. Removed spellings use structured syntax diagnostics.

The public Wiring family now separates accepted design from executable support.
Phase 6 still owns shared member lists, head-directed `of`, exclusivity,
immediate receivers, forwarding, binding identity, choice updates and
child-message lifting. Future examples keep their labels, including directional
`reader, writer Channel of AppMessage = Channel()` endpoints. Credit retirement
to its own benchmark comparison rather than later representation work.

## Phase 2 - accepted representation and lifetime decisions

**Complete.** The compiler architecture owns the selected interfaces. The
benchmark report retains candidate evidence and reopening rationale.

| Decision | Accepted contract |
|---|---|
| Nodes and payloads | Safe Rust enum/AoS reference, owner-local typed IDs and checked typed ranges. Fixed children stay inline, variable children use explicit ID lists and cold facts use typed owners. The 64-byte projected-row ceiling guides decisions rather than describing production memory or licensing lost facts. |
| Results | Explicit inline zero/single forms and ordered multiple types. TypeId zero stays valid. Optional absence and divergence remain separate. |
| Construction and release | Scratch promotion of completed reachable nodes, immutable unchanged-root reuse and last-reader compaction. Candidate instantiations keep isolated replacement roots/overlays. Owned service results and detached preparation pass P4.5 lifetime proofs. |
| Fallible HIR | Invoke-like continuation with edge-defined success slots and a separate error slot. Only the selected edge defines its values. |
| Diagnostics and numeric content | Preserve the exact eight-byte SourceSpan, locked 22/10 split and captured snapshot owner. Keep integer extrema, float bits/signed zero, exact Dec coefficients/scales, U32 error codes and sparse origin-owned failure witnesses. |

Phase 5 keeps the existing sharing of `Arc<BigInt>` coefficients and established
interning/caches. The rejected owned-folder signature left copies and regressed
matched timings. Key normalisation cost and low TIR-cache reuse didn't justify
new formats or wider caches. Reopen an optimisation with a changed causal
hypothesis and complete lifetime/invalidation evidence. The diagnostic display
boundary remains with diagnostic-layout work, including its locked 32-byte
record and at-most-48-byte draft. Retain required TypeEnv ownership until that
boundary changes.

## Phase 3 - accepted dense HIR and binding contracts

**Complete.** Dense HIR, template/control-flow hardening and explicit binding
contracts remain accepted.

`HirValueId` addresses canonical module-owned expression rows. Arguments,
elements, entries, fields, String pieces and projections use typed ranges.
Flat places contain a root local and ordered projections, with index expressions
by ID. Statements, terminators, patterns and all durable consumers use this
representation directly, including remapping, numerical proofs, link facts,
borrow/lifetime analysis, Boracle, generated services and both backends.

`FrontendArenaCapacityEstimate` gathers cheap facts through existing preparation
and header aggregation. Naturally known local counts refine reserves. Hard caps
and saturating arithmetic bound estimates, while underestimation grows normally
and changes allocation cost alone. Structural freeze fixes counts/topology and
retained capacity. Owning scalar remapping can continue before publication.
Private-lane installation/pruning precedes final CFG validation and link-fact
refresh in base and generated modules, including unchanged reuse.

### Local definitions and assignments

`Write` distinguishes `DefineLocal` from `AssignPlace`, local destinations
distinguish `Define` from `Update` and CFG edges carry source/destination pairs.
`ValueKind` owns incoming place/rvalue classification.

Binding semantics come from explicit HIR and `ValueKind`, rather than raw `Load`
shape or borrow-analysis state. Analyses validate and track those operations
without reclassifying them. Future optimisations need a compiler-owned lowering
proof, rather than access to solver-local snapshots.

- Each dynamic definition creates a fresh binding occurrence and retires stale
  alias state, including repeated loop entry. Parameters receive definitions
  through the ABI. Compiler temporaries gain no Wire-visible binding identity.
- Assignments preserve mutable alias write-through. Call results count as caller
  rvalues even if lowering loads an internal result local. Fresh result bindings
  may share allocations without sharing the callee's binding slot.
- Multi-bind and edge transfer read every input before writing destinations.
  Selected edges define bindings afresh. Definitions create no allocation family,
  lifetime owner or capability by themselves.
- Range scratch locals `next_value`, `distance` and `candidate_in_range` use
  definitions. Persistent current/step/index state uses updates. A float candidate
  defines scratch output only when in range, while dedicated operations can
  explicitly update persistent destinations.
- JS constructs fresh place/rvalue wrappers for definitions and uses ordinary
  local/projected write helpers for updates. Solver-local mode snapshots, `UNINIT`
  decisions and unmeasured alias specialisation have retired. Boracle consumes
  the same definition/update semantics. Wasm retains immutable aliases and
  value-backed mutable writes, with its existing structured rejection for
  unsupported mutable binding aliases.

Unrelated HIR collections and broad borrow-fact compaction remain with their owners.

## Phase 4 - integrated typed-expression and native-result cutover

**Complete and squash-merged into main from expression-refactor source `534aea1a5`.**
R1, R2.0 to R2.4 and P2 isolation are audit-accepted. Phase 5 remains pending
explicit reactivation. The
[benchmark report](../../../benchmarks/frontend-optimization-results.md#r24-final-qualification)
owns historical R2.4 evidence for `6b855aab9` and exact-checkpoint timing limits.

**Contract:** selected stores/lifetimes, dense HIR, explicit local/edge
destinations and exact numeric construction form one coupled
typed-expression/native-result checkpoint across 4A to 4D.

Preserve exact identities, I32/F64 defaults, body-root completion, receiving
witnesses, D15 domains/checks, source-independent casts and authored safety
catches. Keep delivered generic expected-type boundaries and binding semantics.
Phases 5 to 7 own repeated-work reductions, Wiring and Core evaluation.

### Accepted 4A to 4C contracts

| Owner | Invariant for remaining work |
|---|---|
| Construction and values | Explicit declaration states, canonical generic preparation and resolved operator facts. Owner-local ExprId/ConstValueId stores replace recursive completed expressions. Checked scratch promotion, consuming freeze and TIR release preserve only required roots and facts. |
| Producers and receivers | One zero/single/ordered-multiple result contract covers calls, returns, multi-bind, joins, forwarding and entry functions. Producers execute once. Folded slots use ordinary constants, not tuple values or host-call caches. |
| HIR and consumers | Native success/error slots are edge-defined through lowering, private failure installation, analyses, stable projection and JS/supported Wasm. Catch joins operate per slot and every RHS precedes writes. Allocation/provenance unions stay conservative where alias precision lacks proof. |

Receiver diagnostics retain kind, target/slot counts and producer location.
Single-target misuse uses value-receiver diagnostics, not `InvalidMultiBind`.
Guidance permits multi-bind/forwarding without selecting slot zero or inventing
discards, dummy names or tuple results.

Compiler-owned non-exported `start`, including synthetic single-file roots,
keeps fragment slots and built-in `Error!`. Entry assembly activates it at most
once. Imported non-entry work stays dormant and private inferred failure
converges before publication.

### Accepted hardening prerequisites

Grouping uses flat transient `GroupOpen`/`GroupClose` markers, with no
`PendingGroup`. Completed-expression walkers use reusable heap stacks.
`validate_types` is iterative and constant materialisation borrows its store.
Capacity failure propagates without rolling back a builder that will be dropped.

Design B keeps raw ExprId rows compact and uses transient `OwnedExpr` or borrowed
token-bound `OwnedExprs` at construction/install boundaries. Check owners before
indices or mutation, including sibling indexes and pre-freeze handles. Freeze
issues a fresh token. `own`/`own_slice` are the only raw re-entry family and must
use the original builder: raw IDs cannot detect a wrong conversion owner.

Constant/alias retention follows actual installer roots and private accessors
avoid duplicate validation. The [checkpoint evidence](../../../benchmarks/frontend-optimization-results.md#typed-expression-hardening-checkpoint)
owns memory, diagnostic, platform and matched timing records. The accepted H2/H3
stage-cost exception is bounded to that change, not later deltas or scaling.
Windows canvas command-line failures remain separate from stack/benchmark proof.

### P4.5 - accepted lifetime handoffs

Store proofs cover dead validation/handler/cold/handoff payload capacity,
same-typed ordered values, replacements and losing/failed candidates. Accepted
TIR overlays and nested contexts survive owned materialisation. Canonical
preparation and frozen providers instantiate later in independent requester and
sibling domains, retaining const-template defaults and donor resource origins.
Service consumers retain recursive failure facts, exact source snapshots and
the required diagnostic `TypeEnvironment` after construction-owner release.

Known-true assertions retain canonical validation-only generic demand through
existing materialisation/convergence, without executable message work. Active
demand wins for shared identities. Inactive branches and abstract validation
roots publish neither class. Numerical catch validation roots remain until HIR
consumes them, then infallible lowering retains only protected value/provenance.

### Accepted closeout and R1 corrections

The existing new-binding guard owns source/import, alias, trait and
forward-declaration collisions, including parameters, loops and
option/choice/catch/template captures. Preserve `MOTH-RULE-0038` for
source/import collisions and `MOTH-RULE-0039` for Core trait reservations.
Value lookup stays value-only, hidden originals and namespace members gain no
bare visibility and compiler-owned `start` stays outside source lookup. Mutable
updates remain assignments. Bound names gain no invented previous-declaration
labels. Add no second lookup route or name registry.

Syntax-accepted multi-bind collisions reach their semantic owner. Native
I32/F32/I64/F64 results retain ABI types and order through execution. Optional
HTML-Wasm keeps its existing capability reason and site. Compact extent tests
prove checked helper boundaries, not production transaction atomicity.
Block/template collision owners retain source-review evidence rather than
dedicated collision cases. Argument-count allowances need owner-specific reasons.

Constant projection uses iterative heap-stack postorder without memoisation,
preserving authored field order, provenance and independently owned
repeated-child outputs. Debug `fold_value` handles the previously overflowing
shallow record chain at depth 1024 on the real main-thread stack. Separate 1 MiB
proofs cover source depth 256 and store depth 2048, not deep owned clone/drop or
downstream walkers. The bounded const-record CLI case hides HIR values and
doesn't prove HIR-pool conversion.

Canvas runs its unchanged script from a test-owned temporary `.mjs` file and
passes locally. Corrected-harness Windows execution remains pending. Keep the
historical OS error 206 exposure OPEN, separate from Windows stack/benchmark
proof and unchanged non-blocking CI policy.

The [benchmark report](../../../benchmarks/frontend-optimization-results.md#typed-expression-hardening-checkpoint)
retains restart anchors, completed-checkpoint gates and both historical timing
comparisons. H2/H3 costs, documented closeout variation and the final comparison's
four residual stage costs have separate, exact-checkpoint acceptances. None
creates a later regression allowance or changes performance/scaling budgets.

### R2 - grouped `not` follow-up

**Boundary:** R1, R2.0 to R2.4 and P2 isolation are audit-accepted and retained
in the main squash checkpoint. Preserve the user's testing-authority commit
`d767691b4` unchanged. Phases 5 to 8 remain unstarted and need explicit
reactivation, their own validation, independent audit and checkpoint.

#### Accepted R2.0 to R2.3 contracts

| Owner | Contract and proof boundary |
|---|---|
| HIR validation | `hir/validation/expressions.rs` validates consecutive logical-not rows iteratively. Preserve each row's type, region, value-kind and source mappings, active-path cycle detection and entered-row cleanup on success/error. Keep distinct malformed-IR checks, with no completed-node cache or general framework. Actual debug `moth check` passes at depths 512/2048. The bounded 1 MiB proof covers 2048 rows, while the same-validator three-row dangling retry covers error cleanup. |
| HIR lowering | `OperatorFrame` carries one `Option<bool>` current-block classification through consecutive successful `LogicalNot` frames instead of rescanning descendants. Keep the boundary scan and other operators' policies. Reuse only within the selected expression view and unchanged active-catch context. Preserve each unary node/span, operand evaluation, pending-prelude flush and current-block delivery, including short-circuit, call and recovery continuations. |
| JS emission | `JsEmitter::lower_unary_op` iterates consecutive unary HIR nodes in one buffer, lowers the terminal once and emits one parenthesised layer per operator. The existing short-circuit case executes 512 nested `not` operators through HTML-JS with exact output and effects unchanged. HTML-Wasm retains `MOTH-RULE-0064` on line 11. |

The [benchmark report](../../../benchmarks/frontend-optimization-results.md)
owns exact R2 cohorts, timings and provenance. Keep these comparison limits:

- Revised testing documentation changes the standard docs workload identity.
  Earlier cohorts remain historical, not matched pairs for changed inputs.
- The failed large-run preflight supplies no measured baseline. Original safe
  depths 32–256 precede R2.1. Deep pre/post-lowering comparisons use the
  post-R2.1 validator baseline, five independent frontend/CLI cohorts, one
  preflight and ten measured runs, timers only and one Rayon thread.
- Historical R2.2 measurements attribute near-linear HIR work through depth
  2048, not an overall compiler speedup. Runtime hashes match, but control
  inputs have path matches only and the uncommitted overlay has no raw patch
  or binary fingerprint. R2.4 adds committed-source and binary provenance.
- The pre-change both-backend overflow remains unattributed to a precise frame.
  Map-use and cast-policy prepasses still recurse by depth. Folded constant
  chains don't prove runtime HIR behaviour or broader backend stack safety.

The [R2.4 evidence](../../../benchmarks/frontend-optimization-results.md#r24-final-qualification)
owns the final cohorts, follow-up movements and local full gate. Keep both
historical and final raw records rather than resetting comparisons.

#### R2.4 - qualification and pause checkpoint

- Five matched standard frontend/CLI pairs retain frozen inputs. Five focused
  pairs compare post-validator `e70d351db` with `6b855aab9` through depth 2048.
  Alternating repeats investigate exposed standard-suite movements without
  waiving costs under older exceptions.
- `just validate-full` passes locally, including 6,105 primary units, 2,784
  integration target results, ten feature lanes, 148 preflights and 14 scaling
  series. The new grouped-not dev HIR series fits n^0.88 under n^1.25.
  Arithmetic's independent n^1.25 budget stays unchanged.
- Corrected-harness Windows Canvas and fresh Windows frontend qualification
  remain unavailable locally. Keep the OPEN ledger and non-blocking policy.
- Completed plan notes now retain future-facing contracts. The evidence report
  owns exact measurements and caveats. Documentation build/surface proof and
  independent scaling, evidence and documentation audits pass without findings.

The P2 isolation correction at `534aea1a5`, after review `20fddc3c4`, isolates
the three constant-store depth tests and the deep HIR `not` validator test in
child processes. All four retain
depth 2048, 1 MiB workers and their semantic assertions. The existing crate
test-support module now shares the project tests' exact-filter subprocess
runner. Focused validation passes 17 constant-store tests, 87 HIR-validator tests
and 15 project stack-budget tests. A temporary smoke probe confirms that a child
abort reports captured output without aborting the parent and that an unmatched
filter fails. The probe has been removed. This correction adds no Windows or
broader owned-value/backend stack-safety proof.

Pause after the merged Phase 4 checkpoint. Phase 5 requires explicit reactivation.
No persistent cache, new parser/arena, semantic-owner transfer, stack increase,
repeated-type/folding work or new roadmap-plan import belongs to this checkpoint.


### 4D. Checkpoint invariants retained by later phases

Preserve these owner-local contracts and their evidence through the remaining work:

| Contract | Required evidence |
|---|---|
| Store integrity | Growth preserves handles, typed ranges cover exact elements, empty ranges work, sibling-domain IDs cannot be confused, injected small limits exercise every compact overflow diagnostic without allocating billions of nodes. |
| Results and control flow | Zero results versus one optional `none` versus divergence, per-slot arity/coercion, same-typed slots with different values to expose swaps, exactly-once producers, all RHS before writes, success/error dominance, catch joins and valid/invalid propagation contexts. |
| Receiver diagnostics | Cover declarations, assignments, returns and nested `then` negatives, with matching multi-bind and already-legal forwarding positives. |
| Numeric fidelity | Preserve all exact numeric identities and entry services, I32/F64 defaults, body-root completion, authored-order receiver conflicts, D15 narrow/mixed domains, per-operation F16 rounding, full integer extrema, signed zero, exact Dec payloads, source-independent casts and U32 error codes. Reuse delivered numeric tests and add only representation-boundary assertions. |
| Immutable views | Two instantiations with different roots cannot contaminate one another. Structural TIR children inherit the selected overlay, nested values use their own context, runtime-dependent paths still undergo required validation and handoff materialises that same view. |
| Static branches | Both authored branches validate. Inactive branches create no active concrete generic requests, emitted HIR/runtime capabilities or active resource uses, while graph-active resource validity/dependencies remain mandatory. |
| Generic lifetime | Release unused validation AST/store, then instantiate later from retained canonical syntax. Base/generated/sibling local ID collisions cannot select the wrong object. |
| Failure lifetime | Drop AST/TIR before base/generated recursive convergence. Retain origin-owned source order, skipped recursive backedges, bounded private witnesses including three private hops plus origin/elision facts and typed deferred-catch producer/handler locations. |
| Subtle failure facts | Known-true assertions fully validate messages and retain private witnesses in inert replacements. Preserve authored safety eligibility independently of actual failure, typed handler constraints before pruning, post-discharge catch folding under D15, typed conflicts and error code zero. Equal-type U8 arithmetic remains U8, not the historical widened U32 example. Checked compound write-back priority applies to its own arithmetic, not unrelated RHS recovery. |
| Diagnostics | Render after expression/TIR release and source-file mutation using the captured source snapshot, donor labels, types and provenance. Retain the current required TypeEnv lifetime until the diagnostic owner changes. |
| Final HIR/publication | Validate final CFG, dominance and link facts after lane install, handler pruning and unchanged reuse in base and generated modules. Check supported JS/Wasm output through existing contractual owners. |

The stack-safe prerequisite owns grouped-arithmetic stack correctness. Its 1 MiB
child-process `moth check` tests (`src/projects/tests/check_tests.rs`) cover
constant chains at 512 and 2048, runtime-leaf chains at 256 and malformed or
postfix chains at 2048. Every completed-expression walker has its own
small-stack test. Keep these passing rather than adding parallel grouped cases.
Require the Windows in-process development-built frontend benchmark to pass on
its real main-thread stack, alongside grouped-numeric scaling within the
existing n^1.25 budget.

Preserve CI's stack budget and contractual fixtures. Recorded native-frame and
accepted/rejected depth measurements do not introduce new parser depth limits
at this checkpoint. Compact IDs relieve payload pressure, not every recursive
path.

Keep independent hazards with their owners when interpreting this evidence:
statement-body parsing reaches about 23 debug levels at 1 MiB and curly literals
about 15. Grouped-not classification now reuses one hint across consecutive
`not` nodes, and JS unary rendering emits each layer without accumulated-child
copies. Map-use and cast-policy prepasses remain recursive by expression depth.
Borrow-state memory scales as O(blocks × locals²). Keep these independent
hazards separate from expression-store and grouped-arithmetic costs.

**Accepted cutover:** no completed semantic expression tree, placeholder
declaration value, legacy result carrier or re-expansion adapter remains.
Affected services use the selected stores, release rules and native slots.
Later phases preserve the source, service, feature-lane and performance
contracts rather than restarting this representation migration.

## Phase 5 - reduce repeated type and folding work

**Consumes:** final representations and current type/fold profiles.
**Produces:** direct semantic handoffs and measured reductions in repeated work.
Representation adoption is mandatory. Optional optimisation mechanisms require
current evidence and can be declined with their investigation recorded.

- [ ] Consolidate callable/constructor preparation into the existing descriptor
  owner: parameter types/access/contracts, receiver facts, results and default
  IDs. Let parsing, inference, validation and materialisation borrow the same
  resolved facts instead of resolving or cloning full signatures repeatedly.
- [ ] Build routed argument/parameter order once. Validate, resolve and fold
  defaults once at their declaration under existing rules, even when no caller
  selects them. At a call, select/materialise/apply the already-validated default
  ID only for an omitted slot. Use borrowed field/variant views
  rather than rebuilding constructor metadata or cloning every variant.
- [ ] At rewritten boundaries, resolve `ParsedTypeRef` directly to canonical
  `TypeId` where no owned `DataType` tree is needed. Keep authored syntax in its
  real owner and diagnostic display in its own path. Avoid reverse type-tree
  allocation on successful executable paths.
- [ ] Preserve existing type interning, compatibility/substitution caches,
  nominal sealing and recursive witness rules. Reuse resolved operation/domain
  metadata in folding and HIR instead of re-deriving it from result types.
- [ ] Make constant lookup return a reusable handle and classification. Local
  environments refer to constants by ID over the existing shared module base.
  Avoid per-generic rehydration of all module constants and constant-store to
  AST to HIR graph expansion merely to cross an API.
- [ ] Remove no-change TIR operand clones, double list copies and whole authored
  AST clones used only to compare/discard a specialised view. Reuse unchanged
  roots and construct changed nodes once under the selected lifetime policy.
- [ ] Borrow text while appending and move completed owned Strings through the
  existing ownership-taking interner. Do not allocate a String merely to pass
  it to an API that borrows and then copies it again.
- [ ] Re-profile after each material change. Investigate further ideas, such as
  numeric factor reuse or narrow memoisation, only when remaining attribution
  and complete semantic keys justify them. Keep key lifetime, config,
  imported-constant and directive invalidation explicit. Do not repeat the
  rejected owned-folder/key experiments without a changed causal hypothesis.

**Checks:** required/opportunistic folding, partial/no folding, structured text,
defaults, generics, scalar/optional/aggregate values and contextual provenance.
Verify that folding does not manufacture a source `#` declaration, erase a
build/config dependency or alter checked-value failure policy. Compare type and
fold stages plus full compilation on the same surviving workloads.

**Exit check:** each retained optimisation has measured benefit or a documented
necessary ownership simplification with neutral performance. No duplicate
resolver/folder/cache, discarded graph construction or avoidable conversion
chain remains in the migrated paths.

## Phase 6 - complete the accepted Wiring foundation

**Consumes:** typed semantic nodes, native result channels, final signature and
public projection contracts, plus the permanent Wiring language reference.
**Produces:** the bounded foundation, implemented directly on final stores.

- [ ] Parse `of` once through its existing retained syntax owner. Dispatch by
  the resolved head. Ordinary generic application stays type substitution,
  `Wire of T` and `Route of T` become parameter contracts beside ordinary types.
  Reject unknown heads, malformed/nested forms and invalid contract positions
  with typed diagnostics. Do not create universal type wrappers.
- [ ] Support a Wire from an eligible named mutable or immutable local binding.
  It carries binding-instance identity and permits reading where an ordinary
  value is expected or unchanged forwarding to another Wire parameter. Keep
  mutability/exclusivity authority unchanged.
- [ ] Keep static ExprId/BindingId distinct from a runtime binding instance.
  Reassignment preserves the same binding's identity, separate loop/function
  activations create separate instances. Reject temporaries, literals, fields
  and unsupported storage/return positions according to the canonical contract.
- [ ] Support `Route of Output` and `Route of Input -> Output` only from a
  direct choice constructor in the immediate route-parameter context, or from
  an unchanged matching Route parameter. Bare `Route` is not a source contract.
  Zero-input forms retain the constructor identity and all payload values for
  later construction. One-input forms leave the canonical trailing payload
  unsupplied. Partial routes accept only a contiguous leading capture prefix,
  without holes, named omissions or reordering. Fully supplied zero-input
  routes retain legal ordinary complete-constructor named argument routing.
- [ ] Evaluate leading capture expressions once at route formation. Preserve
  their effects, error/control flow, aliases, lifetimes and provenance through
  ordinary semantic owners. Do not re-evaluate capture expressions at later use.
- [ ] Give the shared argument owner the selected parameter contract before
  constructor completeness checking. Validate ordinary constructors normally
  outside that context. No failed ordinary parse followed by a route retry.
- [ ] Preserve contracts in local/imported/exported signatures, generics,
  fingerprints, frozen/public semantic projections and HIR. Add only metadata
  consumed by supported formation/reading/forwarding, with no observer scaffolding.
- [ ] Implement faithful minimal synchronous lowering in supported backends.
  Reject unsupported reachable execution at target validation, without changing
  frontend legality. Test the actual supported-target matrix explicitly.
- [ ] Complete declaration/read/forward, wrong origin/context/type/arity,
  capture-once, per-activation identity, import/generic and borrow/provenance
  coverage. Revisit the Phase 1 teaching examples and status rows now that this
  checkpoint exists.
- [ ] Cover a Route between ordinary parameters, its arrow versus the enclosing
  return arrow after `|`, repeated/malformed arrows and missing output. Test
  ordinary underlying-type aliases and reject capability aliases, defaults,
  storage and returns. Preserve the compiler-owned head collision policy. Wire
  identity itself is not an active value borrow, and an ordinary value parameter
  cannot infer its caller's identity by inspecting the body.

**Exclusions:** observation, subscriptions, event delivery, route invocation or
composition, channels/async, UI scheduling and first-class capability storage
or return. Preserve accepted deferred design in its authority. Do not implement
an interim runtime to demonstrate a future example.

**Exit check:** the permanent bounded contract passes source/backend tests, with
one head-directed argument path and no legacy reactivity or result transport.

## Phase 7 - trusted Core external constant evaluation

**Consumes:** native producers/receivers, direct constant handles, final public
projection and the canonical external/Core Text contracts.
**Produces:** trusted operation registration and one shared evaluator path.

- [ ] Add optional `ExternalConstEvalOp` metadata beside existing external
  lowering metadata through the definition/registration/clone owners. Use a
  closed typed vocabulary with only live operation variants. It is not a
  source annotation, callback, name lookup or backend lowering.
- [ ] Validate registration before partial installation: trusted Core origin,
  shared parameters, accepted Fresh classification for every success slot,
  no error slot, supported concrete types/optional wrappers and a complete
  signature matching the operation. Builder/Dependency/provider JS stay ineligible.
  No spelling of `@core` can manufacture trusted origin.
- [ ] Dispatch by the resolved external identity through a borrowed definition
  and its small operation token. One AST-owned path resolves/folds arguments,
  evaluates and checks every result slot, then materialises ordinary constants
  through the same receiver as runtime producers. No `ExternalConstValue` graph.
- [ ] Use the shared path for operator operands, its single-operand fast path,
  constant substitution, required initialisers, supported defaults, templates
  and static-if finalisation. Keep pure RPN reduction separate from operation
  dispatch and make `ConstValueResolver` reuse that dispatch.
- [ ] Preserve normal visibility, argument/access/arity/coercion and handling
  validation before replacement. A foldable callee does not become an external
  constant or legalise restricted config imports, forward references, runtime
  bindings or arbitrary source-function interpretation.
- [ ] Use a typed required/opportunistic requirement and typed refusal outcome.
  Refusal for no operation, a runtime argument, unavailable concrete structural
  text or a deterministic bound preserves the call opportunistically and gives
  the owned diagnostic when required. Keep already-folded arguments and effects.
  Existing checked-value failures keep their established policy. Broken trusted
  signatures/output invariants are compiler bugs, not refusals.
- [ ] Reuse concrete-text resolution and the shared deterministic work/output
  bound owner. Define missing units/bounds once if needed, with checked lengths
  and tests. No wall-clock budget framework, guessed resource URL, locale,
  normalisation, host-time/randomness/IO or fallible host-call evaluation.
- [ ] Enable only existing `@core/text` `length`, `is_empty`, `contains`,
  `starts_with` and `ends_with`. Read their current exact signatures. Cover
  empty strings/patterns, non-ASCII/non-BMP and combining sequences against the
  canonical contract. A UTF-8 byte count cannot replace its length semantics.
- [ ] Test optional presence/absence and multiple success slots through the
  real normal materialisation/receiver and resulting HIR/backend boundary,
  including each permitted optional layer and per-slot type through constant
  storage, export/import and lowering. The five single-result Text
  operations cannot prove multi-result transport. Use a focused internal
  fixture at that boundary, not an invented public Core API, unused production
  enum variant or second test-only evaluator.
- [ ] Publish imported constants as completed values with stable identities.
  Consumers do not rerun provider evaluation. Preserve input/config provenance
  conservatively across results and normal graph-active resource validation.
- [ ] Test aliases, namespaces and re-exports plus independently built external
  registries, proving canonical identity resolves in the consumer without name
  dispatch or a donor-local index. Runtime parity cases must demonstrably retain
  a call. Mixed folded/runtime cases keep exactly the helpers they still need.
- [ ] A fully folded call leaves no external HIR call, runtime helper or JS asset
  dependency for that call. Refresh existing reachability/link facts. Preserve
  semantic visibility and build/cache dependencies used to compute the value.
  Test a visible operation on a target without runtime lowering: folded succeeds,
  the equivalent remaining runtime call receives the existing target diagnostic.

**Exit check:** five production evaluators and shared zero/one/multiple/optional
plumbing pass their owned tests. No mutable/opaque/unrepresentable values,
source-handler interpreter or broad package expansion slipped into this phase.

## Phase 8 - hardening, final review and handoff

- [ ] Re-audit every original expression/declaration/default/constant owner and
  all dense HIR consumers. Remove obsolete constructors, recursive walkers,
  duplicate facts, adapters, unused variants, temporary flags and stale comments.
- [ ] Verify semantic store release after each service, generic validation,
  finalisation and publication boundary. Measure mostly-folded and repeatedly
  specialised workloads as well as live expression graphs.
- [ ] Re-run the agreed memory/layout, type/fold, dev-stack and end-to-end
  comparisons on the final integrated tree. Confirm scaling stays within the
  existing workload budgets and that smaller headers did not increase retained
  memory, lookup traffic or compile time elsewhere.
- [ ] Review every affected future plan against delivered capability names.
  Keep terminality, diagnostic display-store migration, memory algorithms,
  broader borrow precision, full Wasm, independent template caching/scheduling
  and later Wiring runtime work in their actual owners.
- [ ] Update permanent architecture with selected layouts, lifetime/freeze and
  cross-domain projection contracts, and concise rationale for rejected choices
  that future work might otherwise repeat. Keep detailed raw measurements local
  and add the concise reproducible evidence to the existing benchmark owner.
- [ ] Update both progress matrices only for delivered behaviour/support,
  documentation, index/navigation and materially affected audit-log status.
  Do not record a new audit as part of an implementation Slice review.
- [ ] Record the five Text operations' folding/parity/elision evidence and the
  remaining evaluator/value-shape/runtime-only blockers in the living package
  notes and Packages/Builders matrix. Preserve separate Boracle research for
  independent/aliased results, projections/copies/unknown or joined origins,
  different last uses, overwrite, loops, recursion, catch and public/generated
  boundaries. Mechanical migration does not change the reference rules.
- [ ] Complete the validation and Slice review below on the final tree. Correct
  all findings that violate an accepted contract before declaring completion.
- [ ] Remove this plan and its roadmap entry in the completion commit after
  permanent decisions and remaining follow-ups have their proper owners.

## Shared validation and acceptance

### Phase cadence

Every phase ends with an architecture/style/ownership review, focused tests of
its changed contracts and a deletion/duplication review. Use the lightest checks
that cover the checkpoint under `validation.mtf`. Do not repeat every full suite
for each small commit, and do not claim an unrun lane passed.

| Checkpoint | Required evidence |
|---|---|
| Research or small implementation slice | Focused invariant/source tests and affected Cargo/features, meaningful before/after probes and corrected documentation where its owner changes |
| Representation or broader integration boundary | `just validate`, affected feature lanes, current HIR/borrow/Boracle consumers, JS and supported Wasm contractual checks, source audit and relevant non-recording benchmarks |
| Documentation-only branch checkpoint | Current `moth build docs --release` or the Cargo equivalent, route/link/example review and generated diff inspection |
| Final implementation/main-bound tree | `just validate-full`, plus affected Boracle/reference/oracle checks and any required lane outside the full gate. Record the exact tested tree/configuration. |

Use `--terse` for `moth check`. Select focused cases through current harness
commands rather than duplicating them in bespoke runners. Benchmarks remain
performance evidence, not correctness fixtures. Known environmental/baseline
failures need matched evidence and an explicit unresolved status, not suppression
or reclassification as passing. Narrow test isolation fixes are appropriate
when an existing shared path prevents valid gates, but do not broaden the plan
into unrelated test infrastructure work.

### Performance protocol

Use the existing frontend and end-to-end non-recording protocols, including
`just bench-frontend-check` and `just bench-check` where they cover the affected
work. Final acceptance needs five independent matched invocations, with paired
order where the protocol supports it, the same toolchains/features/profiles and
the same surviving source cohort. Use counters/timers/profiles for attribution,
not as a replacement for end-to-end measurements.

Report per-store and whole-compiler live/retained/peak bytes, allocation growth,
frame/stack behaviour, typing/folding stage time and representative project and
scaling results. Existing arithmetic/call/generic/default/template/constant,
collection/map, borrow and deep-nesting cases come first. Add a fixture only
when a required storage shape or cost is otherwise unmeasured.

Any repeatable movement outside noise needs explanation. A repeatable slowdown
above 5% on an affected representative case is automatically material, and a
smaller consistent regression is not an allowance. Revise the representation or
accessor before accepting a slowdown. Do not reset the benchmark baseline to
hide a loss. If an unavoidable tradeoff needs a change to the accepted performance
contract, report it explicitly instead of declaring the plan complete.

A benchmark preflight pass without a local baseline shows that cases run. It
does not establish unchanged performance. Credit removed Reactivity workloads
to retirement and compare later storage work against the post-removal baseline.

### Completion criteria

- Compact typed semantic nodes and dense HIR are the only durable expression
  owners. Parser scratch, declarations, signatures, places, constants and cold
  facts have explicit separate responsibilities and proven lifetimes.
- AST typing/folding timing, contextual numeric rules, exact source snapshots,
  TIR view isolation, failure convergence and stable publication remain intact.
- Native zero/one/multiple success slots and separate errors reach every producer,
  receiver, analysis and supported backend without a synthetic semantic carrier.
- The bounded Wire/Route contract and five Core evaluators are implemented on
  those final representations, with V1 removed and future runtime work deferred.
- Measured memory, type/fold workload, dev-stack pressure and end-to-end results
  support the chosen forms. No repeatable accepted compile-time regression remains.
- All transferred documentation, teaching, tooling, test and downstream handoff
  obligations have landed, with no transient-plan dependencies or duplicate owners.
- Required gates pass on the tree proposed for main, and the final Slice review
  checks ownership, stage boundaries, duplicate/legacy paths, test honesty,
  source diagnostics, docs/status/navigation and the complete accepted scope.
