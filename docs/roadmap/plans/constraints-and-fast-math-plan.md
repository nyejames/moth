# Constraints and fast arithmetic implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task by task. Track each task using its `- [ ]` checkboxes.

**Status:** Queued. Activate after the roadmap's required typed-expression, native-result, directive-attachment and HTML page-directive foundations have merged.

**Goal:** Deliver `$fast_math`, `$safe_math` and `$infallible` as three small compiler-owned constraints, plus a portable fast-arithmetic contract on supported JS and Wasm paths. Ordinary numeric behaviour stays unchanged.

**Architecture:** Attach source constraints through the existing directive pipeline, select numeric semantics in typed operations, carry them through HIR and existing call-summary convergence, then lower with the existing backend numeric owners. Keep semantic mode, error/trap delivery and optional proof-based optimisation separate.

**Tech stack:** Rust compiler/frontend and HIR, HTML-JS/HTML-Wasm backends, Moth integration fixtures, developer and user `.mtf` documentation.

**Repository destination:** Replace `docs/roadmap/plans/constraints-and-fast-math-plan.md` with this plan. This is plan **1 of 3**: constraints and fast arithmetic, then `numeric-optimisation-plan.md`, then a separately designed Core Math fast-counterparts plan. Only the first two plans are being prepared now.

## Global constraints

- Follow `AGENTS.md`, `index.md`, `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`. Read `docs/compiler-design-overview.md`, `docs/build-system-design.md` and the canonical language owners routed by `docs/developer-docs/language/overview.mtf`.
- Moth keeps one way to do things. No effects engine, general-purpose constraint solver, trait changes, backend-specific source semantics, new block syntax or speculative `$pure`, `$no_io` and `$no_wrap` implementation.
- Preserve ordinary checked arithmetic, exact I8-I64/U8-U64/F16-F64 identities, D15 result domains, typed recoverable errors, memory safety and existing foreign-boundary validation. No implicit numeric widening floor.
- **Dec is entirely legal inside `$fast_math` but never enters fast mode.** Every Dec operation and conversion involving Dec retains ordinary type, scale, rounding, recoverable-error and local-catch behaviour. Numeric operands resolved into a Dec domain use ordinary semantics. No Dec fast helper or special Dec failure delivery.
- Do not add the proposed `fast_*` Core Math APIs here. Provider trust needed to verify constraints is in scope. A future third plan owns counterpart selection, implementation and benchmarking.
- For each behaviour change, first add a failing regression, run it, implement the narrowest fix, rerun focused tests and review the Slice. Commit small coherent slices. Keep benchmark output and activation baseline in working notes rather than queued status.

## Review focus and acceptance hazards

1. **Proven safe versus fast:** integer modular overflow cannot create a recoverable Error, but zero division, signed `MIN // -1` and finite-float violations remain semantic traps. `NumericFailureMode::Trap` alone is insufficient to encode fast semantics.
2. **Transitive constraints:** a callee's fast operations cannot be hidden by a caller's checked mode, `catch`, folding, inlining, generic materialisation or re-export. A fast annotation with no reachable fast operation is vacuous.
3. **Infallibility:** locally handled ordinary errors are permitted, including in an `Error!` function. The handler and every reachable call must also be infallible. A declared `Error!` slot remains part of the ABI and catch eligibility.
4. **Correct width and order:** use semantic result widths rather than JS carrier widths. Preserve each intermediate operation's finite check, first fault, signed zero, one-time evaluation and trap-before-store ordering.
5. **Host boundary:** a trap faults the builder-selected execution instance, not necessarily a source module or the host process. Lappet's ABI requires quarantine and no subsequent guest calls on that instance. No rollback or normal guest cleanup is promised.

## 1. Accepted source contract

### Directive placement and reach

