# Wiring V1: reactivity removal and semantic foundations

Intended repository path: `docs/roadmap/plans/wiring-v1-cleanup-and-foundations-plan.md`

## Status

- Status: queued, with the design interview accepted.
- Current slice: not started.
- Blockers: accepted source/token-layout Phase 3 completion and the intervening work selected by the maintainer must be merged into the activation branch.
- Next action: establish the current baseline, pause the compact-diagnostics migration before Phase 4 and start Phase 0 below.

## Purpose

Replace Moth's current Reactivity V1 with **Wiring**: stable state connections through `Wire of T` and constrained constructor routes through `Route of ...`.

This is a removal, documentation and frontend-foundation plan. It removes the obsolete runtime model rather than renaming it. It then implements the smallest coherent syntax, semantic and HIR foundation for the accepted replacement. It does not deliver an interactive UI runtime.

The user-facing goal is a small set of language primitives that make state and event wiring easier without general closures, source-visible reference types or a prescribed rendering architecture. Ordinary templates, choices, functions and exclusive access remain the building blocks around these primitives.

A successful implementation leaves:

- one accepted Wiring reference and a truthful implementation-status boundary
- a closed, head-directed meaning for `of` that includes existing generic application
- contextual wire and route arguments without new call-site operators
- validated wire identity and route-construction facts in the normal compiler pipeline
- ordinary strings and templates without hidden reactive dependencies
- no obsolete reactive declarations, subscriptions, scheduling, mounting or compatibility paths
- corrected future channel examples and concrete teaching material about Moth's reused syntax

## Sequencing and activation

This work is a hard interlude between the source/token-layout migration's Phase 3 and Phase 4.

```text
Source/token-layout work through Phase 3
    -> maintainer-selected intervening work and merges
    -> Wiring removal, documentation and V1 semantic foundation
    -> source/token-layout Phase 4 and remaining layout work
    -> the separately paused diagnostics-improvement work
```

Phase 3 provides fixed tokens, source-owned token stores and retained syntax ranges. Phase 4 replaces diagnostic storage and schema machinery. Removing obsolete reactivity before Phase 4 prevents that migration from rebuilding a discarded diagnostic and metadata model.

The separate diagnostics-improvement work remains paused until the complete layout migration finishes. This plan does not resume it early or absorb its remaining work.

At activation:

1. Verify the Phase 3 exit criteria against the actual merged tree, not a branch name or an old progress note.
2. Record the activation revision, worktree and baseline results in working notes. Do not put a speculative baseline SHA into this queued plan.
3. Update roadmap sequencing so the layout work explicitly waits before Phase 4 until this interlude lands.
4. Adapt to any intervening native-result-slot, const-evaluation, package or HIR changes already present. Their exact scope is established from the activation tree. This plan neither implements them nor restores older representations.
5. Keep the existing post-Phase-3 source, token and diagnostic ownership boundaries. New diagnostics use the diagnostic API that actually exists at activation. This plan does not start the compact-diagnostics migration itself.

The roadmap owns ordering. Name prerequisites and resume conditions in neighbouring work items rather than linking temporary plans to one another. Publish shared semantic contracts in permanent authorities.

Suggested entry in `docs/roadmap/roadmap.md`, placed before the separately paused diagnostics-improvement entry and accompanied by the explicit Phase 3/4 interlude note:

```markdown
- [Wiring V1: reactivity removal and semantic foundations](./plans/wiring-v1-cleanup-and-foundations-plan.md) - Queued interlude after source/token-layout Phase 3 and maintainer-selected merges, before compact-diagnostics Phase 4
```

Delete this plan and its roadmap entry in the commit that completes the work. The final status and remaining deferred work belong in permanent documentation and the progress matrix.

## Authority and scope control

The accepted user interview changes the old reactivity contract. Phase 1 publishes those changes into the canonical references before implementation. Thereafter, canonical references own semantics and this plan owns delivery order.

Read at activation and whenever the affected domain changes:

| Authority | Required use |
|---|---|
| `AGENTS.md` | Current reading routes, ownership rules and Slice review |
| `docs/src/developer-docs/style-guide/style-guide.mtf` | Full implementation and documentation style |
| `docs/src/developer-docs/style-guide/testing.mtf` | Full test ownership and pruning policy |
| `docs/src/developer-docs/style-guide/validation.mtf` | Current commands, feature lanes and completion gates |
| `docs/compiler-design-overview.md` | Full compiler architecture and affected stage handoffs |
| `docs/build-system-design.md` | Full build architecture, target contracts and fragment assembly |
| `docs/compiler-data-layout-design.md` | Delivered source/token invariants and the current migration boundary |
| `docs/src/developer-docs/memory-management/overview.mtf` and routed leaves | Access, retained data, identity, lifetime topology and physical planning |
| `docs/src/developer-docs/language/overview.mtf` | Routing to canonical unsuffixed language references |
| `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` | Current source-writing orientation |
| `docs/src/docs/progress/@page.moth` | What is implemented and on which targets |
| `docs/roadmap/roadmap.md` | Activation order and plan maintenance rules |
| `index.md` | Navigation only, with paths rechecked against the actual tree |

Exact language references include the existing `reactivity/` family before deletion, `generics/`, `functions/`, `choices/`, `bindings/`, `templates/`, `constants/const-records.mtf`, `design-scope/` and `async/@page.moth`. Read the new `wiring/` family after it is published.

The review requirements below incorporate the supplied `moth-code-review-guide.md` deletion, duplication and abstraction checks. Execution does not depend on that attachment. Use current `AGENTS.md`, the style guide and `index.md` to locate and review the actual owners.

### Confirmed interview decisions

| Decision | Locked result |
|---|---|
| Sequencing | Hard interlude after source/token-layout Phase 3 and the selected merges, before Phase 4 |
| Old syntax | Strict removal without migration-specific compatibility diagnostics |
| V1 wire operands | Named local bindings and existing wire parameters. Mutable or immutable locals are eligible. Projections and fresh expressions are outside V1 |
| Wire forwarding | Immediate receiving context selects ordinary reading or identity forwarding. No call-site marker |
| Route formation | Direct choice constructors with a captured leading payload prefix and future trailing payload fields |
| Route use | Parameter acceptance and unchanged forwarding. No ordinary storage, return, comparison, channel transfer or route composition |
| Implementation depth | Frontend semantic foundation through validated HIR, with only necessary minimal lowering or explicit target rejection |
| `of` | Closed, head-directed specialisation. No user-extensible specifier language |
| Channels | Future syntax is `reader, writer Channel of AppMessage = Channel()`. Async implementation and initialiser omission are outside this plan |
| Teaching | Concrete shared-syntax examples in a general language page and the cheatsheet, in addition to the new Wiring reference |

## Accepted language contract to publish

### 1. Terminology

**Wiring** is the feature family. A **wire** preserves the continuing identity of a binding. A **route** describes how future input completes a known choice constructor. A **message** is an ordinary choice value. A **channel** transports ordinary values between async activities.

A route has no destination by itself. Constructing a message, delivering a message and sending a value through a channel are different operations.

