# Const functions and shared compile-time function evaluation

> For implementing agents: execute this plan in numbered slices on the `const-functions` branch. Read `AGENTS.md`, the routed canonical compiler and language documentation and the style, testing and validation guides before changing code. Preserve the semantic ownership and stop gates below. This plan activates the work but does not authorise shortcuts around the documented compiler stage boundaries.

## Status

```text
STATUS: ACTIVE - dedicated const-functions branch
CURRENT_SLICE: 0 - establish baseline and prove Stage 4 preparation ownership
BLOCKERS: none assumed, Stage 4 and cross-module feasibility gates must pass
NEXT_ACTION: perform slice 0 investigation and record the chosen minimal integration points
```

## Goal

Add explicit Moth const functions with the declaration spelling `name #|args| -> Return:` and use one bounded typed-function execution mechanism for both required const calls and optional release optimisation of ordinary functions.

A const declaration guarantees that every call is executed by the compiler with constant arguments. Invalid const operations, unresolved runtime arguments, dependency cycles and required evaluation failures produce source diagnostics. A const function with no calls is still type-checked and checked for static eligibility, but its parameterised body need not execute.

Ordinary functions retain runtime semantics. Debug builds perform no opportunistic **source-function** call evaluation. Release builds attempt folding of every eligible ordinary call with proven constant inputs, subject to finite execution limits. Successful folds replace the runtime call. Budget-limited attempts retain the call and emit a low-priority `Note` suggesting a higher configured budget. This is an optimisation advisory, not a warning or error.

A practical acceptance example is a cross-module const function that constructs a nominal component record containing a template String and a collection of structural resource Strings. Its consumer must receive the correctly folded value without losing resource identity or exposing the provider's private implementation.

## Authority and scope

- `docs/compiler-design-overview.md` owns stages, AST/HIR facts, cross-stage ownership and constant-value semantics.
- `docs/build-system-design.md` owns module/package publication, project settings and scheduling.
- Canonical unsuffixed language references under `docs/src/developer-docs/language/` and the associated public language references own final source syntax and semantics. Update them when implementation is delivered, not the progress matrix alone.
- `docs/src/developer-docs/style-guide/style-guide.mtf`, `testing.mtf`, `validation.mtf` and `AGENTS.md` govern every slice.
- This plan is implementation scheduling, not an override of those authorities. When an accepted contract below is missing from canonical documentation, the implementing slice must update that authority and then its tests.
- Builder-defined top-level template slots, HTML component abstractions, generic const functions, runtime anonymous records and generic macros are not part of this plan.

## Accepted language contract

### Declaration and invocation

```moth
square #|value I32| -> I32:
    return value * value
;

answer #I32 = square(7)
runtime_binding = square(8)
```

Both calls to `square` are required compile-time calls in every build profile. The resulting values are ordinary constants, regardless of the binding form. Calling `square` with a runtime-dependent argument is an error at that call, even when its surrounding binding is runtime-owned. An explicit const function is not a runtime function with an optional optimisation attribute.

- Const function declarations are statically checked for eligibility independently of whether their bodies are executed.
- A const function may call other const functions and registered deterministic compiler-owned operations. It cannot call ordinary functions, arbitrary host bindings or operations that read or mutate external state.
- Local mutable bindings, control flow, loops and locally owned collection operations are permitted where their semantics are supported by the evaluator.
- Results are ordinary Moth values. Structural resource Strings remain structural until a builder resolves them.
- Const functions do not generate declarations, tokens, AST or executable source. They are typed value computation, not macros.
- V1 admits monomorphic free functions with ordinary value parameters. Defer source generics, receiver methods, mutable-reference parameters, Wiring contracts, fallible `Error!` return channels and other non-value ABI contracts. Keep ordered multiple success slots if their existing native result path can be reused directly. Do not emulate tuples or introduce alternate result carriers.
- `assert()` and ordinary checked operations participate in const execution. An observed failure in a required call becomes a diagnostic with the exact operation site and const-call chain. A failed speculative release fold preserves the runtime operation instead of introducing a release-only runtime-failure diagnostic.
- Validate the full body, including branches unreachable for any particular call. A function with a forbidden operation is rejected at the declaration even when that operation never executes with the arguments supplied.
- No const function is emitted as a callable runtime function. Its compile-time representation may be discarded when no consumer needs it. Ordinary functions remain available to their runtime backends.