- `$fast_math`, `$safe_math` and `$infallible` are parameterless source modifiers: use `$fast_math`, never `$fast_math()`.
- Reuse the existing contiguous following-line directive run. Different names may share a line with whitespace, order does not matter and a blank physical line, scope boundary or EOF breaks attachment. Repeating any name on one target is a two-span diagnostic, including repetitions without arguments.
- Valid targets are functions, methods, generic functions and complete executable statements. A compound statement includes its condition, branches and body. Function annotations cover the function body. Nested callable declarations retain their own contracts. Runtime root statements are valid targets.
- Imports, parameters, fields, types, export groups, compile-time-only declarations and template heads are not targets. No file-level mode is introduced. Preserve the existing parser rule for nested template operands when finding the next directive.
- Lexical mode covers caller-authored operands, initialisers, RHS expressions and compound write-backs. Callee bodies keep their own declarations. Explicit `cast`, comparisons, logical operations, assertions, collection operations, memory checks and foreign validation keep their current contracts.

```moth
$fast_math $infallible
increment |value I32| -> I32:
    return value + 1
;

$safe_math
recover |left I32, right I32| -> I32:
    value = left + right catch then 0
    return value
;
```

### Constraint meanings

- `$fast_math` selects **fast semantics only for resolved fixed-integer and binary-float arithmetic**. It does not make unrelated calls, `cast` or Dec operations fast. A fast numeric failure traps rather than taking an implicit recoverable Error edge. It never weakens storage or bounds invariants.
- `$safe_math` guarantees that no executed or statically selected reachable computation uses fast numeric semantics, including through calls and future trusted `fast_*` contracts. It does **not** guarantee successful arithmetic. Source operands whose values previously wrapped are still valid values, with no history taint.
- `$infallible` guarantees that no recoverable error escapes its boundary and no reachable Moth-semantic panic/trap can execute. It admits proven-total operations and locally recovered errors only if the complete recovery path satisfies the guarantee. It excludes environmental exhaustion, external termination and implementation faults, and does not prove termination.
- A statement boundary ends at statement completion or escape. A function boundary ends at its return. Catch handles recoverable Error only, never a trap. Work done before a fault is not undone.
- `$safe_math` and `$fast_math` may coexist: first select lexical fast mode, then reject the boundary **only if an actual relevant fast operation/contract is reachable**, including fast range-guard trap delivery. Dec-only arithmetic with both directives is valid. `$fast_math` and `$infallible` likewise coexist when the computation is proved trap-free.
- A no-slot function is not automatically infallible. An `Error!` function may satisfy `$infallible` while keeping its declared return contract. Authored `catch` eligibility is distinct from actual failure production and must continue to obey current expression-level call-count and nesting rules. A currently unreachable handler still receives ordinary source validation.
- Check semantic reachability after static selection, but before optional optimisation erases evidence. A selected fast operation counts even if later constant-folded or dead-result-eliminated. Inactive ordinary branches still receive syntax and type validation. Preserve call provenance where folds occur before constraint verification.

### Constraint proof and external calls

- Use a bounded, target-independent proof policy: exact type bounds, valid constants, existing mandatory failure discharge and narrow local facts. Unproved potential errors or semantic traps fail `$infallible`. No SMT, general loop induction or whole-program path solving.
- Follow all direct, generic, recursive and module calls using the existing monotone convergence. Private and exported annotations must be checked at their declaration, even without a selected caller. Facades, public interfaces and re-exports preserve guarantees.
- Moth-source packages are analysed normally. Opaque foreign JS/native bindings cannot establish absence of fast operations or traps by virtue of their type signature. Only explicitly registered compiler/builder-owned contracts provide trusted closed evidence. A wrapper around an unknown binding cannot launder its behaviour.
- Distinguish declared guarantees, known positive hazards, proved absence and provisional/unknown summaries. Initial `escapes_builtin_failure: false` is a convergence seed, **not evidence** for `$infallible`.

## 2. Numeric semantics V1

The result domain is the **resolved semantic domain**, never the destination variable or backend carrier. Both modes use the same typing, operator eligibility, evaluation order and per-operation rounding. Unsupported pairs remain unsupported. Fast semantics are portable Moth rules, not unchecked host arithmetic.

### Fixed integers