Use these names in source documentation and diagnostics. Use precise internal names for binding identity, route signature, constructor identity and captured arguments. Avoid calling the new model reactive, a signal system, a callback system or a live-string system.

### 2. Head-directed `of`

The common surface is:

```moth
Box of String
Wire of String
Route of String -> FormMessage
Route of FormMessage
Channel of AppMessage
```

`of` introduces a specification whose accepted shape is determined by its head. Sharing this spelling does not make all five forms ordinary generic types.

| Head | Specification | Semantic result |
|---|---|---|
| A user generic nominal such as `Box` | Existing concrete type arguments | An ordinary specialised semantic type |
| `Wire` | One underlying value type | A parameter contract preserving binding identity |
| `Route` | An input/output relation, or only an output for a zero-input route | A constrained constructor-route parameter contract |
| `Channel` | A carried value type | A future typed channel construction/endpoint contract |

Preserve existing generic arity, inference, alias and nested-`of` rules. This work does not introduce explicit generic call arguments, arbitrary inline nesting, higher-kinded types or type values.

Use a named ordinary concrete alias when an underlying type needs a nested specialisation that the existing grammar rejects. Do not permit `Box of Wire of T`, route-valued collections or other first-class capability containers by treating every head as a type constructor.

`Wire` and `Route` are compiler-owned heads. Integrate their recognition and collision handling with the existing compiler-owned name policy. User declarations cannot override their meaning or register new `of` grammars. Channel syntax remains documented future design rather than a reason to build async now.

### 3. Wires

A source function parameter may require a wire:

```moth
read_name |value Wire of String| -> String:
    return value
;

forward_name |value Wire of String| -> String:
    return read_name(value)
;

name ~= "Priya"
label = forward_name(name)
```

These are foundation examples, not promises of automatic UI updates.

The immediate parameter contract determines the use:

- An ordinary `T` receiver reads the current value under ordinary Moth access rules.
- A compatible `Wire of T` receiver preserves the same binding identity.
- A mutable receiver still needs ordinary exclusive authority. A wire grants none.

V1 rules:

- A named local binding can acquire identity when a wire-receiving call demands it. No special declaration is needed.
- Both mutable and immutable locals can be wired. Wiring an immutable local does not make it mutable.
- An existing wire parameter can be forwarded to another compatible wire parameter.
- A field projection, collection lookup, literal, constructor call or computed expression cannot directly satisfy a V1 wire parameter.
- An ordinary value parameter does not preserve its caller's binding identity. Do not infer a wire contract by inspecting its function body.
- Identity belongs to the binding instance, not its current scalar contents, allocation address or source-file position.
- Reassigning an eligible mutable local preserves that local binding's identity. Repeated function calls or loop executions create distinct local instances where ordinary binding semantics do so.
- Creating an ordinary local from a wire reads its value. It does not silently create a first-class wire or an alias for wire identity.
- An ordinary read is not a deep copy. Existing alias and `copy` rules still apply, especially for aggregate values.
- Returning `T` read from a wire returns an ordinary value. Returning a wire capability is outside V1.
- Wire identity is not an active long-lived value borrow. Actual reads, captured values and mutations still pass normal borrow validation.
- Identity metadata does not prove storage outlives a consumer. It cannot make an otherwise invalid escape legal.

V1 introduces no storage of wires in fields, choices, collections, maps, aliases, defaults or ordinary return slots. There is no `Wire()` value constructor, use-site `wire(...)`, `<value>`, `<>value` or `$` wiring marker.

Persistent observation, source-change callbacks, UI ownership and cross-task wire retention are not implemented by this foundation.

### 4. Routes

Required V1 parameter forms are:

```moth
on_click Route of FormMessage
on_input Route of String -> FormMessage
messages Route of EditorMessage -> AppMessage
```

The first has zero future inputs. The second and third each have one. The implementation acceptance target is these confirmed zero-input and single-input forms. Multi-input syntax explored earlier is not a requirement of this cleanup and must not be presented as implemented without a separately specified grammar and coverage.

`->` belongs to a route contract only after a `Route of` head inside that parameter. The ordinary return arrow after the closing function parameter list keeps its existing meaning.

A direct choice constructor is contextualised only at an immediate route-receiving boundary:

```moth
FormMessage ::
    NameChanged | value String |,
    Submit,
;

TodoMessage ::
    Rename | id Int, value String |,
    Delete | id Int |,
;
```

| Expected contract | Supplied argument | Meaning |
|---|---|---|
| `Route of FormMessage` | `FormMessage::Submit` | Construct the unit variant for a future zero-payload event |
| `Route of String -> FormMessage` | `FormMessage::NameChanged` | Future String fills the sole payload field |
| `Route of String -> TodoMessage` | `TodoMessage::Rename(todo_id)` | Retain the leading ID and leave the trailing String for future input |
| `Route of TodoMessage` | `TodoMessage::Delete(todo_id)` | Retain the ID for a future zero-input construction |
| `Route of EditorMessage -> AppMessage` | `AppMessage::Editor(editor_id)` | Retain the ID and leave the child message as future input |

Captured arguments fill a contiguous leading payload prefix. Future input fills the remaining trailing fields in declaration order. There are no holes, named omissions or arbitrary payload reordering for partial routes. A fully supplied zero-input constructor may use the existing complete-constructor argument rules, including legal named arguments.

The root route expression is a direct choice variant path/application or an unchanged compatible route parameter. It is not an arbitrary expression producing a callable or message. Captured constructor arguments are ordinary expressions evaluated at formation, under normal argument-order, error and ownership rules. They are not saved executable expressions to rerun on a later event.

Outside a route boundary, incomplete construction remains an ordinary constructor error:

```moth
message = TodoMessage::Rename(todo_id)
```

No separate `|>`, angle-bracket or `route` expression form is added.

Additional rules:

- Messages remain ordinary nominal choices. No special Message base type or trait is introduced.
- Route contracts retain exact input and output type identity. No variance, callable-trait or higher-order inference system is added.
- A generic helper may use `Route of String -> Message` with ordinary concrete generic inference. The supplied constructor establishes the concrete choice output. This does not require treating the constructor as a first-class function.
- Route parameters can be forwarded unchanged to compatible route parameters.
- Ordinary local assignment, collection/field storage, return, comparison, serialisation and channel transfer of a route are outside V1.
- A normal function name, closure, arithmetic expression or arbitrary block cannot stand in for a route.
- Applying a route as a general function and composing routes are outside this plan. In particular, do not implement `outer_route(InnerMessage::Changed)` as an implicit composition operator.
- A route is a mapping recipe, not an inbox, destination, event registration or implicit send.
- Captured runtime values remain runtime values even when the constructor and route signature are statically known. Static metadata is not permission to erase capture evaluation or retention.

### 5. Templates and runtime scope

Ordinary templates continue to produce ordinary String values. Returning a template, passing it through a String parameter or assigning it does not carry a hidden wire, route, subscription or rerender recipe.

