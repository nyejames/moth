# Native result slots and Core constant evaluation

## Purpose and authority

Preserve zero, one or many produced values through AST, HIR and analysis without representing
multiple returns as a semantic tuple. Build Core external constant evaluation on that representation and
prove it with the existing five `@core/text` inspection functions before package expansion resumes.

This is an ordinary compiler implementation plan, not a living package plan or a new semantic
authority. The user approved the decisions below through a ten-question design interview. Publish
those decisions in the relevant canonical references during activation, before changing code.
Implementation observations and file paths below are navigation aids, not frozen source layouts.

## Current-state capsule

```text
STATUS: queued, design approved
CURRENT_SLICE: activation and merged-baseline audit
BLOCKERS: diagnostics Phase 3 completion and merge, then validated package-foundations baseline merge
NEXT_ACTION: finish both prerequisite merges into main, create a fresh worktree, then run Phase 0
```

## Scheduling and worktree boundary

This work is a serial compiler-foundation checkpoint. It is not part of the merge-isolated package
implementation lane.

1. Finish Phases 2 and 3 on `token-and-diagnostic-data-layout-changes`.
2. Pause that work after Phase 3, before `Compact diagnostics, type snapshots and frozen reports`.
3. Validate and merge that completed checkpoint into `main`. Do not start the compact-diagnostics
   refactor while the result representation is changing.
4. Reconcile and validate the current `packages-and-builder-progress-plan` foundations against that
   `main`, then merge that baseline. Package expansion remains paused.
5. Create one new implementation branch and worktree from the resulting HEAD of `main`. Record the
   actual revision, worktree and validation evidence in untracked working notes.
6. Complete and merge this plan before either diagnostics implementation or package expansion
   resumes. Both resumed worktrees must adopt the new `main` before their next code-bearing phase.

The planning branch is documentation only. Its publication does not establish that either
prerequisite is complete, authorise an early implementation start or perform those merges.

Phases 2 and 3 of the diagnostics work are still moving while this plan is written. At activation,
re-read current authorities and trace the current code. Preserve newer source IDs, spans, token
ownership and failure lanes. Never restore an older API because an example in this plan uses it.
No speculative baseline SHA belongs in the committed status block.

## Required reading