For an N-bit result, `wrap_N(x)` takes the mathematical value modulo `2^N` and reinterprets the high bit for signed types. No published integer leaves its declared range. Preserve D15 equal-type widths (`U8 + U8 -> U8`) and smallest supported mixed domains (`I8 + U8 -> I16`).

- `+`, `-`, `*` and permitted unary `-`: ordinary mode checks overflow, fast mode returns `wrap_N` with no numeric error or trap.
- Unsigned `//`: ordinary quotient, fast trap on zero divisor.
- Signed `//`: truncation toward zero, fast trap on zero divisor or `MIN // -1` for the result domain.
- Unsigned `%`: ordinary remainder, fast trap on zero divisor.
- Signed `%`: dividend-sign remainder, fast trap on zero divisor only. `MIN % -1` is exactly zero, even if the host primitive would fault.
- Bounded integer `^`: ordinary mode stays checked. Fast mode uses modular exponentiation, trapping on a negative signed exponent. `0 ^ 0 == 1`. Do not construct a huge host power before truncation.
- Integer `/`: produces the already-specified real-division F64 domain, then applies the binary-float rule. `MIN // -1` does not affect real division.
- Unsupported unsigned negation, Byte arithmetic and unsupported integer/float operators stay compile errors.

### Binary floats

F16/F32/F64 round at **every language operation** in their resolved precision and reject a non-finite published result. Preserve subnormals and signed zero. Intermediate host-carrier invalidity may exist only until the same operation validates it.

- `+`, `-`, `*`: normal rounded result, fast trap if non-finite.
- Unary `-`: normal finite negation, no fast numeric trap.
- `/`, `%`: normal rounded quotient or dividend-sign remainder, fast trap on either signed zero divisor or a non-finite result.
- `^`: the existing scalar power, rounding and domain rules, fast trap on invalid/non-finite result. Fractional/negative float exponents do not inherit integer exponent restrictions.
- Comparisons keep ordinary semantics. No reassociation, approximate reciprocal, flush-to-zero, fused rounding or lost signed zero is implied by `$fast_math`.

`1.0 / (a * b)` must fault at the multiplication if that multiplication is non-finite, even when the backend could compute a finite final result after continuing. A float range's uncommitted candidate retains its documented termination exception below.

### Dec and mixed-domain operations

Dec has **no fast numeric mode**. An operation resolved to a Dec domain, including accepted Dec/integer mixing, uses exactly the ordinary coefficient/scale/rounding and recoverable-failure contract. Dec division by zero and negative Dec power use the current ordinary Error recovery. No numeric wrapper, type promotion, trap conversion or fast provenance is introduced for Dec. Dec addition and a locally caught Dec division are valid inside `$fast_math`, subject to normal syntax and error handling. `$safe_math $fast_math` accepts a Dec-only computation when its transitive calls also meet the safe guarantee.

An implicit compound conversion involving Dec also retains ordinary semantics and failure delivery. If the resolved arithmetic itself uses fixed integer or binary float before an otherwise legal non-Dec write-back, apply the arithmetic mode to that operation, not to unrelated conversions.

### Compound stores, explicit conversions and loops

- Evaluate place and RHS once in existing order. Perform arithmetic, then implicit write-back, then store. A trap before the store leaves the target unchanged but does not roll back prior effects.
- Fast integer-to-integer compound write-back reduces to destination low bits and destination signedness only for conversions already permitted by Moth. Fast float-to-integer write-back truncates toward zero and traps if out of range, never saturates. Integer-to-float and float-to-float write-back use ordinary precision rounding and trap if non-finite. Conversions involving Dec stay ordinary. Other total conversions stay total.
- Explicit `cast` and `cast!` retain their existing ordinary failure policies under fast mode. `/=` on integers uses real division, while `//=` uses integer quotient.
- Range loops retain one-time header evaluation, direction, signed step, last-valid-endpoint termination and the correct source-index/ordinal binding. Authored header/body arithmetic uses its lexical mode. Generated step/counter updates must **never wrap**. The existing full-width endpoint-distance guard stays before update. Zero-step, no-progress and otherwise unavoidable semantic range failures become traps under `$fast_math` instead of recoverable Error. Keep floating out-of-range/non-finite *uncommitted* candidates as normal termination and trap on an in-range no-progress candidate.
- Allocation sizes, pointer/offset calculations, collection bounds and capacity arithmetic are never fast operations.

