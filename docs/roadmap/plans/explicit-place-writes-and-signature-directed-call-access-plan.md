# Explicit Place Writes and Signature-Directed Call Access

**Status:** Queued
**Current slice:** None
**Blockers:** Complete the active compiler source, token and diagnostic data-layout cutover, including its final failure-lane and cleanup gates
**Next action:** Activate from the completed baseline, refresh the ownership inventory and execute Phase 0

## Purpose and sequencing

Make writes to existing places explicit without requiring access markers on ordinary function arguments. Keep Moth's existing reference semantics, mutable aliases, borrow and lifetime validation, binding identity, inferred cleanup transfers and Wire/Route parameter contracts. This is a final language-surface cutover, not a compatibility migration.

Schedule this work **immediately after the active diagnostic data-layout work** and before the queued diagnostic recovery and improvement work. At activation, use the fully completed data-layout and typed-diagnostic architecture. Later diagnostic work must consume the new access rules and must not reintroduce explicit `~` call-argument guidance. Other sequenced plans retain their relative order.

The plan is based on the `diagnostic-data-layout-changes` branch's AST, HIR, diagnostic and documentation ownership, but intentionally pins **no** pre-activation commit. Inspect the merged tree again before implementation. The active branch still contains temporary Phase 4 cutover details, which are not architectures to preserve after that work finishes.

Permanent authorities: canonical language references under `docs/src/docs/`, access/aliasing and borrow references under `docs/src/developer-docs/memory-management/`, `docs/compiler-design-overview.md`, `docs/compiler-data-layout-design.md` and the codebase style, testing and validation guides. The progress matrix owns *implementation status*. Do not copy status or transition notes into final language references.

## Accepted final contract

### Explicit mutation and access

| Context | Legal form | Meaning |
|---|---|---|
| New immutable local | `count = 0` | Define a binding |
| New mutable local | `count ~= 0` | Define a mutation-capable binding |
| New typed mutable local | `count ~I32 = 0` | Define a typed mutable binding |
| New mutable alias | `writer ~= original` | Retain the established exclusive write-through alias semantics when `original` is an eligible mutable place |
| Replace existing binding/place | `~count = 3` | Explicit write to an existing mutable place |
| Compound update | `~count += 1` | Explicit checked read/modify/write of the existing place |
| Field write | `~record.position.x = 3` | Exclusive access to the complete rooted destination place |
| Multi-result update | `~count, message = next()` | Write an existing target and define a new one in a single receiving statement |
| Mutable ordinary call argument | `update(count)` | Callee's established parameter contract selects exclusive access |
| Shared ordinary call argument | `inspect(count)` | Callee's established parameter contract selects shared access |
| Mutable receiver | `~count.increment()` | Explicit exclusive receiver access |
| Shared receiver | `count.inspect()` | Ordinary shared receiver access |
| Wire argument | `observe(count)` | Established `Wire of T` contract forwards binding identity, not exclusive access |
| Route argument | `register(Message::Changed(id))` | Existing immediate constructor-route receiving rules apply |

The leading `~` on an **assignment target** is mandatory for every authored write to an already existing place, including every compound-assignment operator. It prefixes the *whole* destination, not a member (`~point.x`, not `point.~x`). All RHS expressions retain ordinary value syntax. The prefix does not copy, move, change the static type, rebind an alias or change a Wire binding's identity. It requests the access required to perform the direct write.

An explicit mutable **method receiver** continues to require `~` and must name an existing mutable place. This includes source methods, collection/map builtins and supported binding-backed receiver operations. Shared method receivers remain unprefixed. A fresh/computed receiver does not become a valid mutable receiver just because the compiler could materialise a hidden argument local.

**Ordinary function arguments are value positions.** A call argument never takes an authored `~` marker, even when its corresponding parameter is `~T`. The resolved function, method non-receiver, generic, Core, Builder or external binding parameter contract determines the access. A mutable parameter accepts an existing mutable place through exclusive access or a fresh value through the established fresh-rvalue materialisation path. An existing immutable place cannot satisfy a mutable parameter. For `Wire of T`, preserve named-local identity rules. For `Route`, preserve its direct-constructor/forwarding restrictions. No call-site access inference searches callee bodies or changes signatures, aliases, captures or type identity.