Read `AGENTS.md` and its current routes first. For this cross-stage refactor, read the full relevant
compiler, build, data-layout and memory authorities, including:

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/compiler-data-layout-design.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` for orientation only
- `docs/src/developer-docs/language/overview.mtf` and its relevant canonical unsuffixed references
- `docs/src/docs/functions/returns-and-multiple-values.mtf`
- `docs/src/docs/constants/constant-folding.mtf`
- the canonical errors, optionals, calls, value-block and package references under `docs/src/docs/`
- `docs/src/developer-docs/memory-management/overview.mtf` and its return, borrow and lifetime routes
- `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf`
- `docs/src/developer-docs/memory-management/boracle/boracle-reference-solver.mtf`
- `docs/src/developer-docs/memory-management/boracle/boracle-operational-oracle.mtf`
- both progress matrices and `docs/roadmap/roadmap.md`
- the supplied `moth-code-review-guide.md`, resolving its older file references through `AGENTS.md`

Use `index.md` to locate current owners. Read adjacent producers, consumers and tests before adding
an owner. The roadmap owns coordination with other plans. Name their prerequisite capabilities here
rather than linking short-lived plan files.

## Confirmed decisions

| Area | Approved decision |
|---|---|
| Scope | Implement the AST/HIR result refactor before Core external constant evaluation and broader package expansion. |
| AST result shape | Use an explicit allocation-free zero/single common path and an ordered multiple-result payload equivalent to `ExpressionResult` below. |
| Multiple values | They are separate semantic result slots, not a first-class tuple, collection or record. |
| HIR channels | Infallible and fallible functions retain ordered success slots and an optional single error channel. |
| Downstream representation | Backend-neutral IR and analyses retain slots. Physical packing belongs only to target-specific ABI/lowering owners. |
| Folding metadata | `ExternalFunctionDef` and `ExternalFunctionSpec` carry `Option<ExternalConstEvalOp>` beside backend lowerings. |
| Evaluator ownership | Compiler-owned typed operations identify Rust evaluation in AST. There is no JS evaluation, arbitrary callback or source purity annotation. |
| V1 eligibility | Core-origin bindings only, shared inputs, fresh success returns, no error channel. Multiple success returns and optional inputs/results are included. |
| V1 value limits | Use currently representable constant values and external signature types. Opaque host handles and unavailable language/ABI shapes remain deferred. |
| Folding contexts | Required constants and opportunistic ordinary expressions share one evaluator. A runtime declaration remains a runtime declaration. |
| First real operations | Enable `length`, `is_empty`, `contains`, `starts_with` and `ends_with` in `@core/text`. Other package APIs are not expanded here. |
| Borrow precision | Preserve result-slot identity and existing conservative legality. Put finer result alias/lifetime investigations in Boracle follow-up work. |
| Scheduling | Run on merged `main` after the two prerequisite baselines, with diagnostics and package expansion paused until this work lands. |

## Scope limits

This plan changes representation and implements a bounded const-evaluation capability. It does not
add source tuple syntax, generic destructuring, multi-value argument spreading, new return syntax,
fallible external const evaluation, mutable const execution, an interpreter for source function
bodies, JS compile-time execution or a general effect system.

It does not finalise the production borrow checker, lifetime solver or collector-free memory
implementation. It does not redesign sorting or implement the broader mixed JS/Wasm package backend.
Existing source-level return and option contracts stay intact except for the newly supported Core
constant calls. Existing target restrictions remain explicit where unrelated capabilities are absent.

## Activation audit: current owners and risks

The reviewed baseline used ordered AST call return-type lists but also manufactured a tuple
`Expression.type_id`. HIR collapsed multiple returns into a tuple return type, one call-result local
and `TupleGet` projections. Constant folding did not evaluate external calls. Reconfirm these facts
against the merged implementation rather than treating them as permanent truths.

Trace these owners or their current replacements:

| Concern | Starting locations |
|---|---|
| AST result typing and constructors | `src/compiler_frontend/ast/expressions/expression.rs`, `expression_kind.rs`, `expression_types.rs` |
| Call parsing, argument slots and coercion | `ast/expressions/function_calls.rs`, `call_argument.rs`, `call_validation.rs` |
| Receiving sites and returns | `ast/statements/multi_bind.rs`, `value_production/`, `fallible_handling/` and function-return validation |
| Early constants and references | `headers/constant_dependencies.rs`, `ast/module_ast/environment/constant_resolution.rs` |
| Fold entry points | `ast/const_eval/`, `ast/const_values/`, `ast/expressions/eval_expression/`, template folding and static-if finalization |
| Value storage and publication | `ast/const_values/store/`, `src/compiler_frontend/folded_value.rs`, public-interface projection and import materialization |
| External metadata | `src/compiler_frontend/external_packages/definitions.rs`, `abi.rs`, `ids.rs`, `registry.rs` |
| HIR types, calls and control flow | `hir/functions.rs`, `statements.rs`, `terminators.rs`, `hir_expression/`, `hir_statement/`, validation and remapping |
| Analysis and summaries | `analysis/borrow_checker/`, normalized problem extraction, Boracle, lifetime consumers and public call-summary publication |
| JS emission | `src/backends/js/`, package bindings and HTML external JS glue |
| Wasm and LIR | `src/backends/wasm/`, its actual LIR owner, target validation and HTML Wasm assembly |
| Package implementation | `src/builder_surface/core_packages/`, `src/backends/js/package_bindings/core/`, `src/first_party_js.rs` |

Search every tuple constructor, projection, interned tuple type and fallible carrier use. Classify
it as multi-result transport, a real semantic aggregate or target ABI storage. Remove only obsolete
transport and code that becomes dead. Do not preserve dead tuple machinery for hypothetical future
source tuples. Do not remove actual structs, choices, options or legitimate backend-private storage.

## Final representation contracts

### AST has one authoritative result shape

Use the approved conceptual shape:

```rust
pub enum ExpressionResult {
    None,
    Single(TypeId),
    Multiple(Box<[TypeId]>),
}
```

Use private constructors or equivalent validation so `Multiple` contains at least two slots.
`Single` adds no heap allocation. An absent result is distinct from a present optional value whose
contents are absent. An expression returning `String?` produces one result even when it folds to
`none`. Zero results also do not imply divergence. Preserve the existing control-flow/terminality
owner rather than treating this enum as a replacement for never-return semantics.

Replace the old single-type assumption throughout expression construction, validation, traversal,
coercion, diagnostics, finalization and lowering. Remove duplicated authoritative return-type lists
from call payloads when the enclosing result shape owns the same fact. Signature types and a call's
resolved result types have separate lifetimes, but a call must not retain two mutable copies of its
own result shape.

Single-value consumers ask for exactly one result through a checked accessor. Source misuse gets a
typed diagnostic at its receiving site. A malformed finalized AST is a compiler invariant failure.
Never substitute the first slot, a synthetic tuple, `NONE`, an inferred type or a dummy value to make
an incompatible consumer compile.

Multi-return calls and multi-value `if`, `match` and `catch` use the same slot vocabulary. Validate
arity and contextual coercion per slot. Retain each call argument's existing evaluated order and
parameter-slot mapping. A single collection, record or optional is still one value.

Separate a producer from its resulting values. A call is executed once even when it produces many
slots. An already-folded producer holds ordered ordinary single-valued expressions through the
shared AST value-production owner. It has no tuple type and does not become a new source expression
surface. Use the same receiving path for folded and runtime multi-value producers rather than adding
an external-only multi-bind path or leaving a dead host call with a hidden cached answer.

The concrete folded-value node name can follow the activation audit. It must be integrated into the
normal AST producer/receiver contract, not an independent `ExternalConstValue` tree. The scalar fast
path remains an ordinary literal or existing constant expression.

### HIR preserves success slots and the error channel

Functions expose a shape equivalent to:

```rust
pub struct HirFunctionReturns {
    pub success: Vec<TypeId>,
    pub error: Option<TypeId>,
}
```

Infallible calls produce ordered result locals. Successful return terminators carry ordered values.
Error return terminators carry one error value. Empty success lists are legal on both infallible and
fallible functions. Success results are not a `Result<Tuple<...>, Error>` semantic type.

Preserve explicit control flow for fallible calls. A fallible call has mutually exclusive success
and error continuations. Its result locals are defined only on the success continuation, and its
error local only on the error continuation. Use the current HIR block-argument/edge-definition owner
or an explicit invoke-like terminator. Freeze the exact Rust shape in Phase 0 and migrate its users
in one coherent cutover. A hidden tagged carrier followed by HIR tuple/payload extraction is not the
final form.

HIR validation checks call/signature agreement, result count and slot types, channel compatibility,
edge definitions, dominance and use-before-definition. Catch recovery joins success-slot values
from the call with the handler's produced values. Propagation transfers the error channel without
constructing a semantic tuple. Normal source validation of `!`, `return!` and catch remains mandatory
even when later folding could eliminate work.

Multi-bind receives each result into its own binding. Compute every RHS value before assigning any
existing target. This preserves swaps, aliases and mixed declaration/assignment targets. Argument
and return-expression evaluation each happen exactly once in their documented order. Fresh caller
bindings do not imply copied or disjoint allocations.

Value-producing control flow merges individual slots through the existing block/value-target
machinery. It does not recreate an aggregate to cross a join. Audit ordinary returns, direct return
forwarding where already legal, generated functions, receiver calls and implicit entry functions.
Avoid changing accepted source syntax just to simplify the refactor.

### Analyses and publication retain slot identity

Thread slot order through HIR walking, validation, rewrites, source maps, local def/use collection,
reachability, link facts, call summaries, borrow/lifetime inputs and generated artefacts.

Preserve external alias metadata per declared result. Keep result-to-parameter and result-to-result
relationships distinguishable in the transport. Distinct result locals alone do not prove distinct
allocation origins. In particular, a `Fresh` return classification must not be expanded into a new
claim of pairwise result disjointness without the relevant existing contract.

Keep the production borrow checker's accepted/rejected behavior conservative. Where it currently
uses a union of possible parameter roots, preserve that conservative answer on the new slots. Do not
invent precision from slot indexes or turn an unknown summary into freshness. Adapt Boracle input
extraction and its current reference/oracle paths to the new HIR, but do not change their reference
rules as part of the mechanical migration.

Cross-module artefacts use canonical type and binding identities, not donor-local `TypeId`, local
result IDs or registry-local external IDs. Imported constants publish completed values. Consumers
must not rerun a provider's evaluator. Frozen generic materialization resolves canonical external
identity against the consuming registry and uses the current compiler-owned operation metadata.

### ABI choices remain target-specific

JS may pack results into an internal array/object and unpack it at the call boundary. The JS owner
must call once, preserve slot order and preserve references to returned allocations. Host external
JS wrappers may keep their private success/error envelope, but that envelope is translated at the
ABI boundary rather than becoming HIR's semantic type.

Preserve result lists in every backend-neutral lowering representation. Determine the actual LIR
ownership at activation rather than assuming every LIR is shared. Wasm-specific lowering should
use native multi-value results for the types it supports. Carry zero/one/many result signatures
through function type encoding, calls, returns, validation and emission for that supported surface.
Unsupported aggregate/handle or fallible ABI cases remain explicit target-validation rejections
until their owned implementation exists. This plan does not claim complete Wasm package support.

Keep physical memory planning in its current owner. A target ABI carrier is not automatically a
source allocation, a new semantic lifetime family or a reason to merge result liveness.

## External constant-evaluation contract

### Metadata and registration

Add the same field to the existing definition and registration spec, and thread it through their
construction and manual clone paths:

```rust
pub const_eval: Option<ExternalConstEvalOp>,
```

It sits beside `ExternalFunctionLowerings`. It is not a JS/Wasm lowering, source signature feature,
`pure: bool`, callback, name lookup convention or dynamic plugin interface.

Use a closed typed operation vocabulary, initially only the actual Text operations. Organize it by
domain when that keeps ownership clear. Do not add empty Math/Time enums or unused variants merely
to anticipate future packages.

Registration rejects `Some(op)` unless:

- package origin is Core, established by the trusted registration path rather than an `@core` name
- every parameter uses shared access
- every success slot has the accepted Fresh alias classification
- there is no error return
- parameter and result types are supported by both the current external signature vocabulary and
  the current constant-value machinery, including optional wrappers
- the complete registered signature matches that operation's owned signature contract

The shape supports ordered success slots rather than imposing a single-return restriction. Validate
all output slots, not only slot zero. Optional nesting follows existing Moth type rules, including
preserving presence at each permitted layer. `Handle`, unresolved generic/inferred signatures and
unrepresentable external values are not admitted just because their JS implementation is convenient.

Check registration invariants before partially installing a definition. An invalid compiler-owned
registration or evaluator result is an internal failure, not an ordinary fold refusal. Provider JS,
Builder and Dependency registrations remain `None`. No annotation can select a Rust operation.

### Resolution and evaluation

The typed AST call keeps its resolved `ExternalFunctionId` and arguments. Its result shape belongs
to the normal expression contract. The evaluator looks up the borrowed definition by ID, copies the
small optional operation token and dispatches in AST-owned Rust code. It never dispatches by package
text, helper spelling or the numeric value of a dynamically assigned ID.

The successful evaluator result is an ordered slot list, conceptually
`ExternalConstEvalOutcome::Folded(Vec<Expression>)`. Every element is an ordinary single-valued
constant with its own TypeId. Keep a typed refusal outcome separate from the existing compiler
failure lane. Do not box a synthetic aggregate just to return several values.

Use the existing compiler-owned constant machinery:

1. Resolve the visible callee through normal package/module binding.
2. Parse arguments once, apply declared access rules and perform argument/result typing and
   contextual coercion. Validate the receiving site's arity and any source handling syntax.
3. Fold or resolve eligible argument values through the shared constant-resolution path.
4. Read `definition.const_eval`. Without an operation, retain the runtime call or report the
   existing required-constant restriction.
5. Evaluate only when every required argument is a genuine compile-time value with the concrete
   data that this operation needs. Shared access alone is not proof of constness.
6. Run the owned operation and validate every returned slot against the declared result shape.
7. Materialize the ordered values through the ordinary AST result/constant owners. A single result
   becomes a normal expression. Multiple results stay multiple results for their receiving site.
8. Continue ordinary operator/template folding, then publish constants or lower the remaining AST.

A definition without an evaluator may still have individually folded arguments. Preserve the call
and all argument effects. An operation returning no values must not manufacture an optional `none`
value. The initial five production operations all return one value, but zero-result call plumbing
must remain correct throughout the representation refactor.

### One fold owner, all relevant entry paths

Keep pure operator/RPN reduction separate from external operation dispatch. Extend the shared
compile-time expression fold and call it for operands before operator folding, including the
single-operand fast path. Reuse it from constant reference substitution, required constant
initializers, supported defaults, template interpolation/control flow and static-if finalization.

Audit the current early dependency and visibility rules before changing them. A callee occurrence
is not an external constant value, and admitting a foldable call does not legalize runtime bindings,
source function interpretation, forward references or new import syntax in restricted config source.
All graph-active resource references remain graph-active before folding or dead-code elimination.

`ConstValueResolver` must resolve eligible host-call arguments and reuse the same evaluator instead
of unconditionally rejecting every host call or maintaining a second operation switch. Advisory
const facts may improve an ordinary initializer but cannot create a source `#` declaration, change
header ordering or make runtime-dependent source legal in a required const context.