### Dependency graph and declaration order

Extend the existing Stage 3 dependency graph, do not add a second sorter or a demand-driven call-graph scheduler.

- Const functions and explicit top-level constant declarations are graph participants. Existing type-surface, capacity, file-value and nominal dependencies stay in the same graph.
- Const function bodies contribute top-level constant references and const-function calls to their dependency edges. Ordinary function bodies remain excluded.
- Const functions may reference const functions and constants in the same file in either source order. A constant may call a const function declared later in the same file.
- Preserve the existing direct same-file forward-reference restriction between ordinary constant declarations. An indirect dependency through a const function is legal when the combined graph is acyclic. Document and test this distinction.
- Direct or mutual const-function recursion is rejected as a dependency cycle in V1, even if it would terminate for particular arguments. Include cycles containing constants, const functions and imported dependencies.
- Type and signature prerequisites must be available before dependent const bodies are prepared. Do not blindly move constants ahead of nominal type identities, aliases or other semantic prerequisites.
- The graph is computed from the owning prepared source and binding facts. Avoid a second expression parser or inaccurate symbol-name heuristics. Parameters, locals, fields, shadowing restrictions, namespace accesses, imports and constructed nominal values must not create false graph edges.
- Report a useful ordered cycle chain and annotate the participating source call/reference spans. Preserve stable source-order tie breaking for independent nodes.

### Determinism, budgets and observable parity

One bounded evaluator implements the language's numeric, string, cast, collection, control-flow, assertion and template-value semantics across its supported subset. Calls with the same typed inputs and available constants must return the same value or failure independent of profile, backend and evaluation entry point.

Use deterministic accounting, not wall-clock deadlines, for the user-visible budget contract. Track executed work units, produced/retained value allocation and call-frame depth. Every required evaluation must either complete or issue a diagnostic, never hang or panic on user input. Release-only speculative evaluation may refuse and preserve the original call.

Add optional compiler-owned project configuration for a larger work allowance and memory allowance, tentatively `const_eval_work_limit` and `const_eval_memory_limit`. The implementation must validate names, integer domains, minima and finite safety maxima through the existing config schema and compiler-owned `FrontendOptions`. Defaults and maximums are to be selected using targeted benchmark measurements in the configuration slice, not guessed as permanent language constants in this plan. The executor also has a non-disableable nesting guard and an aggregate compilation work safeguard. No option permits unlimited execution.

A release speculative attempt stopped by a budget emits a structured `DiagnosticSeverity::Note` with the call-site span, the exhausted resource and its effective limit. Explain that compilation remains correct, the call will execute at runtime and the user may raise the relevant project setting to seek additional optimisation. Notes must be deterministic, deduplicated by unique call site, capped per module and emitted only for actually attempted budget-limited folds. Do not generate warnings for nonconstant arguments, impure functions or disabled optimisation. Preserve notes in successful diagnostic reports and make them visible in appropriate CLI/dev-server reporting without converting them into primary errors.

### Ordinary function optimisation

- Dev: no speculative ordinary source-function interpretation. Keep existing required constant, Core operation and template folding unchanged.
- Release: conservatively classify each ordinary function's entire transitive call graph for const-safe operations, without requiring a `#|` annotation. Only proven eligible functions with known constant argument values are candidates. Try each eligible call under the optimisation budget.
- A successful fold replaces the call and its result slots with normal folded values. A refused fold leaves the original HIR operation and all observable runtime checks intact. Never evaluate IO, clock, randomness, external mutation or arbitrary user host functions at compile time.
- Begin with directly known constant inputs. Extend to local dataflow-proven constant arguments only after a separate evidence-backed optimiser improvement. Preserve execution order and visible failure semantics.
- Perform optimisation within compiler ownership before analyses and publication that depend on runtime call reachability, provenance and borrow facts. Recompute or derive affected facts from the final representation. Do not let a backend or the build system reclassify foldability.