The special syntax is intentional: **a method's receiver is an access-bearing syntactic position while ordinary call arguments are contract-directed values**. A free function equivalent to a mutating method may therefore use `update(value)` where the method uses `~value.update()`. Public signatures and editor tooling expose the access requirement. Do not claim every mutation is visually marked at the caller: a free function may mutate through an unmarked argument.

The following are invalid or retain their existing distinct diagnostics:

- `count = 3` and `count += 1` when `count` already exists, regardless of its mutability; require `~count`.
- `~count = 3` when `count` is immutable or unavailable; diagnose the actual binding/access problem, not an assumed declaration.
- `~count` as an ordinary value, including `f(~count)`, `f(arg = ~count)`, `return ~count`, `copy ~count` and `writer ~= ~count`. There is no remaining call-argument-marker or alias-source-marker exception.
- `~temporary.field = value` when the target is not a writable existing place; `~value` as a standalone statement.
- `~this = value`, retaining the receiver binding's non-reassignability. `~this.field = value` is eligible when receiver and projection access rules permit it.
- `point.~field = value`, `point.~method()` and prefixing a shared method receiver with `~`.
- Applying `~` to a fresh/computed method receiver or to an immutable receiver.
- `~` on declarations other than their existing `~=`, `~Type` and mutable parameter syntax; constants, defaults, named argument labels, struct fields and choice payload labels do not become assignment targets.

No `&`, `&mut`, explicit move operator, reference type, additional mutability type identity or alternative marker is introduced. No compatibility parsing, deprecated warnings, legacy aliases or two-form acceptance window.

### Preserve semantic invariants

- A definition creates a new dynamic binding instance, including re-entry through loops. Assignment writes an existing place. A mutable alias writes through the referent without silently detaching. Ordinary value results may share allocations without aliasing the caller's binding cell. Binding identity, allocation provenance and cleanup responsibility remain distinct.
- Read all multi-result RHS slots once before committing existing targets. Each multi-bind target independently distinguishes definition from update. Duplicate targets and invalid mixed declarations remain errors. The final HIR owns the distinction rather than reconstructing it from `~` or borrow states.
- A compound update computes its RHS, checked arithmetic and store conversion before committing. Failure leaves the old value/topology intact. Retain existing numeric receiving, error/handler and rollback behavior, including pending body-local numeric origins.
- Borrow checking still rejects overlapping exclusive/shared accesses, multiple exclusive arguments, illegal long-lived aliases and invalid retention. Optional last-use transfers remain an optimisation only. Source `~` is never evidence of an inferred move.
- Calls retain parameter-order routing, named arguments, defaults, multiple result slots, generics, exact numeric contexts, deferred numeric completion, effects, imported signatures, cross-module summaries and HIR `CallPassingMode` or an equally narrow final passing classification.
- Wires keep the identity of a reassigned binding, including across repeated loop instances and ordinary value reads. Constructor routes keep once-evaluated captures and their zero/one-future-input rules. No new observation or event-delivery semantics.
- Backend/target capability limits are unchanged. This syntax refactor cannot legalise unsupported Wasm mutable call operations, change selected target or weaken mandatory analysis.

## Verified ownership map and likely edits

These are source locations verified against the diagnostics data-layout branch. Refresh them when activating, since its current diagnostic migration is still in progress.