Retain source locations, per-slot types, synthetic-interface/config provenance and other current
value metadata needed by diagnostics and linking. The current call constructor may not already
merge argument provenance, so explicitly trace this. A folded output must not silently lose its
input's project-context dependency. Preserve conservative provenance across the produced slots
until a documented finer rule exists.

### Values, optionals and refusal

Reuse AST constants and the existing constant store. The owned public folded-value vocabulary is
the publication boundary, not a second evaluator input tree. Preserve typed optional Some/None
values and their payloads through folding, storage, export/import and materialization. Optional
absence is a successful value, not `NotFoldable`, an error channel or zero results.

Keep multi-result transport out of first-class constant storage. Where source syntax permits
constant multi-bind, publish its individual bindings through existing rules. This plan adds no new
constant multi-bind syntax. Never add a public tuple constant solely to store several call results.

Required and opportunistic folding share value semantics but differ in how a refusal is handled.
Use a meaningful requirement enum instead of spreading boolean flags across entry points.

| Outcome | Required constant context | Opportunistic context |
|---|---|---|
| All slots folded | Consume ordinary constant results. | Replace the call while preserving declaration semantics. |
| No evaluator or runtime argument | Explain why this expression is not a supported constant. | Retain the runtime call. |
| Structural string characters unavailable | Use the existing precise structural-text diagnostic. | Retain the operation until runtime placement is available. |
| Deterministic evaluation limit reached | Report the limit through the current typed diagnostic lane. | Retain the call without partial output. |
| Existing checked-value failure | Apply the canonical compile-time failure policy. | Apply the same established folding policy, not a new silent fallback. |
| Broken signature/output invariant | Report an internal compiler failure. | Report the same internal failure. |