## Existing code and constraints

Read the current main-derived branch before every slice. These are ownership landmarks, not instructions to copy the current implementation indiscriminately.

- `src/compiler_frontend/headers/header_dispatch.rs`, `types.rs`, `constant_dependencies.rs`, `ordering_hints.rs` and `src/compiler_frontend/module_dependencies.rs`: header classification, reference hints and one dependency graph. Existing ordering excludes ordinary function bodies and rejects direct same-file forward constant references.
- `src/compiler_frontend/ast/module_ast/environment/builder.rs` and `constant_resolution.rs`: Stage 4 resolves constants before normal function signatures and later AST body emission. This is the primary ordering obstacle.
- `src/compiler_frontend/ast/module_ast/emission/emitter.rs`: ordinary function-body parsing and const-template folding after the environment is built.
- `src/compiler_frontend/ast/const_eval/mod.rs` and `external_calls.rs`: required/opportunistic expression evaluation, staged candidate publication, iterative walkers and compiler-owned Core operations.
- `src/compiler_frontend/ast/const_values/store.rs`: canonical module-local folded values, including structural String pieces, collections, records and choices. Its dense IDs are not portable across modules.
- `src/compiler_frontend/hir/functions.rs`, `statements.rs`, `expressions.rs`, `terminators.rs` and `hir/hir_builder.rs`: typed executable vocabulary with control-flow blocks and native result slots.
- `src/compiler_frontend/ast/generic_functions/materialisation/` and `module_compilation/generated/provider_materialisations.rs`: source-owned retained bodies, stable semantic closure, generic request materialisation, immutable provider publication and identity rebasing.
- `src/compiler_frontend/public_interface/model.rs`, `module_compilation/artefact.rs` and `src/build_system/create_project_modules/module_artifact_store.rs`: stable callable contracts, completed interface publication and independent compiler metadata lanes.
- `src/compiler_frontend/compiler_messages/diagnostic_severity.rs`, `diagnostics/schema.rs`, `diagnostic_bag.rs` and renderers: `Note` already exists. Add a typed diagnostic family rather than inventing a new severity or rendering-only string.
- `src/compiler_frontend/module_compilation/options.rs`, project config schema and validation: compiler-owned profile and template loop limits are established examples of settings crossing the build/frontend boundary.

## Architectural rules

1. One canonical semantic function body and one authoritative const result value representation. Do not keep parallel AST/HIR functions or interpreter-only value graphs.
2. Reuse compiler-owned operation semantics with small AST and HIR adapters. Do not implement new, independently drifting numeric, cast, Text or template rules inside the interpreter.
3. Early const-function HIR preparation must use the same type checking and lowering owner as ordinary function bodies. Prepare each demanded function body once per compilation and reuse it for every call.
4. No donor-local `TypeId`, `PathId`, HIR ID, resource index or live AST/TIR store crosses a module interface. A published implementation must use stable identities plus an owned, transportable declaring-module closure, with consumer-local preparation when needed.
5. Share the generic materialisation **pattern**, not its entire implementation or a new generic framework. Generic templates reparse a retained body because concrete types change. Monomorphic const bodies should be prepared at most once per consuming semantic context and evaluated repeatedly without reparsing.
6. Preserve transactional folding. Failed required execution publishes no partial constants. Refused optimisation changes no HIR or diagnostics except for its deliberate low-priority budget note.
7. Const execution is target-independent. Backends never evaluate const functions and receive no compile-time-only functions as runtime exports. Folded structural Strings and stable synthetic provenance survive public projection and builder handoff.
8. There must be a single place to decide whether a callable is const-safe, consumed by declaration validation and the release optimiser with differing failure policy. No host-call allowlist keyed by string spelling.
9. A private helper reachable from an exported const function remains private to other source modules but is retained inside the export's executable dependency closure.
10. A provider implementation or dependency change affecting a folded exported const result must invalidate the consumer, even when the public signature is unchanged. Reuse the existing provider dependency/invalidation design. Do not create a speculative new persistent build cache.

