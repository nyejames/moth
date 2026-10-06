# Typed semantic expressions and dense compiler foundations

## Status

```text
STATUS: queued, design direction approved
CURRENT_SLICE: Phase 0 - activation, contract map and baseline
BLOCKERS: the Dec contextual-materialisation correction, followed by the separate Uint addition, must be merged
NEXT_ACTION: activate from current main after both numeric prerequisites, refresh the owner inventory and record the baseline in working notes
```

## Purpose and authority

Replace the growing, recursively owned `Expression` record with compact typed
semantic nodes addressed by IDs. Preserve Moth's architecture: AST construction
resolves types, applies contextual coercion and folds constants, TIR stays
AST-local, and HIR remains the first backend-facing semantic IR. This work
changes representation and ownership, not the timing of those decisions.

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
| Parsing and contextual construction | Short-lived parser/builder state owns incomplete operands, numeric literal recipes and pending receiving context. Completed semantic nodes do not impersonate declaration shells or unresolved placeholders. |
| Declarations and symbols | Dedicated records own identity, visibility, declared type and callable/constructor descriptors. A declaration without an initializer has an explicit absence, not a fake expression. |
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

## Phase 0 - activation, contracts and baseline

**Inputs:** merged Dec contextual-materialisation correction and the separately
planned Uint addition, delivered MON syntax/tooling and the existing numeric
profile contracts. Do not infer Uint's exact identity or width from its spelling.
Read its merged contract, including literals, external signatures and profiles.

**Deliverable:** a current owner/lifetime map, an obligation checklist tied to
this plan, reproducible baseline evidence and concrete work packages for Phase 1.
Keep baseline revision/toolchains/worktree details in working notes rather than
pinning this queued plan to a speculative commit.

- [ ] Record the activation tree, effective Rust/Node/tool flags, build profiles,
  validation state, relevant existing failures and benchmark environment. Pin
  the comparison cohort and retain raw measurements under `tmp/`.
- [ ] Inventory every durable and transient use of `Expression`,
  `ExpressionKind`, `ExpressionRpnItem`, declaration/signature/default records,
  constant materialisation and recursive expression/place carriers. Classify
  which fields represent syntax, semantic identity, content, occurrence facts,
  analysis summaries, control flow or obsolete reactivity.
- [ ] Map single-module, single-file, config, direct-template, generic validation,
  concrete materialisation and generated-sidecar services. For each owner record
  its issuing ID domain, dependencies, last reader and release point.
- [ ] Freeze the semantic contract for zero/one/multiple results, fallible call
  continuations, per-slot aliases/provenance, normal-root `start` and the private
  inferred-failure installer. Select the precise HIR control-flow form in Phase 2.
- [ ] Map TIR overlay inheritance, nested-value context, static-if views and
  retained generic syntax. Identify facts that must survive AST release before
  failure-summary convergence or diagnostic rendering.
- [ ] Census node and payload counts, variant frequencies/co-occurrence, list
  lengths, cardinality bounds, read frequency, clone/copy traffic, allocation
  growth and retained bytes. Instrument only facts needed to answer a decision.
- [ ] Profile type resolution, compatibility/substitution, call/default/constructor
  preparation, constant resolution and folding separately from HIR and backend
  work. Record stage and end-to-end time, peak memory and dev-stack pressure.
- [ ] List the source, unit, backend, Boracle and benchmark owners needed by each
  phase. Add coverage only for a missing contract or invariant, with one primary
  test owner per behaviour.

### Starting owner map

Refresh these paths and follow their callers at activation. They identify the
work that already exists, not proposed extra abstraction layers.