Reuse `require_concrete_text` or its current owner. Constant resource/site-root strings may be fully
known structurally while their rendered characters remain unavailable. Length/search must not guess
a URL or inspect display paths. Reactive strings and runtime template snapshots remain runtime.

Use bounded operations, checked lengths/capacities and the existing const-evaluation limit owner.
If a missing work/output bound must be added, define one deterministic budget in the shared context
at Phase 0, with documented units and tests. Avoid wall-clock limits, evaluator-local magic limits,
a new execution engine or a compiler-wide budget framework unrelated to this slice.

Fallible calls remain ineligible for const evaluation even when their particular inputs would
succeed. Optional propagation and recovery still use the existing source/control-flow rules. This
work does not execute arbitrary source handlers or lift the restriction on fallible const calls.

### First production evaluators

Enable only these existing `@core/text` functions:

| Operation | Constant result |
|---|---|
| `length` | The canonical text length as the currently supported integer type. |
| `is_empty` | Whether the text is empty. |
| `contains` | The canonical case-sensitive containment result. |
| `starts_with` | The canonical prefix result. |
| `ends_with` | The canonical suffix result. |

Reconfirm the package contract before implementing Rust equivalents. Cover empty patterns, non-ASCII
text, non-BMP characters and combining sequences. Derive expected values from the accepted Moth
contract, not solely from comparing two implementations. Preserve the declared integer range and
existing string boundary behavior. A UTF-8 byte count is not a replacement for a character-count
contract. Do not introduce locale, case mapping, normalization or host Unicode-version dependence.