## Work packages

### 0. Baseline, ownership map and Stage 4 feasibility gate

- Record current branch, profile/toolchain, relevant test cases and frontend benchmark baselines. Read the routed canonical language, compiler, build and diagnostics documents. Identify the active main state rather than assuming paths or stages are unchanged.
- Trace the precise Stage 2 reference capture, Stage 3 graph edge construction, Stage 4 nominal and alias resolution, constant resolution, function signature registration, body parsing and HIR lowering sequence.
- Build a minimal internal experiment: one preceding constant, one const-capable function and one following constant that calls it. Prove the compiler can prepare that function's typed executable body before folding the second constant, without cloning all declarations or parsing the body twice.
- Map circular prerequisites: aliases whose capacities use constants, nominal members, trait identities, signatures with constant defaults and nested const calls. Design an incremental readiness boundary that publishes required type/signature facts without accidentally admitting incomplete executable bodies.
- Compare small early-HIR preparation options. Prefer extracting a callable preparation operation from the existing owner. Stop and record a concrete alternative if a shared HIR path demonstrably requires parallel AST/HIR semantics or broad duplicate state.
- **Gate:** document exact owner changes and a runnable regression proving one prepared function body is shared by at least two constant calls. No large interpreter implementation before this passes.

### 1. Grammar, callable classification and static eligibility

- Extend token/header recognition for `#|` without colliding with constant bindings, template delimiters or ordinary `|` signatures. Preserve exact declaration span and source identity.
- Add one const/ordinary callable fact to header, canonical signature, type-checking, call target and public semantic surfaces. Apply it consistently to local, imported and re-exported callables. Avoid mirrored booleans if a meaningful enum already owns classification.
- Reject unsupported V1 callable forms at the declaration with typed diagnostics: source generic const function, const receiver method, mutable-reference parameters, Wiring contract and error-return contract.
- Check all executable operations and transitive call targets for const eligibility, including operations in never-executed branches. Reject runtime-only bindings, runtime function calls and untrusted host effects with declaration/call labels.
- Validate const calls in any binding context as compile-time-required, including an ordinary runtime declaration, nested expression, template or ordinary function. Runtime-dependent arguments are a source error at the call.
- **Gate:** parser/typing tests cover local, imported, forbidden and never-called declarations, including accurate spans and no runtime-call escape.

### 2. One const-aware dependency graph

- Extend existing initializer reference ownership to collect const-function body dependencies. Distinguish a callee from a variable of the same spelling using scope-aware prepared facts. Do not duplicate expression parsing or invent name-only edges.
- Permit same-file forward edges from const functions to const functions and constants and from constants to const functions. Preserve direct constant-to-constant source-order rejection as already designed.
- Resolve type, constructor, namespace, imported and file-value edges through existing canonical binding machinery. Respect support facades and public/private boundaries.
- Reject direct self recursion and all strongly connected const/constant cycles at the graph owner. Emit source-anchored ordered chains, including transitive cycles through helper functions.
- Keep ordinary function bodies and implicit start outside this graph. Preserve deterministic sorting of unrelated headers and existing capacity/alias semantics.
- **Gate:** tests exercise forward references in both directions, the indirect forward constant case, local and multi-file cycles, false-positive local references and imported providers.

### 3. Stage 4 const declaration preparation