| Area | Starting owners | Required question |
|---|---|---|
| Expression and parser scratch | `src/compiler_frontend/ast/expressions/{expression.rs,expression_kind.rs,expression_rpn.rs,parse_expression.rs}` | Which fields/variants need to survive semantic completion, and which only serve parsing? |
| Symbols, declarations and types | `src/compiler_frontend/headers/module_symbols.rs`, `ast/ast_nodes.rs`, `ast/module_ast/environment/`, `ast/type_resolution/`, `datatypes/` under `src/compiler_frontend/` | Where do fake values, duplicate signatures and ParsedTypeRef/DataType/TypeId round trips enter? |
| Calls and defaults | `ast/expressions/{function_calls.rs,call_argument.rs,call_validation.rs,constructor_views.rs,struct_instance.rs,choice_constructor.rs}` and `ast/generic_functions/calls.rs` | Which resolved descriptors and default IDs can parsing, inference and validation borrow once? |
| Operators and folding | `ast/expressions/eval_expression/`, `ast/expressions/const_eval/`, `ast/const_values/`, `ast/templates/` | Which resolved operation facts, borrowed values and unchanged roots get discarded or copied? |
| Views and generic lifetime | `ast/module_ast/finalization/`, `ast/generic_functions/{body_rules.rs,templates.rs,materialisation/}` | Can unused validation data be released while later materialisation retains canonical syntax and stable semantic inputs? |
| HIR and failure closure | `src/compiler_frontend/hir/`, especially `hir_expression/`, `failure_facts.rs` and `private_failure_lane.rs` | Which carriers, rewrites and analysis tables must change together before final publication? |
| Cross-stage consumers | `src/compiler_frontend/module_compilation/generated/`, borrow/Boracle extraction, public interfaces and both backend lowerers | Which local IDs need remapping, which stable projections need rebuilding and which published facts must refresh? |

Paths shortened to `ast/` and `datatypes/` in this table are relative to
`src/compiler_frontend/`. Confirm module ownership before adding files.

**Exit check:** every transferred obligation has a phase and a test/documentation
owner. Every ID domain has a defined lifetime, or a named Phase 2 experiment that
will decide it before migration. No numeric, TIR, result or failure semantic
decision remains hidden in an implementation convenience.

## Phase 1 - retire Reactivity V1 before storage migration

**Consumes:** Phase 0's owner and fixture inventory.
**Produces:** one supported compiler path without V1 reactivity, accurate public
documentation and a second baseline for representation comparisons.

- [ ] Remove reactive `$Type`, `$=` and `$T` parameter forms, `$(source)`
  interpolation and obsolete call/header metadata through their current owners.
  Audit deferred `$bind` prose separately rather than assuming it was implemented.
  Preserve legitimate `$` directives and use ordinary structured syntax diagnostics
  for removed source. Do not add a migration-only diagnostic family, renumber
  existing codes or reuse a retired code for a new Wiring meaning.
- [ ] Remove expression/TIR reactive metadata, String reactivity fixed points,
  subscription summaries, public/generated projection fields, HIR invalidation
  machinery, JS scheduling/snapshot paths and reactive HTML mounting glue.
- [ ] Preserve ordinary template evaluation, structural/resource Strings,
  fragments and insertion, value/alias/borrow contracts, assertions and existing
  builder lifecycle ownership. Do not turn normal retained values into a new
  subscription representation.
- [ ] Classify every affected fixture. Delete tests whose only contract was
  retired behaviour. Rewrite mixed-purpose tests around the surviving semantic
  contract and retain their original failure evidence. Remove test-only V1
  hooks and stale audit allowlists with the production owner they served.
- [ ] Publish the Wiring teaching/reference structure described below, clearly
  separating accepted foundation contracts from executable support and future
  work. Remove obsolete public reactivity pages and navigation. Correct async
  channel examples without implementing channels.
- [ ] Audit compiler/build/memory references, progress rows, source examples,
  grammar/highlighting/editor snippets, test instructions, diagrams, `AGENTS.md`
  routing and `index.md` for the retired model. Change each affected owner, not
  merely the page visible from the site menu.
- [ ] Inspect indirect helper-demand, template-object preservation, String
  coercion, map-key and numeric-formatting paths even when their names omit
  `reactive`. Give each remaining hit a remove/adapt/preserve/historical/external
  follow-up disposition. Preserve lazy assertion messages, imported-root
  suppression, normal entry activation and unrelated dependency invalidation.
- [ ] Rebuild documentation and verify removed pages disappear from generated
  navigation/output. Verify ordinary templates, fragments, resources, static
  branches and imports still pass their contractual coverage.