Preserve template composition, runtime control flow, named slots, wrappers, formatting, compile-time folding and ordinary fragment insertion. Preserve structural resource/path metadata where those independent language contracts require it. Removing reactive metadata is not permission to flatten every structural String representation.

Wiring does not automatically update a text node, register an input handler or give a template a persistent lifetime. Those operations require a later explicit package/builder contract.

The V1 implementation target is source semantics through validated HIR. Minimal existing backend machinery may be adapted when it faithfully represents the accepted synchronous foundation. Reachable capabilities needing an unimplemented runtime contract receive a structured target/deferred-feature diagnostic before lowering, never a silent value-only downgrade.

Before implementation is called complete, record the exact supported frontend and backend matrix. Frontend-positive tests can use the existing compiler-owned test entry points to inspect validated HIR. They must not be mislabeled as successful executable UI tests or used to bypass the target checks performed by public commands.

No scheduler, automatic invalidation, observation API, route application runtime, event queue, mount lifecycle, DOM renderer or channel bridge is required or authorized by this plan.

### 6. Future channels

The accepted future declaration is exactly:

```moth
reader, writer Channel of AppMessage = Channel()
```

The annotation supplies the carried type. The value-position constructor is `Channel()` and receives its type information from the immediate receiving contract. This does not make `AppMessage` a runtime value or permit types as arbitrary call arguments.

The declaration receives two endpoints of one channel, reader first and writer second. The shared annotation describes the carried message type, not two independently constructed channels or two automatically bidirectional endpoints. The future async implementation must retain endpoint direction.

Replace inconsistent examples using `Channel(AppMessage)`, `Channel[AppMessage]` or `Channel<AppMessage>` as the recommended source form. This plan does not implement channels, endpoint passing, suspension, buffering, cancellation, close behaviour, select or host-event ingress.

Rewrite or remove larger draft examples whose endpoint-parameter spelling is not yet specified. Do not invent a complete endpoint type system from the shared construction annotation.

Keep the accepted memory rules: explicit send transfers an independent graph, source wire identity is not channel data and route formation does not perform a hidden send. Preserve `>>`, `<<`, `spawn`, `yield` and structured async scope discussion as deferred async design where it remains accurate.

Initializer omission is a separate open design possibility:

```moth
reader, writer Channel of AppMessage
```

Mark this example as hypothetical and outside current syntax. This plan does not add zero-value initialisation, assume all types have defaults or generalize a two-endpoint receiving annotation to unrelated multi-bind forms.

## Non-goals

The following stay outside delivery:

- implementing channels or designing the rest of async
- fields and collection positions as wire operands
- general route application or composition, callable values and closures
- first-class capability storage or public capability aliases
- new traits, effect solvers or user-defined `of` semantics
- persistent observation, derived state graphs, change handlers or two-way binding
- automatic live templates, UI components, event ingress, widget IDs or rendering policy
- new release optimisers, cache frameworks or a profile-dependent language
- the remaining compact diagnostic representation migration
- blanket zero-value initialisation or constructor omission

The discarded reactive follow-up list is not an inherited promise to implement these features later. Preserve only the new model's explicit deferred boundaries and genuine open questions.

## Removal and reuse map

These are inspection starting points, not an exhaustive or activation-pinned inventory. Follow current callers, re-exports, generated-function paths and tests before editing. Paths may move during intervening work.

| Area and current navigation | Action | Preserve or replace with |
|---|---|---|
| `src/compiler_frontend/tokenizer/`, `declaration_syntax/`, `headers/` and `keywords.rs` | Remove old reactive marker recognition, declaration classifications and subscription tokens that exist only for that model | One shared declaration-shell path, delivered fixed token layout and normal directive lexing |
| `src/compiler_frontend/ast/templates/template_head_parser/reactive_subscriptions.rs` | Remove | Ordinary head expressions and directive parsing |
| `src/compiler_frontend/ast/templates/reactive_template_metadata/` | Remove the reactive-only reducers | TIR and owned-handoff traversal still required by ordinary template/resource contracts |
| `src/compiler_frontend/ast/module_ast/finalization/reactive_templates/` | Remove reactive String flow collection, annotation and return-metadata fixed points | Existing ordinary value, result and resource flows. No replacement Wiring pass unless a real foundation fact needs it |
| `src/compiler_frontend/ast/expressions/expression.rs`, call arguments, signatures and declarations | Split old identity facts from obsolete template facts, then delete discarded fields and consumers | Small explicit wire and constructor-route contracts at their real owners |
| TIR node/store side tables and `runtime_handoff.rs` | Remove subscription annotations and reactive propagation dimensions | Exact TIR identity, static selection, slot ownership, resource anchors and ordinary handoff invariants |
| `src/compiler_frontend/hir/reactivity.rs`, `hir_builder/reactivity.rs` and `hir_side_table.rs` | Remove template IDs, dependency lists and reactive classifications. Adapt only useful binding-identity relationships | Minimal HIR wiring facts without TIR ownership, backend render plans or a new value-type hierarchy |
| HIR validation, remapping, visitors and reachability | Remove obsolete fields and branches end to end | Explicit validation and remapping of new capability arguments and captures |
| `src/compiler_frontend/analysis/borrow_checker/types.rs`, `transfer.rs`, `transfer/access.rs` and `transfer/access/statement.rs` | Remove reactive invalidation facts and scheduler-oriented write recording | Ordinary alias, access, last-use, mutation and lifetime facts. New wire reads cannot weaken these |
| `src/compiler_frontend/public_interface/`, canonical identities, module artefacts and generated materialisation | Remove old reactive summaries and stale signature assumptions | Canonical parameter contract kinds and underlying types where the new foundation requires them |
| `src/backends/js/runtime/reactivity.rs` | Remove reactive scheduling, registered fragments, dependency maps, template snapshot objects and reactive mount helpers | Ordinary binding/read helpers only where they still have real consumers |
| `src/backends/js/emitter.rs`, `value_use.rs`, `js_expr.rs` and callers | Remove reactive helper-demand scans and live-String preservation branches | Ordinary value lowering, including correct lazy assertion messages and string/resource handling |
| `src/projects/html_project/js_path.rs` and `src/projects/html_project/tests/js_path_tests.rs` | Remove reactive-vs-plain mount dispatch and reactive registration | One ordinary fragment insertion path preserving order and entry activation |
| `src/backends/backend_feature_validation.rs` and per-function link facts | Remove old reactive sink/backend gates | Existing target-contract mechanism for any genuinely unsupported new Wiring execution |
| `src/backends/js/tests/reactivity.rs`, AST/HIR reactive tests and borrow-checker reactivity tests | Delete obsolete expectations and retarget only distinct surviving contracts | New semantic tests for identity, captures, forwarding and rejection boundaries |
| `tests/cases/manifest.toml`, reactive case directories and goldens | Classify, remove or rewrite each affected case | One primary owner per surviving behaviour, with backend-specific expectations where necessary |
| Runtime-output harness, test support and testing docs | Remove reactive-only observation hooks, rerender expectations and microtask waits with no remaining purpose | Shared console/fragment output ordering, exact-once assertions and unrelated async support |
| `packages/`, examples, benchmarks, fixtures and documentation output | Find indirect users and generated assumptions | Ordinary snapshot examples or correctly labeled future Wiring examples |