- Introduce the minimum const-declaration readiness lane within the existing AST environment orchestration. It consumes Stage 3's order and uses resolved nominal identities and type surfaces, without waiting for unrelated runtime body emission.
- Prepare, validate and lower each const function exactly once through the canonical function-body lowering path. Store its executable in a compiler-only owner indexed by canonical local identity.
- Evaluate constant initialisers as their graph dependencies become ready. Connect parameter and local value bindings to the executor without adding special const-only expression semantics.
- Maintain normal availability of signature defaults, trait metadata, generic nominal types and compiler-owned Core operations. Avoid retaining a second mutable declaration table.
- Preserve ordinary HIR emission and current function registration for all runtime functions. Ensure const-only bodies cannot enter runtime output.
- **Gate:** exact-body-prepared-once instrumentation, source-order diagnostics and parity tests across independent functions and constant calls. Re-run existing alias, trait, nominal and constant suites.

### 4. Bounded typed HIR execution core

- Add a compiler-owned const-function execution service over the existing HIR vocabulary. Use an explicit frame stack for functions, basic blocks, instruction position, locals and ordered return slots.
- Use producer-local staged values backed by the existing `ConstValueStore` payload domain. Commit only after the complete required expression succeeds. Preserve provenance and structural Strings on every intermediate value and result.
- Implement supported literal and unary/binary evaluation, local assignments, reads, casts, branches, loops, pattern/option operations, returns, nested calls and deterministic assertions. Add nominal/aggregate construction and projection once the scalar core works.
- Factor shared numeric and trusted Core operation evaluation into clear compiler-owned semantic helpers where AST and HIR need exactly the same behaviour. Leave AST expression folding and TIR traversal with their existing owners.
- Enforce instruction/work, allocation, nesting and total compilation guards without Rust recursion overflow. Distinguish a budget refusal, a const-rule diagnostic and infrastructure failure. No user-dependent panic.
- **Gate:** deterministic scalar, branch, loop, checked arithmetic and assertion cases pass in debug and release. A multi-call failure cannot publish partial results. Stress deep expressions and long loops safely.

### 5. Composite values, templates and structural resources

- Evaluate nominal constructors, records, collections, collection mutation and field/index access using already resolved type semantics. Preserve collection ordering and deterministic map operations where supported.
- Support parameter-dependent constant template production by using the existing AST-owned TIR classification and fold authority. Do not reparse template syntax inside HIR or flatten structural Strings to URL text.
- Establish the smallest supported AST/TIR-to-HIR handoff for parameterised constant templates. If ordinary HIR is missing an operation required to represent the template, extend the authoritative lowering seam instead of inventing an interpreter-only template engine.
- Reject concrete-text operations on unresolved Resource/SiteRoot pieces where existing constant rules require final builder context. Preserve exact resource origin through results, public projection and final output.
- Add an integration example returning a nominal component-like record with body String and `{String}` stylesheet resources. Its caller consumes both outputs as ordinary data. This plan does not implement builder head slots.
- **Gate:** same module component example, nested templates, branches and loop-constructed collections work with correct structural resource identity and no second TIR representation.

### 6. Cross-module const executable publication

- Extend public callable semantics with const classification, using stable function identities and canonical type surfaces.
- Extend the existing compiler-produced module metadata/publication boundary with a const executable contract and its transitive declaring-module semantic closure. Reuse the generics lane's retained source-token owner, stable signature/materialisation blueprints, visibility snapshot, origin rebasing and publication collision checks where appropriate.
- Do not publish raw local HIR or local numeric IDs. Materialise the provider body into a consuming semantic context once per unique provider identity and resolved typing context, not once per call. Run it through the same typed preparation and executor used locally.
- Include all reachable private const helpers and module constants in the closure without making their declarations visible as user imports. Preserve resource origins and cross-module source spans for diagnostics.
- Re-exported const functions must keep the **original** definition identity, rather than acquiring duplicate executable owners at facade boundaries. Reject missing, duplicate and mismatched provider artefacts before evaluation.
- Ensure dependency tracking notices exported const body and transitive helper/constant changes that alter consumers' compile-time results. Build tests should mutate provider implementation while keeping its signature fixed and verify changed consumer output. Avoid claiming persistent cache invalidation infrastructure exists where it does not.
- **Gate:** project module, source package, facade re-export, private helper, structural resource and provider-update tests pass without donor-local ID leakage or per-call reparsing.