The existing runtime helpers remain the nonconstant path. Keep both implementations small and
handwritten. No new third-party runtime or evaluator library is introduced. A later package's Rust
and JS implementations must implement the same Moth contract, not merely call similarly named host
functions. Time, randomness and IO remain runtime-only in this slice.

### Runtime elimination and target validation

A fully folded call leaves no external call in HIR and no runtime helper or JS asset requirement for
that call. Recompute reachability/link facts through their existing owners. Retain normal semantic
package visibility, input dependencies and any supported cache-invalidating semantic dependencies.
Eliminating a runtime edge must not erase a build/config dependency used to compute a constant.

A visible Core function may fold on a target with no runtime lowering for that function. If the call
remains runtime, target validation still rejects it. Test both sides. This does not expose packages
that the builder did not select and does not broaden otherwise restricted config compilation.

## Implementation phases

Every phase ends with the mandatory gate below. A phase is a coherent checkpoint, not permission to
commit a half-migrated interface. Use bounded internal work items and audits, but land no duplicate
legacy/current API or compatibility wrapper to manufacture a green checkpoint.

### Phase 0 - merge checkpoint, audit and final contract map

- [ ] Confirm both prerequisite baselines are merged, both competing workstreams paused and the
      implementation worktree starts from current `main`.
- [ ] Run and record baseline validation, supported backend lanes, Boracle gates and known unrelated
      failures. Read the new data-layout contracts before naming any diagnostic or source type.
- [ ] Trace every producer/consumer in the owner table, including frozen generics, module interfaces,
      external glue, compiler metadata, return summaries and optional normalization.
- [ ] Record a compact final contract map in working notes: AST slot owner, folded-result owner,
      HIR call/return/continuation shapes, backend ABI owners and conservative analysis migration.
- [ ] Resolve concrete storage and any required evaluation budget against current owners. Measure
      the common AST shape and avoid a heap allocation per ordinary scalar expression.
- [ ] Update approved canonical design sections to the slot model and Core-evaluator capability,
      explicitly distinguishing accepted design from implementation status.
- [ ] Refresh roadmap/umbrella sequencing without marking prerequisites or this implementation done
      on the strength of this planning document.

Exit: the integration map and target support baseline are explicit. No code relies on today's line
numbers, obsolete SourceLocation layout or a guessed fallible-call representation.

Mandatory closeout: authority/design audit, style review and documentation validation. Baseline
code validation is also required to establish the starting point, not claimed as this phase's code.

### Phase 1 - AST native result production

- [ ] Introduce the single authoritative `ExpressionResult` shape and update all expression factories,
      scalar consumers, slot receivers, traversal and diagnostics.
- [ ] Remove synthetic tuple typing of calls and multi-value `if`/`match`/`catch` in AST.
- [ ] Use the existing value-production owner for runtime and folded ordered result lists. Keep
      single-value expressions on the ordinary fast path.
- [ ] Thread per-slot type/coercion/source facts through multi-bind and return checking, preserving
      exactly-once RHS evaluation and assignment ordering.