- [ ] Record a post-removal baseline on the surviving workload cohort. Use this
  baseline to measure representation/folding work. Keep the pre-removal baseline
  to attribute retirement separately. Removed cases are not a storage speedup.

### Required Wiring documentation delivery

The permanent language route owns exact semantics. Complete the public Wiring
family with introduction, Wires, Routes and scope/future material, using Basic
teaching, unsuffixed detailed references and page composition according to the
site conventions. Keep precise source/runtime availability in the progress
matrix. The teaching work transferred here is part of delivery:

| Lesson | Required distinction/examples |
|---|---|
| Shared member-list forms | Function parameters, struct fields and choice payloads share a pipe-list shape but have different contracts. Named const-record values do not become runtime type declarations. |
| Head-directed `of` | Ordinary generic type application, Wire/Route parameter contracts and future Channel specialisation use one separator without becoming one inference rule. |
| `~` | Declaration mutability and operation/argument exclusivity retain their separate meanings. A Wire does not grant mutation authority. |
| Receiving context | Generic construction, `none`, casts and routes get only the information their immediate receiving contract supplies. No ambient higher-order inference or parse-failure retry. |
| Blocks and templates | Structural blocks and ordinary templates keep their own roles. A template creates no hidden Wire, Route or subscription. |

Include a reusable forwarding helper, captured row identity with a zero-input
event, an ordinary choice-based update, child-message lifting without route
composition and a labelled future channel receiver. Preserve the future spelling
`reader, writer Channel of AppMessage = Channel()` and directional endpoints.
An omitted initializer is only an explicitly hypothetical future form. Compile
executable examples through normal docs/tests and label future examples as such.
Do not claim unsupported route application or new anonymous-record semantics.
Keep one canonical contract owner after the public rollout: compose the permanent
reference or move it and update every route in the same checkpoint, without
retaining competing copies. Include educational compiler diagrams, template and
design-scope pages, editor resources and test-harness instructions in the delivery
inventory, alongside both progress matrices and generated navigation.

**Exit check:** searches and source/runtime coverage show no supported V1 owner
or renamed compatibility implementation. Surviving semantics remain covered.
The new baseline measures the feature set the storage redesign will preserve.

## Phase 2 - investigate representation, packing and lifetimes

**Consumes:** current census, two baselines and fixed semantic contracts.
**Produces:** selected layouts, issuing domains, release/freeze boundaries,
concrete internal interfaces and bounded migration tasks for Phases 3 and 4.

IDs are the chosen architecture. The investigation selects their efficient
physical form. Begin with a small safe Rust enum plus IDs/ranges as the readable
reference, then compare a small number of justified alternatives. Keep at most
three live candidates per decision. An additional idea may replace a candidate
when its expected benefit, invariant cost and discriminating test are recorded.
Do not keep permanent experiment flags or an exhaustive matrix of combinations.