### 7. Release ordinary-function folding

- Add a compiler-owned pre-validation HIR optimisation pass enabled only for `FrontendBuildProfile::Release`. Reuse the executor and conservative whole-function/transitive const-safety classifier, with separate required versus speculative refusal policy.
- Detect directly available constant argument values, evaluate eligible ordinary calls and replace the result values using the established HIR rewrite/lowering APIs. Preserve zero/one/multiple success slots and side-effect order.
- On failure, inability to prove constness, structural text refusal or budget refusal, leave the exact runtime call and its semantics intact. Only budget refusal produces the advisory specified below.
- Ensure downstream borrow validation, reachability, runtime imports, synthetic provenance, failure summaries, link facts and backend emission consume the final optimised HIR. No stale facts from a removed call.
- **Gate:** debug retains source function calls, release eliminates successful eligible calls, impure or dynamic calls remain runtime and failing or bounded calls behave the same across profiles.

### 8. Budget configuration and release optimisation suggestions

- Introduce typed compiler-owned work and memory budgets, explicit hard safety maxima and project config fields. Select defaults and permitted maxima after measuring representative const template, component and arithmetic workloads.
- Project config parses positive bounded numeric values and projects them into `FrontendOptions`. Keep direct-module and synthetic compilation defaults consistent. Document units, precedence and the difference between per-call limits and global safeguards. Do not abuse `template_const_loop_iteration_limit` as a general function-work budget.
- Add typed diagnostic reasons and stable codes for required evaluation budget exhaustion, invalid const operation, nonconstant call arguments, recursion/cycles, observed assertion/numeric failures and release optimisation budget advice.
- Emit release budget advice as `DiagnosticSeverity::Note`, not `Warning` or `Error`. Include location, operation, exhaustion category, effective limit and concrete setting to change. A note must not alter exit status. Deduplicate by source call-site and bound overall note count. Test terse, normal and dev-server rendering.
- Guard against unbounded speculative compile work through a total module budget and deterministic short-circuit policy. Distinguish attempts that never started from those that exceeded a budget.
- **Gate:** increased configured budget produces an extra successful fold in a benchmark-sized test, a smaller budget emits a single useful Note with runtime fallback and required calls issue an Error instead.

### 9. Documentation, performance and release validation

- Update canonical const/functions, template, declarations and compiler architecture references with the final syntax, dependency rules, compile-only semantics, budgets and cross-module contract. Then update cheatsheet and progress matrix to match implemented reality. Avoid advertising unfinished slices as implemented.
- Instrument preparation count, required vs speculative attempts, success/refusal reasons, evaluation work, scratch allocations, resource payload transport and cross-module body materialisations. Keep instrumentation feature-scoped and away from hot data if measurements show cost.
- Benchmark unchanged ordinary functions, constant-heavy modules, template-heavy docs, repeated const calls, cross-module packages, a large refused release fold and both build profiles. Preserve current benchmark schema and compare against a clean baseline using `just bench-frontend-check` or the project benchmark runner appropriate to the branch.
- Validate JS and supported Wasm backend parity for final folded outputs. Do not require a backend to execute const-only functions. Verify compiler-owned numeric semantics, resource handling, emitted runtime imports and no const-only runtime symbols.
- Perform the final Slice review: search for duplicate AST/HIR adapters, repeated parsers, stale signature paths, unused flags, cloned donor contexts, overbroad ownership and redundant tests. Delete obsolete mechanisms rather than preserving compatibility layers.
- Remove the plan and its roadmap entry in the same commit that completes it, per roadmap policy. Durable language semantics remain in canonical documentation.