| Owner | Relevant files and cutover |
|---|---|
| Statement dispatch | `src/compiler_frontend/ast/statements/body_dispatch.rs`, `body_symbol.rs`, `declarations.rs`, `body_expr_stmt.rs`. Route leading `~` once to existing-place assignment or receiver-call parsing. Preserve statement diagnostics and `this` special handling. |
| Assignment semantics | `src/compiler_frontend/ast/expressions/mutation.rs`, `parse_expression_places.rs`, `src/compiler_frontend/ast/statements/numeric_body_completion.rs`, value-producing receiver modules. Keep place resolution and compound write-back; add explicit-target gate without duplicated expression evaluation. |
| Multi-bind | `src/compiler_frontend/ast/statements/multi_bind.rs`, `src/compiler_frontend/ast/numeric_construction.rs`. Each target needs independent prefix intent, declaration classification and complete deferred-result handling. Current `parse_target_list` assumes a symbol first; it must accept prefixed existing targets without confusing `name ~Type` with `~name`. |
| Call syntax and access | `src/compiler_frontend/ast/expressions/call_arguments.rs`, `call_argument.rs`, `call_validation.rs`, `src/compiler_frontend/ast/place_access.rs`. Remove the authored argument marker, derive passing from the signature and source state, and retain only facts consumed after validation. |
| Receiver methods | `src/compiler_frontend/ast/field_access/receiver_access.rs`, `receiver_calls/source_methods.rs`, collection/map builtins, `src/compiler_frontend/ast/generic_functions/receiver_method_instantiation.rs`. Preserve authored receiver marker and do not conflate synthetic receiver arguments with ordinary unmarked arguments. |
| Deferred/special calls | `src/compiler_frontend/ast/numeric_construction.rs`, `expressions/expression_store.rs`, template directive argument handling, anonymous const records, Core/binding-backed calls, compiler-owned intrinsics and assertions. Remove marker state from deferred argument shapes while preserving established slot/contract routing. |
| HIR and analysis | HIR call/assignment lowering and validation, `src/compiler_frontend/analysis/borrow_checker/` and Wiring summaries. Keep semantic passing modes, dynamic definition/update, result provenance and flow effects; change only what is actually derived from deleted source marker metadata. |
| Diagnostics | The final typed `compiler_messages/diagnostics/` draft families and renderers, `syntax_errors/`, source labels and integrations. Do not revive deleted `CompilerDiagnostic` payload/renderer architecture from main or depend on active-branch migration adapters. |
| Documentation/tooling | Canonical bindings, functions, structs, collections, branching, patterns, memory and Wiring references; Basic pages, cheatsheet, design overview, compiler/data-layout and memory docs, progress matrix, HTML `$code("moth")` highlighting and VS Code grammar where necessary. |
| Test and sample owners | `tests/cases/`, `src/compiler_frontend/**/tests/`, `src/compiler_tests/`, benchmark `.moth` inputs, source-backed packages, templates, examples, generated docs and test scaffolds. Avoid blind text replacement. |

**Specific confirmed redundancy:** `CallArgument` currently carries both `access_mode: CallAccessMode` (authored marker intent) and `passing_mode: CallPassingMode` (validated semantics), plus `marker_span`. `DeferredCallArgumentShape` retains marker intent across numeric completion. Ordinary calls will no longer need authored marker state. Remove those fields/variants and their mapping only after verifying that receiver synthesis is independently handled. The final semantic passing classification **must stay**. Call validation currently has its own recursive argument-source classifier, while `ast/place_access.rs` has a borrowed iterative receiver-place classifier. Consolidate the common source-state walk if it genuinely eliminates duplicate work and doesn't obscure receiver-specific policy. Measure before adding any further abstraction.

## Phased implementation

Every phase ends with a working checkpoint: no mixed old/new language semantics, no temporary compatibility acceptance and all relevant owned tests green. A phase may internally use short-lived uncommitted edits but must not leave a temporary fallback or obsolete adapter at its accepted boundary. If a semantic parser cutover invalidates fixtures, migrate all affected primary fixtures and runnable examples in that same phase before committing it. Documentation edits may be staged with their owning syntax changes to keep doc compilation green, then comprehensively audited in Phase 5.

### Phase 0 - Activation inventory and locked baseline

1. Start from the completed diagnostic-layout branch merged to the intended execution baseline. Confirm final typed draft/frozen report APIs, the progress matrix, `index.md`, current style guide, testing guide, validation commands and all active source/fixture locations. Do not start from temporary Phase 4 diagnostic adapters.
2. Record a scope-owned search inventory for `~` markers, `~=`, every compound assignment, `CallAccessMode`, `CallPassingMode`, `marker_span`, mutable missing-marker reasons, deferred numeric argument shapes and value-producing/multi-bind receivers. Include Rust source, integration cases, fixtures, source-backed packages, benchmark inputs, docs, emitted example assets and editor highlighting. Classify each hit by semantics before editing.
3. Capture baseline `just validate` and feature Clippy results plus baseline memory/throughput measurements for the existing call parsing, validation, AST argument storage and focused frontend benchmarks. Record actual commands and environments. Reuse existing benchmark counters; add one temporary/local measurement only if it can be retired.
4. Identify distinct existing positive/negative primary tests so the refactor does not duplicate them. Make one canonical test matrix for direct writes, typed and inferred declarations, mixed multi-bind, mutable calls, methods, aliasing, Wire/Route, deferred numeric and target-specific behavior.
5. Confirm no agreed semantic case is missing from the final contract. Unclear behavior is recorded as a blocker for that specific slice rather than silently interpreted from implementation quirks. Keep this plan's status small; record test/performance evidence in working notes, not permanent design docs.