### Reuse rule

Reuse a piece only when its current responsibility still exists in the new model. Record what it owns, who consumes it and why the ordinary subsystem cannot already provide that fact.

A source-identity table is a candidate for adaptation. A source-to-fragment dependency graph is not. A normal binding-slot helper can remain useful. A microtask dirty scheduler does not become useful merely by being renamed.

Keep shared behaviour local to its owning subsystem. Delete obsolete branches in mixed helpers instead of replacing every helper wholesale. Do not retain dead types, compatibility aliases, placeholder registries or disabled modules for a later phase. If a useful algorithm has no consumer during removal, recover and adapt it from Git when its real consumer is implemented.

Use a wiring-specific internal ID name if an additional ID is necessary. The delivered source database already has `SourceId` for source files. Binding identity must not be confused with file identity or implemented as a raw path/string key.

### Protected neighbouring behaviour

This cleanup must preserve:

- ordinary immutable reads, mutable calls, alias exclusivity and explicit `copy`
- ordinary return provenance and any merged native-result-slot contracts
- compile-time templates, runtime templates, named slots and wrapper composition
- structural resource-bearing strings and output path resolution
- lazy assertion-message evaluation and inactive static-branch validation
- checked numeric operations and Float formatting, including removal of only their obsolete reactive branches
- ordinary collection/map mutation and string-key normalisation
- entry-selected `start`, imported-root suppression, fragment ordering and exactly-once activation
- canonical module interfaces, generic instantiation and deterministic ID remapping
- normal JavaScript binding/reference helpers still needed by the existing ABI
- independent compiler caches, dependency invalidation, dev-server events and async infrastructure

Broad terms such as `source`, `binding`, `dependency`, `snapshot`, `slot`, `channel` and `invalidation` have legitimate meanings elsewhere. Audit their context rather than performing blind global replacement.

## Documentation delivery matrix

New file names below are proposed destinations. Keep this compact structure unless the current docs organisation has changed, in which case move the content into the corresponding existing owner without duplication.

| Owner | Required change |
|---|---|
| `docs/src/docs/wiring/@page.moth` | New Wiring introduction, navigation and Basic/Advanced composition. Explain stable state connections and constructor routes without promising a UI runtime |
| `docs/src/docs/wiring/wires.mtf` and `wires-basic.mtf` | Canonical wire contract and a short named-local/forwarding lesson |
| `docs/src/docs/wiring/routes.mtf` and `routes-basic.mtf` | Canonical route contract, zero/one-input examples, prefix capture and unchanged forwarding |
| `docs/src/docs/wiring/scope-and-lifetimes.mtf` and paired Basic file | Non-first-class restrictions, ordinary alias rules, frontend/runtime split and channel boundary |
| `docs/src/docs/reactivity/` | Remove the old family. Replace imports and navigation. Do not retain a second current reference or migration tutorial |
| `docs/src/docs/language-overview/contextual-forms.mtf` and paired Basic file, composed by the existing page | Concrete general lesson described below. This is the main teaching owner for reused forms beyond `of` |
| `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` | Replace Reactivity with a concise Wiring summary, correct common translations and add a compact contextual-forms table |
| `docs/src/docs/generics/type-application.mtf` and related Basic/limits pages | Explain generic application as one use of `of`. Preserve generic inference, arity and nesting limits |
| `docs/src/docs/functions/` and `bindings/` | Add concise receiving-contract cross-references. Keep ordinary read and exclusive-access rules in their existing owners |
| `docs/src/docs/choices/variant-construction.mtf` | Describe the immediate Route-boundary exception and link to its canonical owner. Ordinary constructors still require complete arguments |
| `docs/src/docs/templates/` and `moth-templates/` | Remove live-String/subscription claims. Preserve normal string/template and compile-time `.mtf` rules |
| `docs/src/docs/async/@page.moth` | Normalize to the accepted typed multi-bind and `Channel()` constructor. Keep async deferred and initialiser omission hypothetical |
| `docs/src/docs/design-scope/` | Replace closure-alternative claims based on reactivity. Keep general closures and broad function-value systems outside scope |
| `docs/src/developer-docs/language/overview.mtf` | Replace the Reactivity reading route with the new canonical Wiring files |
| `docs/compiler-design-overview.md` | Replace reactive boundary, metadata, summary and target-handoff wording with the actual Wiring foundation and deleted runtime boundary |
| `docs/build-system-design.md` | Remove reactive mount ownership and old live-sink claims. Preserve ordinary assembly and general lifecycle-topology requirements |
| `docs/src/developer-docs/memory-management/**` | Remove obsolete reactive invalidation/subscription contracts. Preserve mandatory topology, ordinary reads, result provenance and independently justified lifecycle roots |
| `docs/src/developer-docs/compiler-design/**` | Update educational diagrams and TIR/HIR explanations that still imply reactive String flow |
| `docs/src/developer-docs/style-guide/testing.mtf` | Remove obsolete reactive runtime-harness instructions and describe only the retained output behaviour |
| `docs/src/docs/progress/@page.moth` | Remove obsolete support rows. Distinguish Wiring semantic foundations from deferred runtime consumers and future channels |
| `README.md`, `AGENTS.md`, `index.md`, docs navigation and existing teaching examples | Remove stale claims and paths. Keep contributor routing and module ownership accurate |
| `docs/roadmap/roadmap.md` and affected still-live work items | Insert the interlude and remove discarded follow-up promises. Update downstream diagnostic and backend assumptions without broadening their scope |
| Editor grammar/highlighting resources present in this repository | Remove old reactive tokens/snippets and account for the new contextual heads through their existing owners |
| `docs/release/**` | Rebuild through the compiler. Verify stale output pages disappear through the normal output mechanism. Never hand-edit generated HTML |

Closed audits and dated benchmark evidence are historical records. Preserve their meaning and mark affected audit coverage stale under the existing policy. Update current claims and live work, not historical evidence to make a text search look empty.

### Required general teaching lesson

Title the general lesson along the lines of **Shared forms, clear context**. Teach through concrete code before introducing terms such as head-directed specialisation. The lesson must cover more than Wiring.

#### A. One pipe-delimited member-list form

Show how the owner determines the meaning of `|...|`:

```moth
-- Function parameters
show |name String, age Int| -> String:
    return [: [name] is [age].]
;

-- Struct fields
Person = |
    name String,
    age Int,
|

-- Choice payload fields
PersonMessage ::
    Rename | value String |,
    Birthday,
;

-- Named compile-time values
labels #= |
    name = "Name",
    age = "Age",
|
```

Explain the shared member-list shape and its different contracts. The const record requires values that fold. The struct declares a constructable type. Choice payload fields retain their own restrictions. Reuse of punctuation does not erase those differences or authorize nested pipe lists and anonymous runtime records.

#### B. One `of` specialisation separator

Show ordinary generic application beside Wiring and future channels:

```moth
Box type Item = |
    value Item,
|

name_box Box of String = Box("Priya")
```