## Test ownership and acceptance matrix

Follow `docs/src/developer-docs/style-guide/testing.mtf`. Add focused tests under owning subsystem `tests/` modules, end-to-end Moth language cases under `tests/cases/` and multi-module harness cases only where smaller tests cannot prove the boundary. Extend existing cases before creating new files. Every test must name a realistic regression and assert its relevant output or diagnostic contract.

- **Syntax and type:** valid `#|`, ambiguity with `#=` and `|`, unsupported const method/generic/contract, imported/re-exported const kind and accurate source spans.
- **Graph:** const-to-const same-file forward, const-to-later-constant, constant-to-later-const-function, direct constant forward rejection, transitive indirect forward allowance, direct/mutual recursion and mixed constant/function cycles.
- **Eligibility:** ordinary function in unreachable branch, runtime global, host IO, private helper call chain, local mutation allowed, external pure Core permitted, untrusted similarly named external function rejected.
- **Evaluation:** parameter-dependent control flow, nested calls, loops, zero iteration, budget exhaustion, assertions, checked numeric traps, cast parity, ordered multi-return slots if supported and call-stack diagnostic labels.
- **Composite:** nominal structs, Options, collections, nested strings, templates with arguments, resource and site-root transport, structural text comparison refusal and no URL pre-resolution.
- **Publication:** same-module, across project roots, source-backed packages, facades, re-exports, private transitive helpers, provider fingerprint changes, missing published body, identity remap and malformed provider collision.
- **Profile:** debug source call retained, release call folded, impure/dynamic/failed release calls unchanged, runtime output parity, no release-only errors for speculative evaluation.
- **Advice:** exactly one Note for repeated failed budget attempt at a call site, stable code, suggestion to raise budget, no warning or error status, no note for non-budget refusal and proper cap under many candidate calls.
- **Robustness:** large loops, deep/nested values, long function chains, repeated compilation, parallel project builds, output/resource size bound and no user-induced panic or Rust stack overflow.
- **Performance:** no ordinary debug compile regression beyond measured and accepted metadata overhead, no body reparse per repeated call, no whole-context clone per source function and stable memory under repeated failed folds.

## Validation and review gates

Use the repository's own `validation.mtf` for exact gate selection. Typical commands:

```bash
cargo test -p moth --lib
cargo run --quiet -p moth-cli --bin moth -- tests --terse
cargo run --quiet -p moth-cli --bin moth -- check docs --terse
just fmt-check
just clippy
just bench-frontend-check
```

At important integrated checkpoints run `just validate`. Before merging completed implementation run the final `just validate-full` gate, include applicable feature-lane tests and capture profiler evidence when changing hot constant-folding paths. Documentation-only plan creation does not imply the code-bearing gates ran.

For each work package, record modified owners, tests and commands, measurements where performance is relevant, remaining blockers and the next action in the small status block. Complete one slice at a time. Before advancing, compare the implementation against the architecture constraints and remove any temporary duplicate paths.

## Stop conditions

Stop the implementation for design review if any of these occur:

1. Preparing const bodies before constants requires a parallel AST/HIR compiler, duplicated semantic resolution or permanently retained incomplete environments.
2. A cross-module solution leaks donor-local HIR or IDs, recompiles a const body per call or cannot preserve private dependencies and structural resource identities.
3. The evaluator requires independent implementations of numeric/cast/template semantics or a second retained general-purpose value domain.
4. Debug and release can produce different observable runtime results or speculation can introduce extra runtime-failure diagnostics.
5. The budget model cannot bound total compile work, report required failures or preserve runtime calls on speculative refusal.
6. Constant-dependency collection relies on false-positive-prone identifier scans instead of established source/binding ownership.

A stop condition requires an evidence-based design correction and plan update before more code. Never bypass it with compatibility wrappers or broad defensive fallback layers.