**Gate:** verified post-diagnostic APIs, a reviewed source inventory, representative old-behavior integration evidence and a test/benchmark baseline. No user-facing semantics changed.

### Phase 1 - Explicit existing-place assignment grammar

1. Add one clear statement-level path for leading `~` followed by a writable place and a supported assignment operator. Reuse the existing place parser, mutability checks, typed AST assignment and HIR `PlaceId`. Keep `~receiver.method(...)` on the receiver-call path; select based on grammar without parsing and backtracking through a completed expression. Handle simple locals, chained fields, mutable aliases and eligible `this` fields.
2. Require that marker on every existing direct and compound assignment. Unmarked `name = ...` remains a new declaration only when no visible binding exists. An unmarked `name = ...` referring to an existing binding must be an actionable missing-marker diagnostic, not a redeclaration and not a successful write. Keep no-shadowing and typed-declaration collisions in their established priority.
3. Preserve the declaration grammar exactly: `name ~= value`, `name ~Type = value`, `name Type = value`, `name #= ...`. Do not let leading `~` reinterpret declaration markers, named arguments or parameter syntax. Maintain the established operator whitespace rules for `=`, all compound operators and adjacent `~=`.
4. Carry only the minimum authored marker location needed for accurate errors through parsing. No new marker field in stable HIR: AST validation consumes syntax and emits the same semantic write to an existing place. Preserve numeric origin/receiver constraints and deferred RHS write-back.
5. Convert all affected runnable fixtures and snippets exercised by test/doc builds together with this cutover. Keep real code behavior and output unchanged. Cover `~this.field`, forbidden `~this =`, temp/non-place targets, missing assignments and mutating receiver distinction.

**Gate:** old unprefixed updates fail with the intended diagnostic, valid prefixed updates preserve runtime behavior, all compound/field/alias/source-method tests pass and no old successful assignment route remains.

### Phase 2 - Explicit targets in multiple-result and value-producing receivers

1. Teach the single multi-bind target parser a leading-marker form for **existing** targets, without violating the normal declaration syntax for new typed/mutable bindings. Retain its unique-name checks, declared types, pending numeric slot construction and single producer evaluation. Do not create a second multi-bind parser.
2. Resolve targets independently: `~a, ~b = pair()` updates both; `~a, new_value = pair()` mixes update/definition; `a, b = pair()` remains a declaration when both are new. `a, b = pair()` with an existing `a` must diagnose its missing marker; prefixed targets must not silently define fresh locals. Keep the existing explicit-type requirement on typed mutable *new* multi-bind declarations and do not misread `~a` as that form.
3. Cover multiple-return calls, value-producing `if`/match/`catch`, multiple-result propagation and deferred generic/numeric producers. In every case evaluate all incoming results before writes. Preserve write-through alias behavior, result-to-result alias relationships, checked writes and failure non-commit.
4. Audit the ordinary single-target value-producing assignment paths (`~target = if ...`, `~target = ... catch ...`) for the same destination classification and diagnostics. Do not add assignment-as-expression semantics.

**Gate:** one integration scenario protects mixed definitions/updates, evaluation order, alias observations and failure paths; focused parser tests cover ambiguous target forms and diagnostics. No duplicate target-policy helper survives.

### Phase 3 - Signature-directed ordinary call access

1. Remove authored `~` from the ordinary call-argument grammar in **all** call-shaped value positions: direct calls, named and positional arguments, generic calls, method **non-receiver** arguments, Core/Builder/external functions, directives, intrinsics, record/constructor labels, forwarded argument syntax and retained numeric arguments. Keep receiver syntax and unrelated declaration `~` markers. Invalid `f(~value)` must be diagnosed as invalid syntax or value-position access, not accepted as a compatibility spelling.
2. Resolve one `CallPassingMode` from the established parameter contract and resolved argument source state:
   - Shared parameter + legal value -> `Shared`.
   - Mutable parameter + existing mutable place -> `MutablePlace`.
   - Mutable parameter + fresh value, including valid `copy` -> `FreshMutableValue`.
   - Mutable parameter + immutable existing place -> typed diagnostic.
   - `Wire`/`Route` -> preserve their separate capability validation and identity/capture facts, never auto-convert a plain value parameter into a capability.