Then show these as parameter/annotation shapes, not standalone runtime expressions:

```text
value Wire of String
on_input Route of String -> FormMessage
on_click Route of FormMessage
reader, writer Channel of AppMessage = Channel()
```

Mark the channel line as future syntax. Explain which forms are ordinary types and which are parameter capabilities. `of` does not turn types into runtime values or let users define new specifier families.

#### C. The same `~` distinction in declarations and operations

```moth
increment |count ~Int|:
    count += 1
;

count ~= 0
increment(~count)
```

Explain mutable binding permission versus exclusive access for an operation. `Wire of T` is read-only identity preservation, not an alternate mutable reference or an implicit `~`.

#### D. An immediate receiving contract supplies missing information

Use the existing canonical examples for ordinary generic construction, typed `none` and `cast` alongside contextual route formation. For example:

```moth
name_box Box of String = Box("Priya")
maybe_name String? = none
text = "42"
amount Int = cast text catch then 0
```

Explain what the immediate receiver determines in each case. Do not claim the mechanisms have identical inference rules or introduce inference from distant function bodies.

#### E. Structural blocks and ordinary templates

Briefly show that `:` and `;` delimit a body owned by a function, branch or loop. They do not imply a general anonymous block feature. Show ordinary template interpolation as value construction, distinct from Wiring:

```moth
name = "Priya"
message = [: Hello [name].]
```

`message` is an ordinary String. It does not become live when `name` is later wired elsewhere.

### Required Wiring teaching examples

Keep the Basic pages short. Put exact rules and errors in Advanced pages. Each example must be labeled as one of current ordinary Moth, V1 semantic-foundation syntax or future UI integration. Compile executable examples under their real support boundary. A highlighted code block in a docs build is not evidence that its example executes.

#### 1. A reusable forwarding helper

This is a future UI-package illustration using the accepted V1 signature shapes. `ui` is illustrative, not a new package delivered by this plan.

```moth
FormMessage ::
    NameChanged | value String |,
    Submit,
;

labelled_input type Message |
    label String,
    value Wire of String,
    on_input Route of String -> Message
|:
    ui.label(label)
    ui.text_input(value, on_input = on_input)
;

name ~= "Priya"
labelled_input("Name", name, on_input = FormMessage::NameChanged)
```

Explain that `name` is a named mutable local. The wrapper forwards its wire contract and its route contract without call-site punctuation. Neither grants mutation to the control. The example does not choose an event destination or implement event delivery.

#### 2. Captured row identity and zero-input events

```moth
TodoMessage ::
    Rename | id Int, value String |,
    Delete | id Int |,
;

todo_id = 42
title ~= "Review the design"

ui.text_input(title, on_input = TodoMessage::Rename(todo_id))
ui.button("Delete", on_click = TodoMessage::Delete(todo_id))
```

Again, label the UI API as future integration. `todo_id` is captured at formation. The input event supplies the future String. The click supplies no value. A click route using incomplete `Rename(todo_id)` is invalid because a zero-input route cannot supply the missing String.

#### 3. Messages and updates are already ordinary language concepts

```moth
CounterMessage ::
    Increment,
    Reset,
;

update_counter |count ~Int, message CounterMessage|:
    if message is:
        Increment => count += 1
        Reset => count = 0
    ;
;

count ~= 0
update_counter(~count, CounterMessage::Increment)
label = [: Count: [count]]
```

This example needs neither a channel nor a UI runtime. A later application can receive messages and call the same function. Routes describe construction, not the update operation or its scheduling.

#### 4. Child-message lifting without claiming composition

Show `AppMessage::Editor(editor_id)` fitting `Route of EditorMessage -> AppMessage`. Explain unchanged forwarding separately. Explicitly state that composing it with an input-to-child route is deferred. Do not sneak a composition operator into a teaching helper.

#### 5. A future channel receiver

```moth
$async:
    reader, writer Channel of AppMessage = Channel()
;
```

`$async:` is a compiler-owned directive block, not builder metadata. No async
implementation is delivered by this Wiring plan.

Use this only to teach the declaration and endpoint roles, not as a working async program. Surrounding text may describe future explicit send/receive and message-driven updates, but this plan supplies no tasks, event bridge or automatic dispatch.

## Compiler architecture constraints

### Parsing and retained syntax

Extend the current parameter/declaration grammar. Do not add a second scanner, parser or generic specifier framework.

- Tokenization remains responsible for lexical forms and owned token storage.
- Declaration-shell parsing retains the syntax of parameter contracts using the delivered token ranges and cold-store conventions.
- AST resolution decides whether a head is a generic type, wire contract or route contract. Tokenization does not query semantic types or resolve constructors.
- The shared call-argument parser still owns delimiters, named arguments and parameter-slot association.
- Route-aware validation operates from the immediate parameter slot before ordinary complete-constructor checking would reject an intentional trailing omission.
- An ordinary constructor call never retries as a route after a diagnostic. No parse-fail-and-fallback path is permitted.

Test a route between ordinary parameters and a function return arrow after the closing `|`. Treat a second route arrow, invalid nested application and missing route output as structured source errors. Preserve existing generic argument-list boundaries.

### AST, HIR and signatures

Keep underlying semantic types separate from parameter contracts. `Wire of String` is not a new String `TypeId`. A route input/output contract is not an ordinary callable type, but it still needs exact type checking and representation in signatures.

Represent only facts required by the foundation:

- wire origin/forwarding relationships and the underlying value type
- route input/output contracts
- the resolved choice and variant identity
- captured argument expressions/values in their normal evaluation order
- the future payload field range and source locations required for diagnostics
- explicit capability argument kinds where ordinary value arguments are insufficient

Do not disguise a route argument as a normal Message value or expose TIR to HIR consumers. Do not widen every ordinary expression with a large permanent payload for a rare feature. Reuse current compact side-table or parameter-contract owners where justified.

Use the activation tree's result-slot, call-argument and value-production representation. Captured expressions must integrate with that representation rather than reconstructing retired tuple/result carriers.

### Public and generated boundaries

A function accepting `T`, `Wire of T` or `Route of A -> B` has a different contract even when some underlying types match.

Preserve this distinction through header binding, exported signatures, public-interface fingerprints, import projection, generic materialisation, HIR call validation and remapping. Ordinary module boundaries must not silently turn wires into snapshots or routes into messages.

Use canonical declaration/type identities and parameter positions for cross-module information. Keep local wire IDs, HIR locals, capture storage and source-file IDs within their own identity domains. Closed signature facts belong in the normal interface owner, not a new Wiring package cache or interprocedural String-flow system.

Existing generic helpers are part of the intended foundation. Test them through normal materialisation rather than adding special Wiring monomorphisation.

### Borrowing and retained captures

Use normal access and lifetime analysis for actual reads and captured data. Retaining binding identity does not capture a mutable borrow, while reading a non-scalar value may create ordinary aliases with real lifetime consequences.

For V1 parameter forwarding, no external persistent consumer is introduced. Any route or wire escape requiring a new lifecycle/retention contract remains rejected or deferred. A JavaScript garbage collector cannot be used as evidence that such a source program is legal.