- [ ] Update the current HIR-lowering inputs and all other AST consumers so this phase builds and
      passes tests. The existing HIR transport is replaced in Phase 2, not duplicated behind a new
      adapter. This intermediate checkpoint is not the final semantic architecture.
- [ ] Remove replaced fields, forwarding accessors and tests that assume source multi-values have a
      tuple type. Add positive and negative receiving-site integration coverage.

Exit: AST has truthful zero/one/many production and no synthetic multi-return tuple TypeId. Current
consumers accept the new AST through one path. Existing language acceptance is preserved.

Mandatory closeout: full phase gate, with particular review of optional None versus no result,
source arity diagnostics, configuration provenance and scalar allocation cost.

### Phase 2 - HIR channels, downstream migration and ABI boundaries

- [ ] Replace aggregate function return types and single aggregate result locals with ordered
      success slots and explicit error-channel control flow.
- [ ] Migrate infallible calls, fallible calls, success/error returns, catch and propagation joins,
      multi-bind, value blocks, entry functions and generated functions together.
- [ ] Extend HIR validation and every rewrite/remap/def-use/reachability consumer to the final shape.
- [ ] Adapt borrow/lifetime summaries and Boracle extraction/reference/oracle inputs mechanically,
      preserving current conservative legality and outcome-specific result definitions.
- [ ] Update cross-module/frozen-generic/public-interface handoffs without retaining donor-local IDs
      or reconstructing signatures from earlier IR.
- [ ] Move JS multi-value packing/unpacking to its ABI owners. Preserve the external host envelope
      only at that boundary and keep returned allocation identity intact.
- [ ] Carry result lists through the current LIR owners and implement native Wasm result lists for
      their supported type surface. Keep unrelated unsupported target combinations explicit.
- [ ] Delete obsolete tuple/fallible-carrier transport, extraction helpers and compatibility paths.
      Retain real semantic aggregates and justified backend-private storage.

Exit: the compiler no longer packs function success slots or control-flow multi-values into a
semantic tuple at AST/HIR/shared-analysis boundaries. Backend output and conservative borrow
behavior match the source contract.

Mandatory closeout: full phase gate, both Boracle lanes when affected, backend/ABI validation and
per-slot source-map/edge-definition audits. This is a broad atomic migration, so review call,
control-flow, analysis and backend work as separate bounded audits before committing the phase.

### Phase 3 - Core evaluator registration and shared fold path

- [ ] Add `const_eval` to definition/spec construction and cloning. Introduce only live typed
      operation IDs and validate complete signatures, Core ownership and per-slot restrictions.
- [ ] Implement the AST-owned external evaluation path and ordered successful output contract using
      existing constant values. Include optional inputs/results and multiple success slots.
- [ ] Route direct expressions, RPN operands, reference substitution, supported defaults, templates
      and static-if consumers through the same fold owner.
- [ ] Preserve refusals, internal failures, checked-value policy, metadata/provenance and required
      versus opportunistic behavior without swallowing errors or returning partial results.
- [ ] Enable all five agreed existing Text functions in the same slice. All other production
      bindings retain `None` unless separately approved.
- [ ] Prove folded calls disappear before runtime reachability and target checks, while dynamic calls
      and unavailable packages retain their existing restrictions.
- [ ] Add the multi-result/optional materialization and receiving-boundary tests described below.
      No new public text API is added solely to exercise infrastructure.

Exit: real Moth programs fold the five Text functions end to end. The evaluator and slot machinery
support optionals and multiple success results, with explicit evidence independent of those five
single-result APIs. No unused scaffolding is declared complete.

Mandatory closeout: full phase gate, registration/identity audit, JS/Rust semantic parity tests and
artifact absence checks for fully folded calls.

### Phase 4 - hardening, documentation and handoff

- [ ] Complete the cross-boundary coverage matrix and prune overlapping fixtures.
- [ ] Compare production and Boracle outcomes with the activation baseline. Explain any changed
      diagnostic or acceptance result rather than treating precision changes as an automatic win.
- [ ] Review both optional layers and every multi-result slot, including swapped same-type results,
      fresh versus aliasing runtime returns and error-only calls.
- [ ] Run non-recording compiler performance checks. Investigate material scalar-path, allocation,
      emitted-code or compile-time regressions without creating broad optimization work.
- [ ] Update all canonical references, progress rows, living package notes and the Boracle TODO
      targets listed below. Keep unavailable evaluators and target limitations visible.
- [ ] Re-audit for obsolete tuple transport, duplicate result typing, HIR mutation by analyses,
      JS execution during folding and package-name-based evaluator dispatch.
- [ ] Run the complete final gate and record exact commands/results. Merge only the completed work.
- [ ] Remove this ordinary plan and its roadmap entry in the completion commit. Transfer durable
      implementation rationale into canonical/developer/package notes first.