3. Preserve named-slot routing before access selection, exact numeric receiver constraints, defaulted arguments, generic specialisation, cross-module published access metadata, binding-backed opaque handles and deferred argument completion. Unknown or unresolved contracts must not default to mutable access or inspect a body; wait for the established contract resolution boundary.
4. Remove source marker state (`CallAccessMode`, call-side `marker_span`, marker-derived defaults, deferred shape fields and historical reasons) wherever it exists solely for ordinary arguments. Retain `CallPassingMode` until evidence shows a narrower equivalent. Adjust test constructors and expression-store mappers instead of keeping fake marker defaults. Avoid recomputing this mode in HIR or the borrow checker.
5. **Receiver exception:** preserve the existing authored `~receiver.method(...)` validation, `ReceiverAccessMode`/receiver source classification and synthetic receiver argument semantics. Do not accidentally route that synthetic receiver through new ordinary-argument inference as though the authored receiver marker were absent. Method non-receiver parameters use the new inferred call contract.
6. Merge duplicated argument/receiver source classification where an existing borrowed classifier already suffices. Prefer a single iterative place-root walk if it avoids recursion and repeated lookups. Keep narrow policy checks local to calls and receivers. Delete redundant wrappers, old branches and repeated scans; do not create a general-purpose access inference framework.
7. Preserve borrow/lifetime facts and exported access/effect summaries across all source and binding-backed call families. An unmarked call may still mutate the original place. Existing immutable sources, live shared aliases, two overlapping mutable parameters and unsupported target calls remain rejected at their owning boundaries.

**Gate:** no ordinary `~` argument compiles; all valid mutable calls compile without it; ABI/effects, Wire/Route, borrowed aliases, generated bodies and cross-module semantics are unchanged. Final AST/HIR carries only validated mode/capability data needed by consumers. The current explicit receiver contract still holds.

### Phase 4 - Diagnostics and error-priority hardening

Integrate directly with the **completed** typed diagnostic draft/schema/render architecture. Reuse stable diagnostic codes and families when the mistake stays in the same category. Add a new typed reason/code only for a genuinely distinct, repeatedly useful failure class. Do not handcraft old `CompilerDiagnostic` carrier fallbacks or preserve retired reasons for obsolete source forms.

Required source-facing diagnostics:

| Failure | Primary guidance | Supporting context |
|---|---|---|
| Existing mutable binding written without `~` | "Updating `count` requires `~count = ...`" or the operator-accurate equivalent | Mark the target name and, where useful, original declaration |
| Existing immutable binding written through `~` | "`count` is immutable. Declare it mutable to update it" | Point to binding declaration; avoid marker-only repair |
| Unknown or non-place prefixed target | "`~` must prefix an existing writable place" | Point at `~`/invalid projection; don't assume a declared binding |
| Existing multi-bind target without prefix | "This target updates an existing binding; write `~count`" | Mark the one target, not whole comma list |
| Prefix used for a new multi-bind target | "`~` before a multi-bind name updates an existing binding; declare a new binding without it" | Mark target and show new-vs-update distinction |
| `~` in ordinary value/argument position | "Arguments use ordinary values; the parameter determines access" or a corresponding concise value-position reason | Mark the authored marker, not a fabricated mutable parameter |
| Mutable call parameter receives immutable place | "Parameter `items` requires mutable access, but `items` is immutable" | Mark argument and optionally declaration or callee parameter when available |
| Mutable receiver lacks `~` | "Mutating method `push` requires `~items.push(...)`" | Preserve receiver-specific facts and exact authored span |
| Mutable receiver is immutable/temporary | Explain the declaration or existing-place requirement | Do not suggest `~` alone for immutable/temporary receivers |
| Reassignment of `this` | Preserve existing receiver-binding restriction | `~this.field` may be valid; avoid blocking legal field writes |

Also audit bad fixes suggested by `&`/Rust-like syntax hints, `~=` token spacing, invalid compound write-back, named arguments, duplicate/missing multi-bind targets, borrow conflicts, type mismatch, source-language highlighter examples and deferred diagnostics. All guidance must reflect the **new** grammar, not alternate or historical forms. Preserve diagnostic ordering/priorities when multiple errors are present; the user should see the earliest actionable authored mistake, with exact name/operator and relevant secondary labels.