Use other compilers as comparison inputs, not as a stage-order template. The
[rustc THIR reference](https://rustc-dev-guide.rust-lang.org/thir.html) describes
separate indexed expression/statement storage and short-lived typed bodies.
Compare those lifetime boundaries against Moth's longer-lived TIR views and
generic inputs while keeping Moth's immediate typing/folding. Zig's
[memory-layout investigation](https://ziglang.org/download/0.8.0/release-notes.html#Reworked-Memory-Layout)
and [AST owner](https://github.com/ziglang/zig/blob/master/lib/std/zig/Ast.zig)
provide compact node, side-data and columnar candidates. Verify current source
when borrowing an idea, then measure its Moth-specific access and construction
cost. Neither precedent proves the fastest layout for this compiler.

### Candidate decisions

| Decision | Candidates and evidence required |
|---|---|
| Hot node layout | Compact Rust enum versus typed tag/flags and fixed payload words. Compare actual aligned size, tag decoding, traversal, construction and type/fold throughput. |
| Record organisation | AoS versus limited SoA only where measured passes read a strict subset. Count all columns, edge stores, allocations and cache traffic, not only one hot array. |
| Result shape | Inline common zero/single forms and a typed range/ID for multiple types. Compare explicit enums and checked encodings without stealing unbounded TypeId bits. |
| Optional identities | `NonZeroU32`/niche-capable IDs where their issuing policy supports it. Test exhaustion and zero handling. Existing TypeId zero remains valid. |
| Variable payloads | Typed ranges for arguments, fields, map entries, structural pieces and cold lists. Measure small-list frequency before introducing any special inline form. |
| Failure/occurrence facts | Dense presence bits or optional fact IDs with sparse typed payloads. Measure co-occurrence and repeated lookup cost. Do not introduce a hash table for every sparse field. |
| Constants and literals | Separate occurrence metadata from content, direct constant handles and exact scalar payload storage. Compare neutral shared content versus one direct phase projection only where lifetime and retained-byte evidence justify it. |
| Construction and reclamation | Append-only owner/partition, safe local rollback, scratch-to-store promotion or justified compaction. Measure peak overlap and unreachable retained payloads after folds/substitution. |

Set a candidate byte ceiling from the census before implementation. It is a
decision aid, not permission to weaken semantics. Measure bytes per useful live
expression, including side stores, edges, cold facts, nested heap payloads,
reserved capacity, construction/freeze overlap and dead/replaced nodes. Include
destruction cost and peak resident memory. A smaller header with a larger live
graph has not won the memory comparison.

### Compact encoding constraints

- Preserve the locked exact eight-byte `SourceSpan`. Reuse its source identity
  and snapshot owner. Do not rescan source or reopen the span bit split here.
- Tag/flag/result codecs own typed constructors and accessors, checked bounds,
  reserved-bit rules and boundary tests. Keep raw masks out of semantic code.
  Any stolen payload bits require a proven bound and exact extension, or choose
  a larger straightforward form. Avoid varint/delta decoding on hot traversals
  without evidence sufficient to justify the extra work.
- Keep full integer extrema, float bits including negative zero, and exact Dec
  coefficients/scales. Numeric identity differs from physical carrier. Do not
  silently narrow a full-width value because its word also stores a tag.
- Closed implicit-failure causes may use a compact typed code/bitset instead of
  repeated static-slice descriptors if order and policy remain exact. Public
  `Error.code` stays U32. Existing failure summaries and origin-owned witness
  lists must not become repeated subtree scans.
- A fact is not absent merely because one vector is empty. Account for explicit
  exit, typed-producer conflicts, deferred catches, post-discharge folding
  eligibility and private-failure witnesses retained in inert expressions.
- Do not reopen already shared `Arc<BigInt>` coefficients by adding another
  sharing layer. Optional interning or caches require measured reuse and a
  complete identity/invalidation model, not only potential duplicate values.

### Lifetime decisions required before cutover

- [ ] For each expression, payload, default, constant and TIR view store, record
  its owner and last reader. Prove that every surviving ID has a live matching
  store. Reusing an integer in a sibling store does not authorise cross-access.
- [ ] Define how a fold that discards an operand releases or bounds its retained
  storage. Dropping an ID does not reclaim append-only payloads. Test repeated
  replacement and mostly-folded large bodies, not only live runtime trees.
- [ ] Preserve immutable overlay/root isolation between candidate instantiations.
  Structural TIR children inherit the accepted overlay, nested value expressions
  use their own context, and runtime dependence does not skip required validation.
- [ ] Separate throwaway generic-body validation storage from retained canonical
  tokens/declaration syntax and from concrete materialisation. Prove later
  instantiation after validation storage is dropped.
- [ ] Define AST/TIR-to-HIR ownership transfer and required failure fact snapshots.
  Base/generated recursive summary convergence cannot depend on a retained AST.
  Diagnostics retain exact locations/types/provenance after expression/TIR drop.
- [ ] Preserve the current diagnostic TypeEnv dependency until its own display
  snapshot boundary replaces it. This plan does not claim that shrinking
  expressions implements diagnostic layout Phase 4 or frees every TypeEnv early.
- [ ] Choose the native fallible-call HIR form: existing edge-defined/block-argument
  results or an explicit invoke-like terminator. Success and error definitions
  are mutually exclusive. Do not hide a semantic carrier behind an accessor.

### Prior evidence to use, not blindly repeat

These are historical observations before the Dec/Uint prerequisites. Refresh
the census and source owners on activation. They are not new benchmark claims
or fixed target sizes for this implementation.

- The reviewed 64-bit build had a 424-byte `Expression`: kind 88, TypeId 4,
  failure facts 168, diagnostic facts 56, receiver 8, value mode 1, SourceSpan 8,
  reactive source 8, reactive template 56, const-record flag 1, division flag 1,
  provenance 24 and padding 1. RPN items inherited the large expression stride
  even for operators. The earlier statement boxing fix reduced AstNode size but
  left nested calls sensitive to expression-parser frames.
- The source-span investigation compared 22/10 and 20/12 splits within a 0.1%
  tie, selecting 22/10. Its measured overflow used 249 rows/1,992 bytes across
  286,779 spans. A terminator-packing prototype was declined because the total
  prize was below 2 KiB. Use the same whole-workload reasoning here.
- The token AoS/SoA experiment accounted for 495,972 bytes over 41,331 tokens
  under equal capacity. A proxy traversal win did not prove a whole-compiler win.
- An owned-folder attempt passed correctness but measured +0.560% and +0.534%
  on the constant chain in five paired runs, with two 256-byte copies still in
  the inspected assembly. A different signature alone does not establish reuse.
- Substitution-key normalisation cost about 39–56 ns and at most 0.19% of the
  measured check time. The observed TIR cache had 1 hit in 11,275 attempts.
  These do not justify new key formats, wider caches or process-wide local IDs.
- The 32-byte diagnostic record and at-most-48-byte draft remain locked design
  contracts for later implementation. Borrow the census/codec/overflow method,
  not a claim that the pending diagnostic migration already improved runtime.

**Exit check:** record selected and rejected candidates, full memory/time
accounting, exact interfaces, invariants, remaining limitations and the evidence
that would justify reopening a decision. Remove losing code and temporary
measurement flags. A repeatable regression requires revision or an explicit
reported design tradeoff, not an automatic acceptance because it is below 5%.

## Phase 3 - complete dense HIR and capacity foundations

**Consumes:** selected store/range contracts and freeze/publication map.
**Produces:** the dense HIR checkpoint, with current semantics preserved and no
durable recursive expression/place representation.

- [ ] Make `HirValueId` the canonical module expression-store handle. Remove
  redundant expression IDs and migrate fixed children to IDs.
- [ ] Move variable expression payloads to typed ranges: arguments, collection
  elements, map entries, struct/variant fields, structural String pieces and
  place projections. Flatten `HirPlace` to root local plus ordered projections,
  with index expressions addressed by `HirValueId`.
- [ ] Migrate every durable carrier, including statements, terminators, patterns
  and nested records, in the same accepted cutover. Temporary lowering locals
  cannot become a retained compatibility graph.
- [ ] Convert lowering, validation, displays, rewrites, remapping, def/use,
  numeric proof tables, link facts, borrow/lifetime inputs, Boracle extraction,
  generated materialisation and JavaScript/Wasm consumers to direct store access.
- [ ] Preserve source locations, evaluation order, region/value facts, numeric
  domains, failure/conversion order and existing target rejections. Do not derive
  source numeric meaning from physical carriers during this migration.
- [ ] Extend `FrontendArenaCapacityEstimate` only where measured store growth
  justifies it. Gather cheap source facts in existing parallel preparation and
  existing header aggregation. Use naturally known local counts. Add no separate
  sizing-only scan or duplicate policy owner. Keep hard caps and saturating math.
- [ ] Track estimate, actual count, initial reserve, growth and retained capacity
  only where they answer allocation questions. Underestimation grows normally.
  Freeze completed arrays to fixed ownership without excessive peak overlap.
- [ ] Keep convergence-dependent rewrites before final publication. Structural
  freeze fixes topology/counts but may allow owning scalar remapping. After
  private lane installation or pruning, publish link facts for the final CFG,
  including unchanged base reuse and generated sidecars.
- [ ] Delete superseded visitors/constructors and add a narrow architecture check
  for reliably detectable forbidden durable recursive HIR ownership.

**Checks:** growth and empty/nonempty range access, flat field/index places,
remapping across base/generated stores, final-CFG validation/link facts, numeric
profile parity, affected borrow/Boracle and both supported backend lanes. Use
source contracts for user behaviour and owner-local tests for store invariants.
Run the affected frontend/end-to-end benchmark checks after the cutover.

**Exit check:** all current HIR consumers use one dense representation directly,
published stores keep no unnecessary growth capacity, and estimate quality has
no semantic effect. Keep unrelated outer HIR collections and broad borrow-fact
compaction outside this phase.

## Phase 4 - integrated typed-expression and native-result cutover

**Consumes:** Phase 2's final interfaces and lifetimes plus dense HIR.
**Produces:** the typed-expression/native-result checkpoint. Treat 4A–4D as
bounded review packages within one coupled accepted representation checkpoint.

### 4A. Semantic construction and values

- [ ] Separate parser/RPN scratch, declarations, signatures, missing defaults,
  places and constant content from completed semantic expressions. Preserve early
  nominal registration, contextual numeric materialisation and AST-local folding.
- [ ] Construct resolved nodes through the selected compact store API. Publish
  child IDs/ranges, canonical result shape, exact spans and required operation
  facts. Move sparse occurrence/failure facts to their selected owner.
- [ ] Replace fake `NoValue` declaration expressions and unresolved placeholder
  values at symbol/signature boundaries. Keep authored type/declaration syntax
  where diagnostics and later generic materialisation need it.
- [ ] Migrate constants, defaults, constructor fields, receiver operations,
  templates/TIR, static-if specialisation and generic validation/materialisation.
  Preserve one shared scalar/optional/aggregate value vocabulary and immutable
  roots. No-selection/no-substitution paths reuse their roots.
- [ ] Pass accepted TIR views into materialisation with their original context.
  Release discarded validation and replacement storage according to Phase 2,
  then project required semantic facts before the AST/TIR owner is dropped.

### 4B. One producer/result contract

- [ ] Replace the single `data_type` assumption with authoritative zero, single
  or ordered multiple success types and a separate error channel. Remove a
  call's duplicated resolved result lists. Signature shapes and instantiated
  call shapes remain separate where their meaning/lifetime differ.
- [ ] Route zero/one/multiple producers through ordinary calls, returns,
  multi-bind, value-producing `if`/`match`/`catch`, receiver calls, supported
  forwarding and implicit entry functions. A collection, record or optional
  remains one result. Zero results do not imply divergence.
- [ ] Materialise folded producers through ordinary single-valued constant IDs
  and the shared result receiver. A multi-result producer executes once. It
  creates no tuple type, first-class multi-result constant or hidden host-call
  cache. Existing constant multi-bind syntax gains no new source forms.
- [ ] Enforce arity and contextual coercion per slot. Single-target misuse gets
  the transferred value-receiver diagnostic family at its receiver, not the
  `InvalidMultiBind` family. Carry receiver kind, target count, produced slot
  count and authored producer location. Guidance cannot suggest `_`, implicit
  discards, fabricated binding names or a tuple. Never select slot zero or insert
  dummy values. Cover declarations, assignments, returns and nested `then`
  negatives, with matching multi-bind and already-legal forwarding positives.
- [ ] Complete the transferred postfix `?` multiple-success/context diagnostic
  work and its narrow enclosing-function facts. Preserve normal-root `start`
  returning through built-in `Error!`. Do not replace valid top-level error
  propagation with a blanket rejection.

### 4C. Channels, publication and backend boundaries

- [ ] Migrate HIR function signatures, infallible calls, returns and fallible
  calls to native ordered slots and the selected explicit continuation form.
  Success locals are defined only on success, the error local only on error.
- [ ] Migrate `hir/private_failure_lane.rs` and every inferred private-failure
  installer together. It must not manufacture the removed carrier shape after
  the main lowerer has switched. Preserve convergence, handler pruning and final
  validation/link-fact refresh for both base modules and generated sidecars.
- [ ] Join recovered values per slot through normal CFG/value targets. Propagate
  the error channel without tuple/payload extraction. Compute all RHS results
  before writing any existing multi-bind target, preserving swaps and aliases.
- [ ] Thread slot identity through walking, rewrites, source maps, dominance,
  def/use, summaries, lifetime/borrow metadata and external alias transport.
  Keep current conservative root unions where precision is unavailable. Fresh
  bindings and separate result slots do not prove distinct allocation origins.
- [ ] Rebuild public/generated projections with stable semantic identities.
  Remap local IDs through the owning handoff and resolve external identities in
  the consuming registry. No donor ExprId, TypeId, HIR ID or registry index leaks.
- [ ] Make JavaScript arrays/objects/tagged envelopes ABI choices only. Use
  supported native Wasm values and explicit unsupported-target diagnostics.
  Keep aggregate/memory/Dec limitations in their existing owners. Do not promise
  the later full Wasm implementation as part of this checkpoint.

### 4D. Coupled correctness and deletion gate

Complete these tests in the relevant owner before accepting the checkpoint:

| Contract | Required evidence |
|---|---|
| Store integrity | Growth preserves handles, typed ranges cover exact elements, empty ranges work, sibling-domain IDs cannot be confused, injected small limits exercise every compact overflow diagnostic without allocating billions of nodes. |
| Results and control flow | Zero results versus one optional `none` versus divergence, per-slot arity/coercion, same-typed slots with different values to expose swaps, exactly-once producers, all RHS before writes, success/error dominance, catch joins and valid/invalid propagation contexts. |
| Numeric fidelity | All current profiles and entry services preserve post-Dec/Uint literal rules, full integer extrema, F16/F32 rounding, signed zero, exact Dec payloads, numeric identity/carrier separation and U32 error codes. |
| Immutable views | Two instantiations with different roots cannot contaminate one another. Structural TIR children inherit the selected overlay, nested values use their own context, runtime-dependent paths still undergo required validation and handoff materialises that same view. |
| Static branches | Both authored branches validate. Inactive branches create no active concrete generic requests, emitted HIR/runtime capabilities or active resource uses, while graph-active resource validity/dependencies remain mandatory. |
| Generic lifetime | Release unused validation AST/store, then instantiate later from retained canonical syntax. Base/generated/sibling local ID collisions cannot select the wrong object. |
| Failure lifetime | Drop AST/TIR before base/generated recursive convergence. Retain origin-owned source order, skipped recursive backedges, bounded private witnesses including three private hops plus origin/elision facts and typed deferred-catch producer/handler locations. |
| Subtle failure facts | Known-true assertions fully validate messages and retain private witnesses in inert replacements. Preserve post-discharge catch folding such as U8 arithmetic widened to U32, typed conflicts and error code zero. Checked compound write-back priority applies to its own arithmetic, not unrelated RHS recovery. |
| Diagnostics | Render after expression/TIR release and source-file mutation using the captured source snapshot, donor labels, types and provenance. Retain the current required TypeEnv lifetime until the diagnostic owner changes. |
| Final HIR/publication | Validate final CFG, dominance and link facts after lane install, handler pruning and unchanged reuse in base and generated modules. Check supported JS/Wasm output through existing contractual owners. |

Use the existing one-MiB child-process dev-stack regression through `moth check`
as a starting owner. Add representative nested calls, parentheses, loops and
conditionals where they expose a distinct remaining risk. Measure native frames
and accepted/rejected limits. Compact IDs reduce recursive payload pressure but
do not prove every recursive compiler path stack-safe. Define bounded structured
rejection where an authored depth exceeds a supported limit. Do not raise CI's
stack budget or weaken fixtures to conceal a remaining failure.

**Exit check:** no completed semantic expression tree, placeholder declaration
value, legacy result carrier or re-expansion adapter remains. All affected
services use the selected stores, release rules and native slots. Meaningful
semantic/invariant coverage and current feature lanes pass. Wider validation
and performance evidence follow the shared gate below.

## Phase 5 - reduce repeated type and folding work

**Consumes:** final representations and updated profiles.
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
  and complete semantic keys justify them. Keep key lifetime, profile, config,
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
  `starts_with` and `ends_with`. Read their actual post-Uint signatures. Cover
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