Remove obsolete reactive facts from Boracle extraction, fixtures, dumps and commentary where they exist. Preserve ordinary borrow/problem facts and existing feature-lane coverage. Reuse current analysis entry points rather than adding a Wiring-specific borrow checker.

### Runtime and optimization boundary

Foundation semantics are identical in debug and release. Correctness, access checks, capture evaluation and lifetime validation are mandatory in both.

Later physical lowering may choose a simple debug representation and more expensive release analysis. Candidate optimizations include eliminating unused wiring, specialising known constructor adapters and removing redundant representation. These are documentation boundaries, not new optimization passes in this plan.

A backend cannot erase the effects or failure of captured argument expressions, change message ordering, add hidden suspension or turn a stable identity into a snapshot merely because that produces simpler code. Any future render-work skipping also needs its own effect-safe contract.

Keep ordinary no-Wiring programs free of new runtime helpers and unconditional Wiring passes. Delete obsolete passes instead of replacing each one with a new phase of the same size.

## Implementation phases

Each phase ends with the relevant validation, a Slice review and a coherent commit. Keep one current path at every accepted checkpoint. Use bounded subagent work where useful, with explicit owners and no concurrent writers for the same subsystem. Independent review complements the implementer's Slice review and does not replace it.

### Phase 0: activate and inventory

**Goal:** establish the real integration boundary and a complete removal map before code changes.

- [ ] Confirm accepted source/token-layout Phase 3 completion and the selected intervening merges.
- [ ] Record activation revision, worktree state, active diagnostic representation and any existing failures in working notes.
- [ ] Add the roadmap interlude and explicit Phase 4 blocker. Preserve the separate diagnostics-improvement pause.
- [ ] Read the authorities and locate every owner in the removal/reuse and docs matrices.
- [ ] Inventory old syntax, runtime helpers, metadata, summaries, test harness behaviour, generated docs and current design claims.
- [ ] Classify each hit as remove, adapt, preserve, historical or external follow-up. Explain each adaptation and protected neighbouring contract.
- [ ] Record the existing support matrix, representative ordinary-template output and relevant non-recording benchmark baseline.
- [ ] Identify any pre-existing native result slots or renamed token/source owners that alter implementation details.

**Exit:** prerequisites are verified, the removal map includes indirect consumers and no unresolved ownership question is hidden inside a broad rename.

### Phase 1: publish the accepted model and teaching structure

**Goal:** give implementation one current semantic authority while making support status explicit.

- [ ] Add the Wiring reference family and publish the contracts above.
- [ ] Replace the old reactivity documentation family and update its navigation/import callers.
- [ ] Generalize the canonical explanation of `of` while preserving generic restrictions.
- [ ] Add the contextual-forms teaching page and concise cheatsheet summary, covering all five teaching groups above.
- [ ] Normalize future channel syntax to the typed multi-bind plus `Channel()` and keep omitted initialisation hypothetical.
- [ ] Update compiler, build, memory and language-routing authorities for the intended foundation and discarded runtime model.
- [ ] Remove the inherited reactive follow-up roadmap. Keep only explicit new-model boundaries and open design work.
- [ ] Label Wiring as not yet implemented at this checkpoint. Keep the matrix honest about old code that remains until Phase 2 without teaching it as the accepted model.
- [ ] Mark all illustrative UI APIs and async examples as future integration, not runnable promises.
- [ ] Rebuild docs through the normal release-build path and check navigation and stale output cleanup.

**Exit:** canonical semantics, teaching examples and implementation status agree. The docs contain no suggestion that a wire automatically subscribes, mutates, rerenders or sends.

### Phase 2: remove the obsolete reactivity implementation

**Goal:** leave ordinary Moth functioning without the old source/subscription/runtime model.

- [ ] Remove `$Type`, `$=`, reactive `$T` parameters and `$(source)` parsing and semantics. Preserve legitimate template directives and literal text.
- [ ] Remove reactive declaration kinds, default restrictions and syntax-specific diagnostic reasons that have no current owner.
- [ ] Delete reactive String metadata propagation, fixed-point return analysis and TIR/owned-handoff subscription reducers.
- [ ] Remove reactive HIR template records, source-dependency lists, invalidation facts and all associated remapping/validation branches.
- [ ] Remove old public-interface summaries, generated-artifact fields and reachability facts that describe the discarded runtime.
- [ ] Delete reactive JS source scheduling, dependency registration, template-object snapshots and reactive mounting.
- [ ] Collapse the HTML-JS bootstrap to ordinary fragment insertion without disturbing order or entry activation.
- [ ] Remove reactive-specific IO/assert sink distinctions while preserving ordinary IO typing and lazy assertion-message evaluation.
- [ ] Review string coercion, map keys, resource strings, Float formatting, copy/return lowering and template composition for now-redundant reactive branches.
- [ ] Delete obsolete fixtures and manifest entries. Rewrite only tests with a distinct surviving contract.
- [ ] Remove reactive-only test-harness hooks and timing assumptions after checking remaining callers.
- [ ] Remove or correct stale comments, educational diagrams, examples and live plans affected by the deletion.
- [ ] Retain source-identity logic only with a real current consumer. Do not keep dead scaffolding for future reuse.
- [ ] Update the progress matrix to record the actual removal.

**Exit:** the old feature is gone from accepted syntax and executable artifacts. No compatibility alias, scheduler, live-String representation or disguised reactive metadata path remains. Ordinary feature regressions pass.

### Phase 3: implement closed `of` parameter contracts

**Goal:** parse and resolve Wiring contracts using existing declaration and call machinery.

- [ ] Extend the shared retained parameter/declaration syntax for `Wire of T` and the confirmed `Route of ...` forms.
- [ ] Keep generic specialisation intact and handle compiler-owned heads through the existing naming policy.
- [ ] Resolve underlying types and route input/output contracts in AST semantics, not the tokenizer or project builder.
- [ ] Preserve type aliases for ordinary underlying types without introducing aliases for wire/route capabilities.
- [ ] Distinguish parameter contracts from ordinary semantic type identity in all affected signature owners.
- [ ] Reject capability positions outside V1, including fields, payloads, ordinary local annotations, defaults and return annotations.
- [ ] Keep arrow and comma ownership explicit when routes appear between parameters or before an ordinary function return type.
- [ ] Update the Moth code highlighter and repository-local editor resources without making the formatter a semantic parser.
- [ ] Keep Channel work to the accepted documentation scope. Do not implement async by making a generic `Channel` placeholder compile.

**Exit:** one parser and one semantic head owner describe the new contracts. Valid foundation syntax has a defined path to the later phases and unsupported execution is diagnosed honestly.

### Phase 4: implement wire identity and contextual forwarding

**Goal:** preserve named-binding identity without introducing wrapper values or hidden mutation.