Test diagnostic **facts, codes, spans, labels and final rendered text** at their appropriate owners, using the post-layout render APIs. Prefer one integration fixture per distinct user-visible contract and focused unit tables for ambiguous punctuation and reason selection. Preserve warning/terse/dev-server rendering via the established shared path. No deprecation diagnostics or automatic syntax repairs.

**Gate:** all new invalid forms receive stable user diagnostics rather than infrastructure failures, panics or misleading receiver/call fixes; obsolete diagnostic variants and render branches are deleted.

### Phase 5 - Documentation, runnable examples and tooling alignment

1. Update canonical source references first: mutable bindings/reassignment, function parameter modes and argument forms, methods/receivers, struct field mutation, collections/maps, multi-bind/branching, copy/exclusive alias semantics and Wiring. Keep the single canonical source contract in each owning page; Basic pages teach examples without duplicating long formal rules.
2. Update `docs/src/developer-docs/memory-management/` access and aliasing, borrow validation and the relevant compiler design paragraphs. Distinguish **explicit write syntax** from **signature-selected exclusive call access**, last-use cleanup transfer, write-through aliases and Wire identity. Do not say every exclusive access uses a source `~` marker.
3. Rewrite cheatsheet examples across sections, not only the bindings section. Check direct/compound writes in functions, `this`, loops, value-producing blocks, mutable alias examples, collection/map calls, `copy`, generic calls, multiple result slots, region placement and Wire/Route examples. Final cheatsheet describes final design only; progress matrix contains implementation status.
4. Review every authored Moth program and Moth snippet in `docs/src/docs/`, developer docs, `.mtf` templates, README, examples, first-party source packages, benches and test fixtures. Separate **code that should compile** from intentionally rejected examples. For rejected examples, update accompanying error rationale and ensure each remains rejected for its intended reason, not an earlier unrelated syntax failure.
5. Run the existing docs generation/check pipeline and verify representative fenced `$code("moth")` snippets through compiler-backed smoke tests where usable. Generated `docs/release/` files must follow their established generation policy; avoid treating generated HTML as an independent documentation authority. Verify links and snippets referencing mutable calls, aliases and compiler diagnostics. Assess the HTML project's Moth syntax highlighter and VS Code grammar/test fixtures for correct `~` token usage; change them only if classification/highlighting breaks.
6. Update the progress matrix only to reflect delivered implementation. Recheck examples in project skeletons, builders, docs sample programs and external-facing materials for both code accuracy and correct type/borrow intent. Do not mechanically add `~` before every `=`: constructors, named arguments and declarations are not writes.

**Gate:** generated docs check/build succeeds; runnable examples compile and execute where supported; illustrative final-design examples have their correct status/rationale; references contain no obsolete call-marker rule; no contradictory snippets remain.

### Phase 6 - Final correctness, performance and code-review closeout

Follow the attached Moth code review guide and the repository's testing/validation rules. This is a **required** cleanup phase, not a discretionary final polish pass.

1. Audit for duplicated or unused access classifiers, obsolete `CallAccessMode`/argument marker fields, redundant stored syntax spans, wrapper adapters, old `~` special cases, stale numeric/deferred retention paths and duplicated parser routing. **Remove**, **merge**, **move** or **leave local** with a stated reason. Delete dead code and redundant fixtures, not just their calls.
2. Audit stage boundaries: syntax intent ends in AST; HIR owns definition/update, resolved argument passing and receiver effects; borrow checking owns access legality; link interfaces own exported parameter contracts; backends do not infer call mutability from authored source. No new build-system parsing or backend semantic fallback.
3. Inspect all affected Rust modules for clear ownership documentation, stage-local helper design, avoidable clones/allocations, unnecessary recursion, repeated walks, oversized records and user-input panic paths. Keep the smallest complete representation. If an apparent optimisation makes control flow harder to understand, require measured benefit before accepting it.
4. Compare baseline and final timings, allocations and retained AST argument record sizes over the same deterministic input mix. In particular test call-heavy frontend, generic/multi-bind/deferred numeric cases and diagnosed workloads. The marker-removal can eliminate source metadata and diagnostics branches, but **record measured outcomes**; do not claim a speedup from deleting syntax alone. Protect against regressions to the existing stage-specific benchmark gates. Remove temporary instrumentation after capturing evidence.
5. Run the project-wide validation and appropriate feature/backend matrices, format/Clippy, compiler integration suite, docs build/check, benchmark smoke/regression suite and representative JS/available Wasm cases. For any unsupported target preserve a precise target diagnostic. No skipped/broken fixtures, stale accepted goldens or silenced errors to obtain green runs.
6. Perform repository-wide semantic searches, including aliases and generated forms, for old call-site `~` guidance, unprefixed existing writes, contradictory docs, legacy diagnostics, obsolete code paths and compatibility language. Classify each remaining hit (valid receiver/declaration, invalid fixture, text explaining a rejected form or real regression). A plain grep count is not evidence of completeness.
7. Review implementation against the original final contract and each required scenario. Confirm public API access/effect fingerprints, inherited/wire binding identities, mutable aliases, no-shadowing, deferred construction, checked failure and stable source diagnostics. Require a concise closeout report listing fixes, intentional specialisations, test coverage, performance evidence and any genuine deferred issue. Any unfixed correctness or duplicated-owner issue blocks merge.
8. Update permanent authority docs as needed and remove the plan in the **same** commit that completes it, following roadmap policy. Do not leave a completed plan, migration note, stale compatibility section or redundant status owner behind.