- [ ] Resume diagnostics and package work only from the merged main containing this refactor.

Exit: final result shapes, Core const evaluation and all current consumers agree. Deferred Boracle
research and package evaluator gaps remain discoverable without reopening this implementation.

Mandatory closeout: full phase gate and final independent correctness, architecture and style
reviews, followed by the AGENTS Slice review. No completion claim with a required gate unrun.

## Mandatory phase gate

### Correctness, architecture and style

Audit the phase against canonical language/compiler/build/memory contracts and the approved changes.
Apply `moth-code-review-guide.md` and the current style guide. Check single ownership, allocation and
copy costs, narrow interfaces, diagnostics, source provenance, dead code and redundant tests.
Analyses consume validated HIR and write facts. They do not repair or rewrite it.

Check especially that a slot list never acquires first-class tuple behavior, that optional absence
never becomes arity zero and that a physical carrier never becomes a semantic allocation proof.
No new compatibility wrapper, fallback to old tuple typing or duplicate operation registry survives.

### Contract-based testing threshold

Every new or changed user-visible contract family has a primary integration owner. Prefer a few
rich Moth programs covering related cases over a fixture per helper. Hidden invariants and malformed
compiler metadata belong in focused tests under their owning test modules. Include meaningful
positive and negative cases, not acceptance-only compilation as the main proof.

The following evidence is required by final closeout:

| Family | Required evidence |
|---|---|
| Zero/one/many source results | Calls, returns and mixed multi-bind targets. Reject single-target reception, arity mismatch and illegal argument spreading. |
| Value blocks | Multi-value `if`, match and catch joins, including nested producers and branches that terminate. |
| Fallible runtime channels | Empty/one/many success slots, success path, recovery, propagation and error-only functions. Success values are unavailable on error paths. |
| Evaluation order | A rich runtime scenario detects repeated calls, reordered argument effects or RHS writes committed before all results exist. |
| Optional values | None, Some, permitted nested optionals and optional payloads in multiple slots. Preserve slot TypeIds through store/import/lowering. |
| Text folding | All five operations, nested calls/operators, supported constant contexts, imported constants, ordinary initializers and static conditions. |
| Text parity | Contract-derived empty/Unicode cases run folded and through a demonstrably runtime path. Verify the runtime call remains in that parity lane. |
| Identity and visibility | Namespaces, aliases/re-exports and independently constructed registries preserve the intended operation. Non-Core origin and spoofed names cannot opt in. |
| Refusal and diagnostics | Runtime arguments, missing evaluator, structural URLs, invalid source arity/types, reactive strings and evaluation limits where implemented. |
| Runtime elimination | Folded-only inputs omit calls/helpers/assets, mixed inputs retain only required runtime code, and a folded visible operation needs no runtime backend lowering. |
| HIR and target invariants | Slot types/order, continuation definitions, remaps, Wasm signatures and JS ABI shapes. Include malformed internal data tests. |
| Analysis stability | Current borrow outcomes and Boracle normalized traces preserve ordering, provenance and conservative alias facts after carrier removal. |

Multiple/optional evaluator outputs need explicit boundary tests even though the five initial Text
operations do not expose those signatures. Use the owning compiler test harness to feed typed
successful evaluator outputs into the actual fold/materialization and receiving paths, then inspect
or execute the resulting HIR/backend output. Pair that with real-source multi-value/optional
integration scenarios and the five real dispatch-to-output Text cases. Test registration rejection
separately with mismatched operation signatures. Do not add fake public Core APIs, production-only
fixture enum variants, an arbitrary callback facility or a parallel mini evaluator for these tests.
Record precisely which boundary each test proves rather than claiming a public multi-return Text
operation exists.

### Validation and evidence