- [ ] Resolve eligible named local arguments and already-wire parameters at immediate wire-receiving boundaries.
- [ ] Demand identity only from actual wire uses, with no special source declaration.
- [ ] Preserve wire identity through compatible calls and forwarding chains.
- [ ] Produce ordinary value reads at ordinary receiving positions and ordinary returns.
- [ ] Keep immutable locals immutable and reject mutation through a read-only wire parameter.
- [ ] Reject direct projected places and fresh expressions under the agreed V1 limit.
- [ ] Separate binding-instance identity from source-file identity, static declaration identity and the current stored allocation.
- [ ] Lower the facts into validated HIR using the current local/result/argument representation.
- [ ] Preserve normal borrow rules for reads and aliases. Reject capability escapes instead of relying on host GC.
- [ ] Thread contracts through canonical public signatures and normal generic materialisation where required for source helpers.
- [ ] Use only minimal faithful backend support already justified by synchronous reads/forwarding. Gate unsupported reachable execution before lowering.
- [ ] Record actual frontend and backend support in the matrix.

**Exit:** named-local wire semantics and forwarding are tested through their real owners. No observer graph, invalidation scheduler or source-visible handle has been introduced.

### Phase 5: implement contextual constructor routes

**Goal:** establish type-checked constructor recipes and captures without building callbacks or event delivery.

- [ ] Use the immediate resolved Route parameter to contextualise a direct choice path/application.
- [ ] Support zero-input unit variants, fully captured zero-input payload constructors and single-input constructors with a captured leading prefix.
- [ ] Check exact output choice identity, future input arity/types and payload field order.
- [ ] Evaluate captured arguments through the existing argument/value-production path, retaining normal effects, error propagation and ownership obligations.
- [ ] Preserve the constructor identity and captured data distinctly from an already constructed message.
- [ ] Support unchanged route-parameter forwarding, including through ordinary generic helpers.
- [ ] Preserve ordinary constructor missing-argument diagnostics outside Route boundaries.
- [ ] Reject arbitrary function/closure arguments, capability storage, invocation and route composition.
- [ ] Add AST/HIR validation for route argument kind, capture count/types, future payload range and canonical remapping.
- [ ] Preserve the contract through exported signatures, imported helpers, fingerprints and generated materialisation.
- [ ] Keep deferred runtime consumption behind the ordinary structured target-contract boundary. Do not create an event queue, callback table or message dispatcher merely to demonstrate the syntax.

**Exit:** the confirmed route forms are represented and validated without general callable values, transport semantics or reactive String metadata.

### Phase 6: close cross-stage and regression coverage

**Goal:** prove that the foundation integrates with the real compiler rather than only isolated parser fixtures.

- [ ] Run real-source frontend tests through the existing compiler service and inspect the validated HIR facts that have no public runtime observer yet.
- [ ] Test compatible and incompatible cross-module wire/route signatures and ordinary generic helper materialisation.
- [ ] Test repeated calls/instantiations so local identities and capture data do not leak across boundaries.
- [ ] Test route captures involving ordinary reads, explicit copies and the current result/error representation.
- [ ] Verify authored inactive branches still receive required semantic checks while inactive executable facts do not leak into target gates.
- [ ] Test public command rejection for unsupported reachable execution without backend panic or silent downgrading.
- [ ] Test ordinary runtime templates, resources, assertion messages, numeric formatting, maps and fragment order after removal.
- [ ] Review Boracle extraction and feature-gated tests touched by the removal. Run the relevant opt-in lane separately.
- [ ] Audit case ownership and remove redundant test helpers, obsolete goldens and weak replacement expectations.

**Exit:** every required contract in the coverage matrix below has an owner and an honest support boundary.

### Phase 7: final documentation, deletion audit and handover

**Goal:** leave one current model and a clean boundary for compact diagnostics to resume.

- [ ] Repeat the repository-wide stale-model sweep, including comments, live plans, examples, helper-demand logic, test tooling and generated output.
- [ ] Complete the documentation matrix and check the Basic/Advanced distinction.
- [ ] Confirm every teaching example uses the actual V1 wire operand limit and labels missing UI/runtime support.
- [ ] Confirm channel declarations use the accepted form and no documentation presents omitted initialisation as supported.
- [ ] Update the progress matrix by actual compiler/backend behaviour, not the ambition of the design.
- [ ] Update `index.md`, contributor routing and stale audit-status markers under the current policies.
- [ ] Run an independent final review of deletion completeness, parser ownership, capabilities/TypeId separation, lifetime safety, runtime gates and teaching accuracy.
- [ ] Run the full current validation gates and non-recording performance checks described below.
- [ ] Record the final support matrix, remaining deferred work and changed diagnostic families in the completion handover.
- [ ] Release the compact-diagnostics Phase 4 blocker only after this interlude has landed and passed its gates.
- [ ] Delete this plan and remove its roadmap entry in the completion commit. Keep durable semantic rules in their permanent owners.

**Exit:** the compact-diagnostics migration can resume against the new vocabulary and actual foundation, with no obsolete reactivity representation left to migrate.

## Coverage and acceptance matrix

Use source integration tests for user-visible behaviour and focused subsystem tests for otherwise unobservable semantic facts. Do not add a public event runtime or new compiler CLI solely to make a test possible.

| Contract | Required evidence |
|---|---|
| Old reactivity is removed | Ordinary invalid-syntax behaviour for representative old forms and artifact absence of obsolete runtime helpers. No migration-specific diagnostic family |
| Directives are preserved | Existing `$md`, `$code`, `$slot`, `$insert`, `$children`, `$fresh` and other accepted directive cases still pass |
| `of` is head-directed | New contracts resolve, existing generic examples still work and invalid capability-as-type forms diagnose |
| Parameter grammar | Route between ordinary parameters, function return arrow after closing `|`, missing types/output, malformed arrows and existing generic comma boundaries |
| Wire eligibility | Mutable local, immutable local and forwarded wire positives. Direct field, lookup and fresh-expression negatives |
| Wire identity | Same local forwarded through helpers remains the same origin. Distinct activations do not accidentally share identity |
| Value versus identity | Ordinary reads/returns carry ordinary values, while compatible wire receivers preserve identity. No hidden wire in String/local assignment |
| Mutation authority | Immutable values stay immutable, wire parameters cannot provide mutation authority and ordinary exclusive calls retain their existing rules |
| Zero-input routes | Unit variant and fully captured payload constructor accepted. A missing payload field cannot be supplied by a no-payload click |
| One-input routes | Bare constructor and captured leading prefix accepted with exact remaining input type |
| Ordinary constructor boundary | The same partial constructor outside a Route argument reports its normal missing-field error |
| Route restrictions | Wrong choice/type/arity, non-prefix partial captures, arbitrary callable arguments, storage, return, invocation and composition diagnose |
| Capture semantics | Capture data and evaluation order survive AST/HIR lowering. Side effects and failures are not removed by treating a recipe as static |
| Generic and module boundaries | Plain, Wire and Route parameters remain distinct through imports, fingerprints, canonical remapping and helper instantiation |
| Unsupported runtime | Public commands use structured target/deferred diagnostics for reachable unsupported execution. Unreachable code follows existing reachability rules |
| Ordinary templates/strings | Runtime and const output, slot/wrapper composition, resource anchors, map string keys and lazy assertions preserve their contracts |
| HTML assembly | Entry activation, imported-root suppression, fragment ordering and exactly-once ordinary insertion remain correct |
| No runtime expansion | No new automatic invalidation, subscription graph, event queue, channel bridge or GUI package appears in production |
| Future channels | Docs use the exact accepted declaration and distinguish endpoint roles, deferred async and hypothetical initialiser omission |
| Teaching accuracy | All shared-form lessons are present, examples are syntax-correct for their labeled scope and no V1 example wires a field projection |