### Trap boundary

A semantic trap ends the current guest call and permanently invalidates the **execution instance selected by the builder's lifecycle**, not necessarily one Moth source module. It cannot be caught as Moth `Error`. The host may continue and construct a replacement instance. An embedder that supports repeated guest calls must quarantine the instance and issue no subsequent guest call or guest cleanup helper against it. No rollback, normal guest cleanup or safe reuse is promised. Prior host IO remains observable. JS and Wasm must expose equivalent Moth behaviour, though host reporting and transport may differ. Do not turn trapping into `assert` or a compiler bug.

Implement this boundary using the existing builder/host integration rather than introducing a new guest runtime. Coordinate runtime semantics with [Lappet host ABI faults](https://github.com/nyejames/moth/blob/main/crates/lappet/docs/host-abi.md#faults) and [Lappet instance ownership](https://github.com/nyejames/moth/blob/main/crates/lappet/docs/design.md#instances-resources-and-scheduling). Lappet's concrete runtime fault-disposition checkpoint is still open: implement only current trap delivery and document the instance contract, do not invent a reset API.

### Backend and optimiser rules

- A transformation must preserve typed values, source-visible work, order of first error/trap and non-executed branches. Dead results are not a licence to erase possible traps. Inlining preserves the callee's arithmetic mode. Future LLVM and RISC-V targets must implement these semantics without UB or instruction-specific gaps. Those backends are **not** work in this plan.
- JS: reuse width/domain logic in `numeric_carrier.rs` and demand-driven helper usage. Use exact low-word multiplication such as `Math.imul` for 32-bit modular products and width-normalised BigInt for 64-bit operations. Number multiplication followed by truncation loses low bits for some U32 inputs. Never publish an out-of-range fixed-width value.
- Wasm: use native modular integer arithmetic where exact, with explicit zero and signed MIN/-1 quotient traps. Handle `MIN % -1 == 0` explicitly where required. Finite-float checks stay at each semantic result boundary. F16 or other unsupported target surfaces retain precise capability errors rather than changing semantics.
- Regression counterexamples: signed division by 8 is not an arithmetic shift on negatives, `x // x` still traps at zero, signed `MIN // -1` cannot become wrapping negation, dead results can trap, and untaken trapping branches remain untaken. An N-bit modular multiply-by-8 may legitimately become an N-bit shift when equivalent.

## 3. Existing owners and implementation boundaries

Confirm these paths against the activation revision. The main-branch inventory at plan writing found:

- Directive contracts: `src/compiler_frontend/style_directives/specs.rs`, `builtins.rs`, `headers/file_parser.rs`, `ast/statements/body_dispatch.rs`. Extend the existing attachment and diagnostics machinery, **not another parser**.
- Numeric resolution: `src/compiler_frontend/datatypes/numeric_operators.rs` for operation/domain and `numeric_operation_cannot_fail`. Keep this mandatory type-based predicate distinct from optional proofs. Runtime folding starts in `src/compiler_frontend/ast/const_eval/mod.rs`.
- Numeric HIR: `src/compiler_frontend/hir/numeric.rs`, `hir/hir_expression/runtime/operation_lowering.rs` and `hir/hir_expression/numeric.rs`. `HirNumericOp` currently contains operator/domain. Add a compact `Ordinary|Fast` semantic flag (or equivalently small existing-owner representation), excluding Dec at resolution. `NumericFailureMode::{Trap,Infallible}` already controls statement delivery, while `HirTerminator::Invoke` owns recoverable calls. **Do not overload those to imply fast arithmetic.**
- Failure evidence: `src/compiler_frontend/hir/failure_facts.rs` and `hir/private_failure_lane.rs`. Preserve actual Error producers, legal authored catches and handler pruning.
- Call facts: `src/compiler_frontend/public_call_summary.rs`, `src/compiler_frontend/public_interface/` and `src/compiler_frontend/module_compilation/generated/convergence.rs`. Extend their current worklist and existing semantic/physical fingerprints. Avoid a second global call graph or general effects schema.
- Trusted providers: `src/compiler_frontend/external_packages/definitions.rs` and provider registration such as `src/builder_surface/core_packages/math.rs`. Attach bounded, explicit callable guarantees at trusted registration, not inferred from a package name or `fast_` prefix.
- JS: `src/backends/js/js_statement.rs`, `runtime/numeric.rs`, `runtime/mod.rs`, `numeric_carrier.rs`. Wasm: `src/backends/wasm/hir_to_lir/stmt.rs`, `emit/checked_integer.rs`, `emit/checked_float.rs` and existing float-power/remainder emitters. Reuse actual helpers and capability gates.
- Existing optional backend-only proofs: `src/compiler_frontend/analysis/numeric_proofs/mod.rs`. They **must not decide source-level constraint acceptance**. Plan 2 owns checked-carrier simplification and expansion of this optional analyser.

### Required permanent documentation

Early in implementation, transfer the accepted source rules to `docs/docs/directives/constraints.mtf` and the numeric operation contract to `docs/developer-docs/numeric-backend-contract/numeric-backend-contract.mtf` with its `@entry.moth` and navigation. Update `docs/docs/directives/directives.mtf`, numbers and error/cast/range references, `docs/compiler-design-overview.md` and applicable build/interface sections. Update the cheatsheet, Basic teaching and progress matrix only where needed. The developer contract, not a roadmap plan about to be deleted, is the final authority for plan 2 and future backends.

Update `docs/roadmap/roadmap.md` to sequence this plan, `numeric-optimisation-plan.md` and a **future Core Math fast-counterparts roadmap item without a broken link**. The third plan will be written separately. Retain a clearly future-design-only note for `$pure`, `$no_io` and possible `$no_wrap` without registering them.

## 4. Execution tasks and merge gates

### Task 0 - Baseline and contract inventory

- [ ] Establish an isolated branch/worktree at the roadmap activation point. Record the exact main revision, relevant prerequisite completion, compiler capabilities and raw `just validate` baseline. Follow the validation guide for baseline exceptions.
- [ ] Map the current resolved operator/type matrix, compound conversions, range failure paths, assertion/integrity traps, external binding policy and test owners. Mark each as reuse/extend/untouched. Check D15, precision-specific F16 support and Wasm rejection paths rather than assuming universal target support.
- [ ] Freeze every unresolved implementation-only question against section 2 before changing semantics. Escalate any genuine conflict with permanent numeric or Lappet contracts instead of silently choosing target behaviour.

**Gate:** A bounded operator/domain inventory and owner map exist in working notes. No new semantics are being invented in code review.

### Task 1 - Publish permanent contracts

- [ ] Add the user and developer constraint/numeric pages above. Include all operator, Dec, range, trap, proof and backend parity rules, with a developer-only reference to future LLVM/RISC-V requirements.
- [ ] Integrate navigation and correct the checked-arithmetic, function, catch, assertion, numeric, cast, range, packages and backend overview text where rules change. Update design-scope wording so checked arithmetic remains the default and portable fast semantics are now permitted.
- [ ] Add queued/in-progress progress entries honestly and review documentation examples through the prescribed documentation build. No hand-edited generated HTML.

**Gate:** A future implementation agent can find the full semantics in permanent topic owners without this plan or conversation.

### Task 2 - Source attachment and typed mode

- [ ] Add failing parser/diagnostic tests for functions, methods, generic declarations, nested statement bodies, root statements, grouped directives, blank lines, duplicates, `()` rejection, invalid targets and different directive orders. Reuse shared attachment and two-span diagnostics.
- [ ] Add tests for caller-owned argument modes, non-inherited callee modes, explicit `cast`, Dec-only fast scopes and compound expression modes. Extend resolved numeric operations and `HirNumericOp` with minimal semantic mode. Validate incompatible domain/mode and HIR delivery combinations.
- [ ] Make ordinary/fast folding agree with runtime for modular arithmetic, F16 rounding, zero division and Dec. Optional folding must leave a runtime trap when it cannot return a value. Required-const contexts diagnose the corresponding invalid constant. Preserve fast-use witnesses when a relevant operation folds away.

**Gate:** Source to HIR has one authoritative arithmetic mode and all folded outcomes preserve the accepted contract.

### Task 3 - Semantic summaries, constraint verification and interfaces

- [ ] Add failing tests for `$safe_math` with direct, transitive, folded, generic, recursive and re-exported fast use, plus vacuous fast and Dec-only uses. Include a **fast range guard with no authored arithmetic**.
- [ ] Add failing tests for `$infallible`: total mixed-width addition, fast wrapping integer operations, unproved integer division, finite-float overflow, assertions, ordinary escaping Errors, compatible local catches, fallible fallback and an `Error!` signature that stays unchanged. Ensure legal-but-unused catch handlers still validate.
- [ ] Add deterministic unknown-foreign and trusted-provider tests, including wrappers, indirect callable values without a closed target set, recursive cycles, public methods, facade exports, dormant generic declarations and invalid declarations never called by an entry. Unclassified provider callables default to unknown, not certified-safe.
- [ ] Extend existing failure facts/call summaries with only necessary hazard and fast-use bits plus sparse witnesses. Derive facts from the final semantic operation and actual recoverable edges, never mere catch eligibility. Reuse the existing monotone generated-call convergence and keep provisional/untrusted facts distinct from certified absence.
- [ ] Validate annotations with a small target-independent proof policy before optional backend proofs. Wire final guarantees and dependencies into `PublicSemanticInterface`, its current fingerprint, retained executable and generated sidecars. Test a same-source public signature whose private helper changes from ordinary to fast or becomes trapping, plus stale cached contracts.
- [ ] Report directive boundary, the first offending operation or bounded call chain and unknown-evidence leaves through existing diagnostic plumbing. Keep generic recursion witness order stable and distinguish overflow from compound write-back.

**Gate:** The same source receives the same constraint verdict across JS/Wasm and debug/release, independent of optional proof-table enablement. Public contracts cannot go stale silently.

### Task 4 - JS fast arithmetic

- [ ] Add Node-executed boundary cases for signed/unsigned I8-I64/U8-U64 modular add, subtract, product and power, U32 precision-loss counterexamples, narrow D15 domains and every integer zero/MIN exception. Use runtime parameters so tests cannot all constant-fold.
- [ ] Add F16/F32/F64 per-operation finiteness and rounding tests, subnormals, signed zeros, compound write-back conversions, non-committed stores and earlier IO surviving a trap. Verify that `catch` cannot intercept a fast trap.
- [ ] Lower fast scalar operations through the existing JS numeric helper owner, reusing Number/BigInt normalisation and runtime demand tracking. Emit fatal delivery for actual fast traps, not an ordinary `{tag, value}` recoverable carrier. Leave ordinary checked helper/carrier optimisation for plan 2.
- [ ] Confirm pure ordinary/Dec modules do not import fast helpers, and that supported Dec operations in a fast scope still follow checked/handled paths. Test source annotation and backend helper selection independently.

**Gate:** Executed JS matches every accepted result, trap and ordering rule without leaking invalid fixed-width or non-finite values.

### Task 5 - Wasm fast arithmetic and instance faults

- [ ] Add paired JS/Wasm executable tests for supported scalar domains and shared expected outputs/traps. Explicitly test precision/aggregate operations that Wasm currently rejects with honest capability diagnostics.
- [ ] Extend current `hir_to_lir/stmt.rs` and `emit/checked_integer.rs`, `emit/checked_float.rs` to use the mode while retaining distinct ordinary checks, proven-total paths and fast traps. Guard zero, signed MIN/-1, signed remainder special case and finite float results.
- [ ] Test ranges at integer MAX/MIN, unsigned descending zero, ordinal/index limits and float no-progress. Verify generated safety counters never wrap, and no possible trap is hoisted into an untaken branch or past earlier effects.
- [ ] Test the trap boundary at the builder/host integration level where available: first call faults, repeated guest calls cannot reuse the faulted instance, host survives and a fresh instance remains possible. For a single-entry HTML run, verify termination without inventing a new invocation API. If the Lappet runtime checkpoint has not landed, exercise its available ABI seams, preserve its quarantine contract and record the unavailable runtime test honestly rather than inventing lifecycle support.
- [ ] Verify fast traps require no recoverable-numeric capability and do not loosen other Wasm capability restrictions. Regression-test signed divide/shift, `MIN // -1`, `MIN % -1`, `x // x` and dead trapping results.

**Gate:** Matched results and faults for supported JS/Wasm surfaces. Any unsupported operation is explicitly rejected, never silently changed.

### Task 6 - Integration, performance and deletion pass

- [ ] Run focused tests after each slice, `just validate` at boundaries, `just validate-full` on the integrated tree, and the documented docs gate. Record precise commands, revisions, failed or skipped tests and target exclusions.
- [ ] Benchmark ordinary versus fast integer kernels, finite floats, recursive/generic summary convergence and constraint-heavy builds separately. Measure runtime and compile-time costs without asserting a speedup in advance. Plan 2 will handle general checked-carrier optimisation.
- [ ] Inspect expression/HIR sizes, new per-call summary bytes, worklist iterations, backend helper count and diagnostic paths. **Remove** redundant scans, double mode encoding, shadowed failure facts, duplicate guards and compatibility scaffolding introduced during implementation. Prefer deletion or reuse over more indirection.
- [ ] Audit source validity and error order, old behaviour in unannotated programs, public interface cache invalidation, generated artefact parity, documentation navigation and unsupported target reporting. Complete the Slice code-review pass, including local Rust clarity and excess abstraction.
- [ ] Move lasting semantics into permanent docs, mark progress delivered only for actually supported targets and remove this finished plan and its roadmap entry in the completion commit. Keep numeric optimisation next and the future Core Math counterpart item queued.

**Final gate:** All three directives and the advertised JS/Wasm fast arithmetic contract work without changed ordinary or Dec behaviour. Convergence is reused, source acceptance does not depend on optional optimisation and repeat-call embedders enforce the documented faulted-instance quarantine whenever that integration exists.

## 5. First-line fixtures

Use the established `tests/cases/` input/config/expected integration layout, internal compiler unit tests and Node observer tests. Start by extending the closest existing fixture families, including `uint_checked_arithmetic_release`, `uint_checked_arithmetic_int64_release` and `numeric_semantic_physical_variants_debug` plus its release counterpart. Avoid creating one fixture per equivalent width if a table-driven unit test or shared fixture is clearer. Test failures in the correct lane: compiler diagnostic, Moth `Error` recovery, semantic trap or unsupported-target diagnostic.

Minimum contract examples:

```moth
$safe_math $infallible
sum_small |left I8, right U8| -> I16:
    return left + right
;

$fast_math $infallible
wrap_small |left U8, right U8| -> U8:
    return left + right
;

$fast_math $safe_math
dec_only |left Dec4, right Dec4| -> Dec4:
    return left + right
;
```

The first is valid because all I8 + U8 sums fit I16. Equal narrow types do **not** widen: `I8 + I8` and `U8 + U8` may overflow in ordinary mode. The second is infallible because wrapping addition cannot trap. The third is safe because Dec uses ordinary semantics. Reject a safe function that calls `wrap_small`, a fast infallible function with unproved `a // b`, and an infallible function whose caught failure can still escape through its fallback.

## Future work outside this plan

Plan 2 optimises ordinary numeric success/error lowering and optional numeric proofs. The third plan will evaluate a small benchmark-justified set of `fast_*` Core Math F64 functions and their explicit trusted metadata. It must preserve ordinary helpers and cannot claim speed purely from a `fast_` name. Possible `$pure`, `$no_io` and `$no_wrap` remain separate, unapproved future design work.