For each code-bearing phase, run the current complete code-bearing gate and focused phase cases:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just first-party-deps
just validate
git diff --check
```

Run `just boracle` whenever HIR extraction, shared borrow problems or reference consumers change.
Run `just boracle-campaign` when oracle/campaign consumers change and at final closeout. Standard
validation does not implicitly cover these opt-in lanes. Reconfirm command names and prerequisites
from the activation checkout. Run the owning JS runtime and Wasm validation/execution lanes for all
advertised affected support. An unavailable environment is an unrun gate, not a passing result.

Use `just bench-check` and `just bench-frontend-check` for deliberate non-recording final performance
evidence. Avoid recording new baselines merely to hide regressions. Keep raw evidence in untracked
working notes and reports with the actual build revision/configuration.

Documentation-only slices use `moth build docs --release` or
`cargo run --quiet -- build docs --release`, inspect generated changes and check links. They do not
need redundant compiler-wide validation. Code-bearing phases still use the full gate.

Finish each phase with documentation/progress closeout and the AGENTS Slice review. Fix blockers
before advancing. A deliberately deferred nonblocking item needs an owner, reason and durable TODO.
Do not defer a regression in this plan's promised surface simply to close a phase.

## Required documentation, roadmap and matrix changes

Perform updates when the owning behavior lands, not as advance claims of support.

| Owner | Required change |
|---|---|
| `docs/compiler-design-overview.md` | Document AST result arity, HIR success/error channels, fold ownership, slot-preserving analysis/publication and target ABI separation. |
| `docs/build-system-design.md` | Update affected backend handoff/ABI descriptions. Keep package availability distinct from a remaining runtime dependency. |
| `docs/compiler-data-layout-design.md` | Reconcile any affected expression/result storage and diagnostic type-fact descriptions with the then-current Phase 3 model. Do not implement compact diagnostics early. |
| Canonical functions/value-block/error references | Preserve source syntax and describe multi-value receiving rules without semantic tuple language. Separate current conservative alias analysis from future precision. |
| Canonical constants and Core text references | Describe supported Core evaluator calls, required/opportunistic behavior, optional/multiple success capability, structural-text refusal and exact implemented Text surface. |
| Developer compiler-design explanation pages | Update affected diagrams/examples and AST/HIR/compile-time handoffs from the canonical contracts. |
| Compiler progress matrix | Track native result-slot pipeline coverage and Core binding const-eval capability. Mark stages/targets Partial until their tests pass. Retain unsupported Wasm and fallible-const cases explicitly. |
| Packages and Builders Progress Matrix | Update the `@core/text` row for the five enabled evaluators and their parity/elision evidence. Record per-package evaluator gaps without claiming all Core calls fold. |
| Package umbrella and future living plans | Make the merged result-slot/const-eval foundation a prerequisite. Require per-operation notes for supported evaluation, runtime-only behavior, parity blockers and value-shape blockers. |
| Boracle docs and roadmap TODOs | Add the follow-up below as an investigation, not implemented reference semantics or a new production checker claim. |
| `index.md` and audit records | Update moved/fundamentally changed owners. Mark affected audit coverage stale under the audit rules. A Slice review is not a new structured audit. |
| Main roadmap | Enforce the serial checkpoint after diagnostics Phase 3 and the package baseline merge. Resume both workstreams after this lands, then delete this plan entry with the plan. |
| Diagnostics work-item state | At the Phase 3 checkpoint, record the validated main handoff and block Compact diagnostics, type snapshots and frozen reports until this compiler foundation lands. Keep the remaining diagnostics work open and refresh its activation assumptions afterward. |

The compiler-foundation plan remains outside `plans/packages/`. Package plans name its delivered
capabilities rather than linking a short-lived plan. The umbrella keeps its living-plan lifecycle
and existing package order after the checkpoint. Sorting's approved contract and prerequisites are
unchanged.

### Boracle follow-up to retain

Add under Boracle future investigations, with a corresponding concise roadmap TODO:

- After the native-result refactor, investigate result-to-parameter and result-to-result alias
  relationships per success slot, including per-slot external metadata and outcome sensitivity.
- Compare independent results, two aliases of one input, aliases of different inputs, projections,
  copies, unknown summaries and joined possible origins. Distinct locals are not disjointness proof.
- Exercise partial result use, different last uses, overwrite/rebinding, loops, recursion, catch
  paths and cross-module/generated calls with adversarial source cases.
- Keep current reference mode conservative. Run stronger precision rules as named experiments and
  report deltas against the operational oracle and the accepted memory contract.
- Investigate which per-result facts help later lifetime reasoning without making Boracle the
  lifetime-topology authority. Preserve that ownership boundary.
- Use the evidence to inform the later final production borrow checker. This representation plan
  does not authorise new acceptance rules or a result-sensitive lifetime solver.

### Deliberate deferrals

Record these in the relevant matrix limitations, canonical design-scope references and durable
package/Boracle notes so they are not rediscovered:

- fallible external const evaluation and evaluation of source catch bodies
- mutable/aliasing const operations and compiler-time host handles
- general source-function interpretation and provider-supplied compile-time execution
- broader collection/map/opaque signatures that the current binding ABI cannot express
- time/calendar and floating-point evaluator parity, Unicode table/version-sensitive text behavior
- per-result borrow/lifetime precision, owned by the Boracle investigation and later checker design
- unrelated Wasm/native package capabilities and broader mixed-backend ABI work
- evaluator caching or incremental memoization until the semantic dependency/key design exists

## Final acceptance

The plan is complete only when the merged code preserves native result arity through AST/HIR and
analysis, fallible channels have edge-correct results, backend packing stays private, the five Text
operations fold through one trusted compiler path, multiple/optional successful outputs are tested,
current conservative legality is preserved and every affected supported lane passes its gate.

Remove obsolete implementations rather than leaving them as migration notes. Leave current status,
per-package evaluation gaps and Boracle research tasks in their durable owners, then delete this
ordinary plan in the completion commit.