Keep broad source-name bans in the existing repository source-audit owner when an enduring architecture check is justified. Do not create unit tests that read production source files and claim a grep result proves runtime behaviour.

## Diagnostic policy during the interlude

Use the structured diagnostic and span APIs delivered at activation. This plan precedes compact-diagnostics Phase 4, so it must not assume that phase's record, draft or cold-store APIs are already available.

Required diagnostic distinctions include malformed specifications, invalid parameter positions, non-binding Wire arguments, forbidden mutation through a wire, route input/output mismatch, incomplete ordinary constructors and unsupported reachable runtime capability use.

Use existing diagnostic families where their meaning matches. Add a family only for a genuinely new source contract. Delete obsolete reactivity constructors, reasons, render branches and descriptor registrations. Do not repurpose or renumber retired external codes for Wiring.

Retain typed facts and primary/secondary source context rather than pre-rendered prose. Prefer locations at the argument and receiving parameter declaration for incompatible wiring. Preserve captured-expression diagnostics at their authored locations.

Discontinued syntax gets ordinary syntax diagnostics. Valid but deferred new surfaces get the current deferred/target mechanism. Neither category is a compiler invariant failure.

The completion handover should identify additions and deletions by their real registered names after implementation. Do not allocate speculative diagnostic numbers in this plan.

## Repository-wide stale-model audit

Start with case-insensitive searches for `reactiv`, then trace definitions and callers. Also search exact runtime symbols and old syntax positions so a renamed or indirectly referenced path is not missed.

Useful inventory terms include:

```text
ReactiveSource
ReactiveTemplate
ReactiveSubscription
ReactiveInvalidation
reactive_source
reactive_template
reactive_invalidations
reactive_templates
__moth_reactive_
__moth_template_string
__moth_template_snapshot
__moth_template_dependencies
__moth_template_collect_dependencies
__moth_mount_template_fragment
subscription-bearing
live sink
live String
whole-source
rerender
```

Old syntax searches must distinguish code from directive expressions and template text. `$Type`, `$=`, `$T` and `$(source)` are removal targets in their former language positions. `$` itself remains valid for directives.

Sweep tracked implementation, tests, manifests, source packages, tooling, benchmarks, current docs, live roadmap work and generated output. Check code comments that never mention reactivity but still preserve template objects, gather dependencies or special-case String arguments for delayed rendering.

Every residual relevant hit needs a disposition:

| Disposition | Acceptable result |
|---|---|
| Remove | No remaining owner or consumer in the accepted model |
| Adapt | A named surviving responsibility, with new callers and tests, no obsolete dependency semantics |
| Preserve | An independently required contract such as resource-string structure or build-cache invalidation |
| Historical | A dated audit/evidence record whose meaning is preserved and whose current status is accurately marked |
| External follow-up | An artifact outside this repository, explicitly reported rather than claimed fixed |

This plan and a bounded completion record may mention the removed model to explain the change. The final user reference, current compiler comments and executable paths must not teach or rely on it.

## Validation and performance gates

Read the current validation guide and `justfile` at activation. Commands below are known entry points, not pinned test counts or a substitute for the current gate definition.

For each code-bearing phase:

```sh
cargo fmt --all -- --check
just validate
git diff --check
```

Run focused tests while iterating, then the full gate before acceptance. A mixed code/docs phase is code-bearing even if most changed lines are documentation.

For documentation-only phases and the final generated-docs check:

```sh
cargo run --quiet -- build docs --release
```

For canonical fixture ownership after removal and replacement:

```sh
cargo run --quiet -- tests --audit
```

At final closeout, run the standard feature matrix and relevant opt-in lanes according to current ownership:

```sh
just test-feature-matrix
just boracle
```

The Boracle lane is required when its extraction, fixtures, interfaces or feature-gated code was touched. Run the separate campaign lane only when affected or required by the current guide. Do not claim `just validate` covers opt-in lanes it excludes.

Use non-recording performance evidence:

```sh
just bench-check
```

Compare ordinary non-Wiring and template-heavy workloads against the activation baseline. Check for unconditional metadata work, new common-record growth and unexpected allocation/helper emission. Fewer source lines or removal of tests is not by itself evidence of faster compilation. Explain measured regressions and remove avoidable new work before closeout.

Do not modify tracked benchmark input while a measurement runs, rewrite benchmark history to hide a change or claim a clean gate when unrelated failures remain. Record the exact command, revision and outcome for each completed gate.

## Final completion checklist

- [ ] All ten accepted interview decisions are reflected in permanent authorities.
- [ ] Old reactivity source forms and their runtime semantics are removed without compatibility paths.
- [ ] `$` remains a directive marker, not a Wiring declaration or subscription operator.
- [ ] `of` has one documented head-directed explanation and existing generic restrictions remain intact.
- [ ] Named-binding wires and contextual constructor routes reach their required semantic and HIR boundaries.
- [ ] Wire and Route contracts are not disguised as ordinary reference/function wrapper types.
- [ ] Capture evaluation, ordinary aliasing, mutation authority and canonical identities remain sound.
- [ ] No source identity is confused with the source database's `SourceId` or allowed to escape merely through metadata.
- [ ] No live String, automatic scheduler, route dispatcher or future channel implementation has been introduced.
- [ ] Runtime support and rejection are stated exactly in the progress matrix.
- [ ] The typed channel declaration is documented as future syntax and omitted initialisation remains outside this plan.
- [ ] The general teaching lesson covers member lists, `of`, `~`, immediate receiving context and blocks/templates.
- [ ] Illustrative UI examples use named local wire operands and clearly state the missing runtime/package boundary.
- [ ] All stale-model audit hits have a recorded disposition and all obsolete mixed-path branches are removed.
- [ ] Tests have one clear primary owner per behaviour and no obsolete scaffolding survives to keep old tests passing.
- [ ] Current validation, docs generation, relevant feature lanes and non-recording performance checks have completed with honest outcomes.
- [ ] The final independent review and Slice review are complete.
- [ ] Compact-diagnostics Phase 4 is unblocked only after this work lands.
- [ ] The completion commit removes this plan and its roadmap entry, leaving contracts in permanent documentation.

## Completion handover

Keep the handover short and factual: the implemented frontend/backend support matrix, permanent reference paths, deletion/reuse results, actual diagnostic changes, validation results and explicitly deferred runtime work.

Do not call the result a UI framework replacement already implemented. It is the clean semantic foundation for later Wiring consumers, with ordinary templates and choices preserved and the old reactivity model removed.