**Gate:** full green validation, measured performance disposition, no obsolete production path, accurate docs/examples, explicit audit findings resolved and final code smaller/clearer where possible.

## Acceptance matrix (suite-level, not one fixture per row)

| Area | Required positive coverage | Required rejection/consistency coverage |
|---|---|---|
| Simple binding | New inferred/typed mutable bindings, `~x =`, `~x +=`, reads | Unprefixed existing writes, immutable/unknown target, malformed prefix |
| Places/receivers | Nested field writes, `~this.field`, valid mutable methods and builtin collection/map calls | `~this =`, non-place/temporary, prefixed shared method, missing receiver marker |
| Multi-bind | Mixed new/existing, all-existing, typed new target, value-producing if/match/catch and multi-return | Missing/extra `~`, duplicate target, arity failure, invalid target, separate failure-before-write |
| Alias/provenance | `writer ~= source`, `~writer =`, result value alias independent of binding slot, explicit `copy` | Live shared-alias conflict, invalid mutable alias, storage-lifetime conflict |
| Call modes | Shared call, `update(x)` mutable existing place, `update(copy x)`, `update(fresh)` and named arguments | `f(~x)`, immutable existing argument, overlapping mutable arguments, retained borrower conflict |
| Methods and generics | `~x.method(y)` with inferred access for non-receiver `y`, generic inference, generated call modes | Missing/extra receiver marker, invalid generic access/body inference |
| Wire and Route | Identity forwarding with unmarked value, reassignment identity preservation, route partial constructor/captures | Wire on projected/fresh value, invalid route/identity coercions; do not silently grant exclusive authority |
| Numeric/failure | Pending numeric writes, checked compound store, success/error continuation, exact literal receiver | Type conflicts, overflow/failure no-commit, post-error alias and value preservation |
| Modules/backends | Exported mutable parameter interface, imported Core/Builder/binding calls, JS behavior | Missing published access metadata, capability/target rejection, unsupported Wasm mutable ABI |
| Tooling/docs | Signature hover can expose mode from exported metadata, correct code highlighting, runnable docs samples | Misleading repairs, stale examples, broken docs/generation, unused legacy schema |

## Practical diagnostic principles

- Say *what the source did*, which contract it violated and the smallest valid new form. Distinguish "existing binding needs a `~` write prefix" from "immutable binding cannot be written".
- Call diagnostics must attribute the required access to the callee parameter and the caller's actual argument source. Never suggest adding `~` in argument position.
- Receiver diagnostics may suggest `~receiver.method(...)` only when receiver/source facts make it a valid repair; do not invent names or paraphrase internals such as `PlaceId` or loan states.
- Preserve exact source locations through deferred numeric construction and multi-bind so a later diagnostic still identifies the one authored bad target.
- Implement one typed cause with an appropriate stable code, not several messages for one source mistake. Follow the completed diagnostics draft/frozen-report design and test strategy.

## Completion criteria

The final accepted source language has **explicit leading `~` on writes to existing places and mutating method receivers, and no `~` in ordinary value arguments**. Mutable alias creation remains `alias ~= source`. Callee parameter signatures select shared or exclusive access at ordinary call sites. The borrow checker and lifetime model are unchanged in legality and execution, while diagnostics, all owned examples, docs, highlighters and tests consistently teach and enforce the new rule. The compiler has no accepting or diagnostic-only legacy paths for the former syntax.