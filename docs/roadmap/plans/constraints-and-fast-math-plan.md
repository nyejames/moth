# Constraints foundations and fast math

## Status

- Status: queued, with the design discussion accepted.
- Current slice: not started.
- Blockers: compact typed expressions, dense HIR/native result channels, general directive attachment and HTML page directives must be delivered.
- Next action: activate after HTML page directives and runtime title, refresh the owner inventory and start Phase 0.

## Purpose and authority

Add compiler-owned `$fast_math`, `$safe_math` and `$infallible` on functions and statements. Call this feature **constraints**, not an effects system. Types describe values. Constraints restrict the behaviour of a computation, including calls made by that computation. The implementation may remain this small permanently.

This plan records the user's accepted replacement for blanket checked arithmetic. Ordinary arithmetic keeps its current rules. `$fast_math` selects a second **Moth-defined** numeric mode with specified wrapping and trapping, not whatever a backend happens to do. Both modes preserve valid typed values, backend-independent language behaviour and mandatory memory safety. Wasm supplies the starting point, not permission to import all of its semantics.

The final decision supersedes the earlier discussion of backend-defined results, unbounded JS values in fixed integer types and escaping NaN/infinity. It also supersedes blanket exclusions of any numeric escape hatch. It does not supersede the prohibition on backend-conditioned source, undefined behaviour, weakened memory checks or arbitrary semantic extensions by builders.

Publish the accepted contracts in the permanent owners in Phase 1 before implementation. Until that transfer, this document records the explicit user decision, not evidence that the compiler supports it. V1 is a complete initial contract within the scope below. Later changes require an explicit design revision, compatibility review and parity tests. An implementer cannot choose different outcomes because the contract is labelled V1.

## Prerequisites and reading

Follow `AGENTS.md` and read the style, testing and validation guides. Read both compiler/build design authorities and the routed canonical numeric, cast, error, directive, function, package and range-loop references. Use `docs/src/developer-docs/language/overview.mtf` for their current paths. Read both progress matrices, the Core Math reference and the living Core Math package plan. Follow the memory overview for failure-edge and scalar-storage handoffs.

Consume these merged capabilities, without rebuilding them:

- Compact typed semantic expressions, dense HIR, native success/error result channels and the bounded Wiring foundation.
- Exact numeric identities and I32/F64 defaults, D15's equal-type preservation and smallest mixed domains, per-operation F16 completion and bounded body-local numeric-origin inference.
- Shared directive recognition, retained attachment syntax and argument validation.
- Explicit HTML root purposes, `$page` and runtime title capability.
- Existing implicit-failure inference, expression catch, public summaries, generic convergence and trusted Core constant evaluation where advertised.

The roadmap owns sequencing. Name prerequisite capabilities rather than linking temporary prerequisite plans. Establish a fresh baseline and isolated branch/worktree at activation. Keep that revision and raw measurements in working notes, not this queued status block.

## 1. Scope and principles

Implement the three directives, a bounded compiler-owned constraint analysis, canonical public guarantees, scalar fast arithmetic on supported targets and selected `fast_*` Core Math counterparts. Include the documentation migration, numeric optimisation work and contract tests below.

Defer `$pure`, `$no_io`, `$no_wrap` and other restrictions. Add no user-defined constraints, effect rows, permission sets, constraint expressions, runtime constraint objects, dynamic handlers or constraint-polymorphic type parameters. Add no speculative fields or registry hooks for those future features. The existing `$checked:` proof-budget direction retains its separate meaning.

Prefer the smallest complete solution. Reuse current operator classification, direct-call summaries, worklists, diagnostics and numeric helpers. Keep mode selection, failure delivery and proof evidence distinct. A fast operation is not a proven-safe checked operation. A constraint never inserts recovery, weakens another rule or rewrites a callee to make its assertion pass.

No claim of maximal speed, constant-time execution or improved total compile time follows from a directive. Measure runtime and compiler cost separately. Finite-result checks and some division guards remain required in fast mode.

## 2. Source contract

### Attachment and lexical extent

Each directive takes no arguments. Use `$fast_math`, not `$fast_math()`. Reuse the general modifier rules for trivia, same-line attachment, duplicate detection and dangling modifiers. Ordinary identifiers with these names stay legal outside directive syntax.

Eligible targets are function declarations, receiver methods, generic function declarations and executable statements, including declarations of runtime values, assignments, calls, returns, branches and loops. A compound statement includes its condition/header, arms and body. A function modifier covers its body. Nested callable declarations retain their own contracts and do not inherit the containing body's mode.

Imports, type/field/parameter declarations, standalone compile-time declarations, project configuration, template heads and export groups are not modifier targets. Defaults retain their declaration-owned evaluation rules. This feature adds no file-wide mode, new block syntax or implicit annotation of every export. A root runtime statement may be annotated without making compiler-owned `start` a user-declared function.

Numeric initialisers and other expressions inside a covered body keep that body's mode even when constant folding removes runtime work. Explicit `#` initialisers inside the body still obey required-constant rules. A called function's body is never retargeted by its caller's `$fast_math`. Caller-authored argument and receiver expressions use the caller's lexical mode. Ordinary calls to Math helpers do not automatically select `fast_*` names.

`$fast_math` covers eligible numeric operators and their implicit compound write-back. Explicit `cast`, comparison semantics, logical operators, collection/map APIs, assertions and foreign boundary validation retain their own contracts. No new operator/type pair becomes legal, including unsigned negation, Byte arithmetic or binary-float `//`. A cast requires an immediate concrete target and does not constrain its source type.

### Meaning of the three directives

| Directive | Meaning |
|---|---|
| `$fast_math` | Select the fast numeric mode for the covered operations. Their numeric invalid cases wrap or trap under section 3 rather than producing implicit numeric Error. Other calls and failures remain unchanged. |
| `$safe_math` | Prove that the covered computation uses ordinary Moth numeric semantics throughout its transitive executed calls. Reject fast arithmetic and fast Math contracts. It does not prove arithmetic succeeds. |
| `$infallible` | Prove that no recoverable failure escapes the selected boundary and no Moth-semantic panic/trap path can execute within it or its calls. Local recovery is permitted only when the handler also satisfies the constraint. |

Constraints apply to the entire selected computation, not just tokens directly in its body. They never change a callee's mode. An outer `$safe_math` is not overridden by an inner `$fast_math`. Applying both to one target is a direct conflict. `$fast_math` and `$infallible` are compatible when the resulting operations have no possible semantic trap.

Assertions with a possible false result contribute a panic path. So do fast numeric traps, language-defined range guards and documented fatal value-integrity guards. A function without an error slot is not automatically `$infallible`. A function may retain a declared error slot while proving that none of its paths returns an error. This does not remove ordinary call-site handling syntax or change the declared result ABI.

Environmental resource exhaustion, stack exhaustion, external termination and compiler/runtime faults are outside `$infallible`. A known input-dependent numeric trap cannot be relabelled as an environmental fault. The constraint proves neither termination nor bounded execution time. Borrowing, lifetime topology, bounds checks and valid storage remain mandatory independently of all three directives.

For a statement constraint, its completion/escape is the boundary. For a function constraint, its callable return is the boundary. Recoverable failure discharged inside that boundary is allowed. Catch cannot discharge a semantic trap. Effects of code executed before failure are not rolled back.

Authored safety-catch eligibility remains separate from actual recoverable
failure or trap summaries. Valid arithmetic, casts or more than one call at
the selected expression's level can permit catch even when currently
infallible. Receiver-chain and separate-operand calls count, argument-nested
calls do not inflate the enclosing count. Actual nested failures remain
protected. Validate handlers and fallbacks normally before erasing unreachable
work. Eligibility creates no Error producer or edge and cannot catch a fatal
integrity guard or fast trap. `$infallible` checks actual failure/trap paths,
not the eligibility flag.

### Guarantees versus value history

`$safe_math` constrains computations, not the history of argument bits. Calling a safe function with an already wrapped but valid integer does not recover the original mathematical value. Keep no fast-origin taint on ordinary values. Width/range and finiteness invariants make such values safe to consume under their ordinary types.

Use active semantic reachability after the existing static selection rules. Invalid syntax/types in inactive ordinary branches remain errors. A selected fast operation retains its semantic mode for constraint checking even if later folded or optimised away. Optional optimisation cannot turn a `$safe_math` rejection into acceptance. Folded function calls likewise retain any constraint-relevant semantic provenance until verification completes.

## 3. Numeric backend contract V1

### Shared domain and execution rules

Both modes use the same literal materialisation, operand eligibility, D15 result domains, exact precision, exactly-once evaluation and observable source ordering. A receiving type does not retag a concrete result or infer named arithmetic operands backwards. Equal types retain their width: `U8 + U8 -> U8` and `F16 + F16 -> F16`. Mixed integers select the smallest supported common operand domain and mixed floats the wider precision. No I32 or F32 promotion floor applies.

For an N-bit integer domain define `wrap_N(x)` as the unique non-negative residue modulo `2^N`. An unsigned result is that residue. A signed result is its two's-complement interpretation, subtracting `2^N` when the high bit is set. Use the **semantic result** width, never incidental carrier width. Every published integer remains in its declared range. Integer zero has its ordinary canonical representation.

A trap ends the current Moth invocation without returning an Error or entering Moth catch. Preserve required preceding observable work and prevent subsequent work. Do not promise rollback, cleanup completion, a particular process exit mechanism, host exception class or identical trap text. A host may report the fault through existing tooling. Fast traps are not assertions and must not be misreported as failed assertions or compiler bugs.

Compiler-known invalid numeric work remains a source diagnostic, including under catch. Fast integer overflow is valid and folds to the specified wrapped value. A known fast zero division, invalid exponent or non-finite result is a diagnostic rather than a compiler panic or an excuse to defer known invalidity to runtime. Required and opportunistic folding use the selected mode through one numeric owner. A backend cannot refold using its host's accidental arithmetic.

### Integer operators

| Operator | Ordinary mode | Fast mode | Fast semantic trap condition |
|---|---|---|---|
| `+`, `-`, `*` | Check the promoted result domain | Apply `wrap_N` to the mathematical result | None |
| Supported unary `-` | Check the negation result domain | Apply `wrap_N` to the negated value | None |
| Unsigned `//` | Truncate integer quotient, failing on zero divisor | Same quotient | Divisor is zero |
| Signed `//` | Truncate toward zero, checking zero and overflow | Same quotient | Divisor is zero or result-domain `MIN // -1` |
| Unsigned `%` | Check zero divisor | Same remainder | Divisor is zero |
| Signed `%` | Dividend-signed remainder | Same remainder, including `MIN % -1 == 0` | Divisor is zero only |
| Bounded integer `^` | Existing checked exponentiation | Exact modular power in the result domain | A negative signed exponent |
| Integer operands with `/` | Existing conversion to the real-division result domain | Same conversion, then fast binary-float division below | As specified for binary-float division |

Unsigned exponents cannot be negative. Preserve the ordinary exponent-zero identity, including `0 ^ 0 == 1`. Use bounded modular exponentiation rather than constructing an unbounded host power and truncating afterwards. Modular intermediate multiplication is part of that algorithm, not another source failure boundary.

Eligible fixed integer `/` produces F64, including narrow equal operands. The signed-division overflow pair belongs to `//`, not real division. Fast mode adds no bitwise surface operators.

### Binary-float operators

This covers exact F16/F32/F64 through D15's result-domain rules. Every language-level operation rounds and checks finiteness at its selected semantic precision. F16 completes at each operation, not only at write-back into storage. A wider carrier is not a wider result type. Preserve direct rounding, signed zero and subnormal behaviour. Finite underflow to subnormal or zero is valid.

| Operator | Fast result | Fast semantic trap condition |
|---|---|---|
| `+`, `-`, `*` | Ordinary rounded result | Rounded result is non-finite |
| Unary `-` | Ordinary negation of finite input | None |
| `/` | Ordinary rounded quotient | Zero divisor, including either signed zero, or non-finite result |
| `%` | Existing dividend-sign remainder, not a new IEEE remainder operation | Zero divisor or non-finite result |
| `^` | Existing scalar power contract and result rounding | Invalid/non-finite result under that contract |
| Comparisons | Existing numeric comparison semantics | No newly introduced failure |

Ordinary mode retains its current recoverable numeric failure conditions. Fast mode changes delivery to a trap, not value validity. Fractional and negative exponents follow the existing binary-float power rules, not the bounded-integer exponent restriction.

NaN and infinity never become Moth operands, stored values, arguments, returns or constants in either mode. A raw computation carrier may hold an invalid intermediate only to validate the current operation before publication. Check each language operation result unless a proof preserves exactly the same result and first-failure behaviour. Checking only the final expression or function result is invalid.

For example, if `a * b` overflows, `1.0 / (a * b)` traps at multiplication even if raw host arithmetic could subsequently produce finite zero. Helper-internal steps are not additional language-level rounding/failure boundaries unless the helper contract explicitly defines that composition.

### Dec operators

Preserve arbitrary-precision coefficients, scales, accepted integer mixing and all normal Dec result types. Fast mode does not convert Dec to a bounded integer or binary float.

| Operator | Fast result | Fast semantic trap condition |
|---|---|---|
| Same-scale `+`, `-` and unary `-` | Exact ordinary result | None, excluding resource exhaustion |
| `*` | Exact product rounded half-even to the result scale | None, excluding resource exhaustion |
| Positive-scale `/` | Ordinary half-even division at the result scale | Zero divisor |
| Scale-zero `//` | Ordinary truncating integer quotient | Zero divisor |
| `%` | Ordinary coefficient/decimal remainder at the result scale | Zero divisor |
| `DecN ^ I32` | Existing exact-power and half-even result contract | Negative exponent |

Scale-zero `/`, positive-scale `//`, unsupported scale mixing and other invalid pairs remain source errors. Preserve the exponent-zero identity. Round at each language operation result, not at arbitrary implementation multiplications. Resource limits remain fatal environmental limitations rather than new recoverable Dec errors or invented semantic arithmetic traps.

### Compound operations and conversions

Evaluate the place and RHS once in their existing order. Perform promoted arithmetic, then implicit write-back, then commit the store. A trap before commit leaves that target unchanged, without undoing earlier effects.

| Implicit write-back | Fast rule |
|---|---|
| Integer to integer | Reduce to destination-width low bits and interpret destination signedness, including negative-to-unsigned conversions where the ordinary compound conversion is supported |
| Binary float to integer | Truncate toward zero, then trap unless the truncated value fits the target range. Do not saturate or reduce an out-of-range float modulo the width |
| Integer to binary float | Ordinary target-precision rounding, trapping if the rounded result is non-finite |
| Binary float to binary float | Ordinary target-precision rounding, trapping if non-finite |
| Supported conversion involving Dec | Preserve ordinary exactness/scale rules. A conversion failure traps rather than becoming recoverable implicit write-back failure |
| Identity or other total supported conversion | Preserve the ordinary result |

Only already-supported compound conversions participate. Byte remains outside arithmetic. Explicit casts keep their current evidence, handling and error contracts, even inside a fast scope. `/=` on an integer follows real division and float-to-integer write-back. `//=` follows integer division.

### Range loops and compiler-generated arithmetic

Preserve the canonical range sequence, one-time header evaluation, step magnitude/direction, last-valid-endpoint termination and optional index meaning. A fast scope never permits counter wraparound to restart a range or an overflowing sentinel to terminate an otherwise valid range.

Authored arithmetic in headers/bodies uses the lexical mode. Generated counter work remains subject to the range algorithm's stronger invariants. Perform endpoint checks before any update that could leave the range. Zero-step and no-progress guards stay required. Fast mode delivers the range's remaining semantic guard/counter failures as traps rather than inferred Error. Checked mode keeps its current delivery. A generated range/index update that cannot preserve its required representable sequence traps rather than silently wrapping bookkeeping.

Preserve the existing floating range exception: a raw candidate outside the range or non-finite may end iteration without becoming a Moth value. An in-range candidate that stops progressing traps in fast mode. Do not apply ordinary expression publication rules to an uncommitted loop candidate and thereby change valid endpoint behaviour.

Allocation sizes, addresses, capacity arithmetic, collection bounds and other memory-safety checks never inherit wrapping permission. They are not authored numeric operators.

### Optimisation and backend obligations

An optimisation preserves defined values, required failure/trap behaviour, first observable failure, source-visible effects and valid types for all permitted inputs. It may omit a predicate only with sufficient proof. It may not speculate a trapping operation onto a path that does not evaluate it or move later observable work ahead of a possible trap. Inlining preserves the callee's mode.

Fast mode does not enable global reassociation, reciprocal approximation, flush-to-zero, discarded signed zero, fused rounding or reordered effects. An equivalent transformation can still be proved locally. In a common modular integer domain, valid algebraic substitutions may be used without pretending overflow is impossible.

Required counterexamples for backend review:

- Wrapping multiplication by eight can become a same-width shift with equivalent bits.
- Signed division by eight cannot blindly become arithmetic shift for negative values.
- `x // -1` cannot become wrapping negation without retaining/excluding the MIN trap.
- `x // x` cannot become one without retaining/excluding zero division.
- `MIN % -1` returns zero even on an intermediate representation whose remainder instruction is undefined there.
- Removing a dead result must not remove its possible trap or operand side effects.

Use native Wasm modular integer operations and integer division/remainder where they match. Add finite-result validation for floating arithmetic. JS must normalise integer widths and avoid precision loss before normalisation: use `Math.imul` or an equivalent exact low-word operation for 32-bit products and width-normalised BigInt arithmetic for 64-bit domains. Number multiplication followed by truncation is insufficient for arbitrary 32-bit operands.

A future RISC-V backend must add the universal division traps even though its division instructions return specified values for those inputs. A future C/LLVM backend must implement modular addition without signed-overflow undefined behaviour and guard invalid division/remainder before executing undefined instructions. Source fast mode does not permit LLVM poison, `undef`, unjustified `nsw`/`nuw` or blanket fast-math flags. LLVM `unreachable` alone is not a runtime trap. Emit actual termination where required.

These are backend conformance obligations, not work to create C, LLVM or RISC-V backends now. If a target cannot implement an accepted operation, use an explicit capability rejection rather than different semantics. Source constraint validity remains independent of target selection and debug/release. Exact numeric identities are semantic facts, never per-backend switches. This coordination introduces no additional Wasm capability.

## 4. Constraint analysis and public guarantees

### Ownership and representation

Extend the existing compiler-owned call-summary boundary. Retain only facts needed now: escaping recoverable failure, possible semantic panic/trap and use of fast numeric semantics. Reuse existing typed error contracts and implicit-failure facts rather than collapsing all handling into one new boolean. Distinguish a completed negative fact from a provisional or unknown foreign fact.

Keep declared constraints separate from inferred summaries. Use compact owner-local statement/function attachments and sparse witnesses, not a bulky payload added to every expression. No second call graph, global registry, source rescan or general constraint engine is needed. Builders provide trusted package contracts and consume validated outputs. They do not run or override constraint inference.

Inference composes calls through the existing finite monotone worklist, including recursion and generated functions. Recoverable failure is removed only by actual compatible local handling. Panic/trap and fast-mode use pass through catch unchanged. Handler behaviour contributes to the enclosing summary. Ordinary unconstrained functions still need summaries when a constrained caller depends on them.

Verify annotated private and exported declarations even if no selected entry calls them. Public methods, facade exports and re-exports preserve guarantees. A generic guarantee must follow conservatively from its template and available type/call contracts, with concrete materialisations revalidated through the same owners. Unknown generic behaviour is not permission to publish an unproved guarantee or defer a known violation until a consumer instantiates it. Add no constraint-polymorphic bounds in V1.

### Mandatory proofs versus optional optimisation

A bounded semantic proof policy decides acceptance. Initially reuse full numeric type bounds, exact constants, existing semantic discharge, local facts supported by the canonical analyser and completed trusted call contracts. Add only narrowly justified branch refinement with explicit invalidation tests. No SMT dependency, whole-program path solver or termination proof is required.

Use the same policy in debug/release and every target for fixed exact semantic types. A possible case that cannot be excluded is a constraint diagnostic. Optional `NumericProofs` must not become an accidental source-acceptance authority. Shared interval arithmetic may have one owner while mandatory verification and optional predicate elision remain separate consumers.

Wrapping integer add/subtract/multiply/negate have no numeric panic obligation. Fast integer division and finite-float arithmetic still do. `$fast_math` skips work needed only to recover/check integer overflow, not all analysis of the containing function. It never hides fallible API calls, explicit casts or assertions.

### Package trust and isolation

Analyse Moth-source dependencies normally, regardless of first- or third-party origin. Publish canonical guarantees for consumers without reopening provider bodies. Opaque arbitrary JS/foreign bindings do not prove `$safe_math` or `$infallible` merely because their signature has no error slot. A Moth wrapper cannot launder unknown behaviour.

Compiler- and builder-provided Core/Builder bindings may supply trusted, closed behavioural contracts. Distinguish trusted origin from ExternalBinding backing: Core Math itself is binding-backed. Validate metadata completeness and compatibility. Preserve it across aliases, namespace selection, generated calls, re-exports and package assembly. Trust resides at provider registration, not in arbitrary source annotations or imported package spelling.

Future side-effect restrictions should isolate capability origins to designated Core/Builder packages and use explicit provider metadata. They will restrict behaviour, such as `$no_io`, rather than grant permissions. Record that organising principle now without implementing IO/purity analysis. Per-callable metadata is already necessary because regular and fast helpers coexist in Core Math.

### Interfaces and fingerprints

Declared guarantees and all inferred facts visible to constrained consumers belong in PublicSemanticInterface and its existing fingerprint. Keep implementation/runtime/physical fingerprints responsible for their own affected facts. Changing a private helper from checked to fast or introducing a trap must invalidate every dependent public guarantee even when exported source text is unchanged.

Include numeric semantics and contract compatibility in existing retained/cached artefact domains. Reject or rebuild incompatible precompiled contracts. No parallel effects cache or new public effect fingerprint family. Generic sidecars and base modules retain facts paired to their exact final executable and exact numeric identities.

### Diagnostics

Identify the violated directive and selected boundary, then the offending operation or bounded call witness. Reuse implicit-failure witness ordering, cycle handling, source ownership and truncation. Distinguish a proved violation from unavailable evidence. A foreign leaf is an unknown guarantee, not an invented panic origin. Show result-domain overflow separately from compound write-back and fast use separately from a trap.

Changing to `$fast_math` is an explicit semantic tradeoff, never an automatic fix for `$infallible` or `$safe_math`. Compiler input errors still use the normal diagnostic lane and must not panic the compiler. Runtime numeric traps are a different subject from that rule.

## 5. Core Math counterparts

Keep ordinary helpers and their published numerical contracts. Their existing lack of a declared Error slot does not prove the stronger `$infallible` constraint. The shared finite-value boundary must be classified honestly. Do not silently convert all regular Math APIs to Error-returning functions.

Investigate and deliver a small useful `fast_*` set, beginning with F64-only counterparts of `sqrt`, `log`, `exp`, `pow` and `hypot`. Preserve F64 parameters/results and ordinary arities. All fast counterparts carry fast-math semantic metadata, retain finite result invariants and expose no typed numeric error slot. They remain ordinary calls usable outside an annotated function. `$safe_math` rejects their transitive use. F16/F32 overloads, generic Math and new Wasm Math capabilities remain deferred.

| Counterpart | Initial V1 contract |
|---|---|
| `fast_sqrt(x)` | Same finite square-root result and domain as regular sqrt. Trap for a negative operand or non-finite result |
| `fast_log(x)` | Same approximation/accuracy contract as regular log. Trap unless x is positive or if the result is non-finite |
| `fast_exp(x)` | Same approximation/accuracy contract as regular exp. Trap on non-finite result, permitting finite underflow |
| `fast_pow(base, exponent)` | Regular Math pow domain, identities and accuracy. Trap for invalid domain or non-finite result |
| `fast_hypot(x, y)` | Explicit fast composition: rounded x*x, rounded y*y, rounded sum, then square root. Validate every specified intermediate. Unlike regular hypot, intermediate overflow traps |

Package approximations retain their documented tolerance instead of promising bit-identical transcendentals. Operator `^` retains its own scalar contract and is not silently replaced by a differently specified Math helper. For fast_hypot, retain its explicit composition rather than later replacing it with the regular overflow-avoiding algorithm, which would change required trapping.

At activation, measure the proposed counterparts against the then-current regular lowering. Seal the useful exported subset before implementation. Retain fast_hypot as the concrete algorithmic tradeoff. Omit a redundant counterpart when it provides neither a cheaper implementation nor a distinct documented contract, and record why. Do not mechanically double the package, invent lower-accuracy approximations or claim speed from a name. New approximation algorithms require explicit error bounds and separate approval.

Reuse one package-local registration table, demand-driven helpers and trusted binding metadata. Never dispatch semantics by `fast_` prefix or package alias. Support only targets with the required accepted helper contract and lowering. Existing scalar Wasm primitives do not deliver Core Math's F64 functions. Additional Wasm Math support remains later target work, not an implicit prerequisite or promised side project. Unsupported reachable helpers retain honest target diagnostics.

## 6. Existing numeric optimisation work

Carry forward the optimisation direction without expanding it into a general optimiser:

- Collapse proven-safe checked-operation success carriers, dead error branches and unwraps in backend-owned lowering when exact proof allows it. Keep source constraint acceptance and source-level handling unchanged.
- Prefer scalar success paths and cold failure materialisation over creating a JS `{tag, value}` object for every primitive numeric step. Retain ordinary ABI carriers where an actual call boundary needs them.
- Give intrinsically total operations, including finite float negation and non-dividing Dec operations, a scalar path without fabricated recoverable failure.
- Extend conservative numeric facts only where a measured case justifies it. Broad loop induction, path solving and cross-function range specialisation are deferred unless separately approved.

Any backend CFG simplification must preserve live handler effects, result-slot dominance, exactly-once evaluation and paired proof identity. Do not mutate published frontend HIR to remove a backend predicate. Measure fast integer kernels, finite-float kernels and recovery-heavy checked code separately.

## 7. Owner and documentation map

Reverify locators at activation. The preceding compiler foundation deliberately changes representation. Do not revive recursive expressions, RPN ownership, result tuples, reactive metadata or pre-foundation failure-lane scaffolding to match an old filename.

| Area | Existing owners or required destination |
|---|---|
| Directive attachment and recognition | Compiler directive registry and shared declaration/statement preparation discovered from `index.md` |
| Numeric semantics and folding | `src/compiler_frontend/datatypes/numeric_operators.rs`, numeric scalar/Dec owners and their post-foundation equivalents |
| Summaries and convergence | `src/compiler_frontend/public_call_summary.rs`, `public_interface/` and `module_compilation/generated/` |
| HIR and proof facts | `src/compiler_frontend/hir/numeric.rs`, numeric emission/validation and `analysis/numeric_proofs/` |
| Trusted binding contracts | `src/compiler_frontend/external_packages/` and `src/builder_surface/` |
| Runtime lowering | `src/backends/js/`, `src/backends/wasm/` and backend feature validation |
| Core Math | `src/builder_surface/core_packages/math.rs`, package reference and living package plan |
| Source contract | Create `docs/src/docs/directives/constraints.mtf` with a compact Basic companion, integrated into the directive page |
| Developer backend contract | Create `docs/src/developer-docs/numeric-backend-contract/numeric-backend-contract.mtf` and its `@page.moth`, with a compact routing `overview.mtf` only if needed |
| Architecture | `docs/compiler-design-overview.md` and relevant capability/interface sections of `docs/build-system-design.md` |
| Navigation | Developer/language overview, developer page navigation and `index.md` |

Publish section 3's complete operation matrix, invariant rules, optimiser obligations and backend exceptions in the developer contract. The page must be reachable from developer navigation and numeric/constraint references, not merely exist as an orphan file. Topic content owns the contract, `@page.moth` owns presentation and overview files remain routing rather than page imports.

Update the canonical unsuffixed numeric operators/checked-arithmetic, casts, ranges, errors/runtime termination, assertions, functions/returns, public API, receiver/generic, module/package visibility and re-export references where their rules change. Update the directive reference, cheatsheet and relevant Basic teaching. Explain with compact valid examples, including wrapping integer infallibility and finite-float rejection.

Amend Design Scope and design principles precisely. Ordinary arithmetic remains checked. Fast arithmetic has specified modular overflow or traps. Backend-independent semantics, finite values, valid integer widths, memory safety and the compiler's no-input-panic policy remain intact. Remove the blanket exclusion of any numeric mode, not the exclusions of UB, unchecked memory or backend-dependent language meaning.

Correct diagnostic guidance that categorically forbids mentioning wrapping. Describe `$fast_math` only as an explicit choice with its costs and guarantees, not the default repair. Reconcile Core Math's no-error-slot wording with possible integrity traps and the stronger constraint.

Progress records accepted queued support first, then actual frontend, JS, Wasm, package and test coverage. It must not claim that every fast operator is check-free or that unsupported Math/Dec targets work. Add a clearly labelled future-design reminder for `$pure`, `$no_io` and possible `$no_wrap`, pointing to the final roadmap note. Keep these out of accepted implementation rows and registered syntax. At completion, the roadmap future note remains while this plan and its sequenced bullet are deleted.

## 8. Implementation phases and gates

For each behaviour change, add the failing test first, run it to establish the failure, implement the smallest owner change, rerun the focused tests and perform the Slice review before committing. Use small coherent commits. Keep all raw benchmark output local.

### Phase 0: activation and contract inventory

- [ ] Rebase onto the named prerequisites and record the baseline and exact toolchain in working notes.
- [ ] Read the authorities and map the current directive, numeric, summary, binding, proof, runtime and test owners. Classify each as reuse, extend, replace or leave unchanged.
- [ ] Inventory every legal exact numeric operator/domain and implicit compound conversion against section 3, including D15 narrow checks and operation-level F16 completion. Unsupported pairs stay unsupported.
- [ ] Identify all known semantic fatal paths and all trusted external operations that can reach them. Separate resource limits and unknown foreign evidence.
- [ ] Seal the Core Math subset with contracts and comparative generated-code/runtime evidence. Record omitted redundant counterparts rather than padding the API.
- [ ] Run the baseline checks required by the validation guide. Treat unrelated blockers separately and never weaken coverage to make the baseline green.

Gate: a complete owner/operation inventory, no undefined backend-choice cell and an agreed useful helper subset. A newly discovered semantic conflict pauses its affected slice for a documented decision rather than permitting an invented fallback.

### Phase 1: permanent contracts and status

- [ ] Publish the complete backend contract V1 and source constraint reference at the destinations above. Mark support in progress, not by diluting final semantics with migration prose.
- [ ] Make the scoped authority/design-scope amendments and add compiler/build interface ownership rules.
- [ ] Integrate website navigation, teaching routes and queued progress entries. Add the exploratory constraints reminder without marking those directives accepted.
- [ ] Check docs and build the release documentation through the compiler. Review generated links and examples. Commit no hand-edited release HTML.

Gate: the implementer and backend author can derive every V1 outcome from permanent docs without this chat. Later phases reference those owners instead of maintaining duplicate matrices.

### Phase 2: directive attachment and typed numeric mode

- [ ] Add source cases for statement/function/method attachment, nested arguments, compound statements, duplicates, empty parentheses, invalid targets and conflicting annotations.
- [ ] Add cases proving lexical mode does not propagate into a callee or disappear during inlining/folding. Caller arguments and explicit casts retain their separate rules.
- [ ] Extend shared attachment and typed-expression owners with compact semantic mode and constraint requirements. Thread these through concrete/generic headers, HIR and dumps without a new parser.
- [ ] Add HIR validation rejecting invalid mode/operator/result shapes and mode/failure-delivery contradictions.
- [ ] Implement mode-aware constant folding, including modular overflow success and known-trap diagnostics under catch.

Gate: one typed mode owner, equivalent folded/runtime contracts and no enabled runtime surface silently lowering through the old checked path.

### Phase 3: summaries, semantic traps and constraint verification

- [ ] Add direct and multi-hop tests for recovered failure, handler failure, assertions, fast overflow, fast division, finite-float traps and range guards.
- [ ] Add unknown-foreign, trusted-Core, recursive, generic, public method, re-export and facade tests. Constraint violations must be caught without a caller executing the export.
- [ ] Extend canonical summaries and existing convergence with the minimal facts and sparse witnesses. Keep missing/provisional summaries distinct from proven absence.
- [ ] Implement mandatory bounded proof and constraint checks. `$infallible` accepts fast wrapping add but rejects unproved division or float overflow. `$safe_math` rejects transitive fast use even if a caller catches errors.
- [ ] Carry guarantees through public interfaces, fingerprints and generated sidecars. Test same-text exports whose private implementation changes and stale/unknown imported summaries.
- [ ] Reuse bounded diagnostic witnesses. Add deterministic rendering and recursive back-edge tests, including a terminal unknown foreign leaf.

Gate: same source verdict on JS/Wasm and debug/release for the same exact types. Optional proof tables cannot alter acceptance. No duplicate global effects engine or new IO/purity machinery.

### Phase 4: JS fast lowering and scalar success paths

- [ ] Add runtime cases for signed/unsigned 32/64-bit wrapping, negative-to-unsigned write-back, exact low-word products, modular power and every numeric trap row.
- [ ] Implement primitive JS fast lowering with required width/precision normalisation and actual fatal delivery. Add no recoverable carrier to a fast numeric operation.
- [ ] Test finite results at every semantic boundary, preserved subnormals/signed zero, float-to-integer truncation boundaries and non-committed compound stores on trap.
- [ ] Simplify proven checked numeric carriers in the backend and total numeric helper paths where supported. Retain live ABI error channels and ordinary explicit cast behaviour.
- [ ] Verify helper demand selection and no fast runtime dependency in ordinary modules. Measure checked, fast and proven-safe success paths separately.

Gate: exact integer parity and faithful floating/trap outcomes. Generated text checks supplement executable tests rather than replacing them.

### Phase 5: Wasm lowering and backend conformance

- [ ] Add paired JS/Wasm cases using one source with shared semantic expectations for the supported scalar domains. Keep genuinely unsupported domains separately target-rejected.
- [ ] Lower modular arithmetic and required division/remainder traps with native instructions where correct. Add the required float validation and conversion guards.
- [ ] Keep mandatory range endpoint behaviour and memory/address checks outside modular permission. Test inclusive maximum, descending unsigned zero and float no-progress cases.
- [ ] Add optimisation regressions for signed division/shift, MIN/-1 negation, MIN remainder, x/x, dead trapped results and untaken branches with invalid operands.
- [ ] Ensure failure carriers are not required for fast numeric traps and the recoverable-numeric capability gate is not requested for them. Preserve unrelated target gates.
- [ ] Document C/LLVM and RISC-V obligations without implementing those backends or introducing an LLVM dependency.

Gate: no raw backend operation chosen solely because its success values match. Result, trap, order and side-effect equivalence are all covered.

### Phase 6: trusted packages and Core Math

- [ ] Extend trusted provider metadata with the minimal numeric/constraint facts. Reject incomplete or inconsistent trusted registrations. Opaque arbitrary foreign bindings remain conservative.
- [ ] Implement the sealed fast Math subset through existing package-local registration and lowering owners. Keep regular contracts unchanged.
- [ ] Add one distinguishing numerical test per new helper, invalid-input/trap cases, transitive safe/infallible cases and direct-selection/alias/re-export tests.
- [ ] Test regular hypot with large finite inputs against fast_hypot's required intermediate trap. Preserve the helper's explicit rounding composition.
- [ ] Verify every advertised helper target and report unsupported ones precisely. Share const evaluation only when its existing eligibility and bounded work rules are satisfied.

Gate: no name-based semantics, no unverified no-panic claims and no promised speed without measurements. Ordinary and fast helper contracts are documented separately.

### Phase 7: integration, performance and final documentation

- [ ] Run all primary contract cases and boundary cases across applicable exact widths, debug/release and supported JS/Wasm paths. Use runtime parameters to exercise runtime traps rather than folding all inputs.
- [ ] Measure runtime cost and frontend time on integer kernels, finite-float kernels, deep private call chains and constraint-heavy recursive/generic cases. Include ordinary no-directive baselines and cold failure cases.
- [ ] Review summary storage, expression sizes, convergence counts and generated helper/carrier counts against baseline. Remove redundant walks and temporary compatibility structures instead of normalising regressions.
- [ ] Finish every documentation owner in section 7 and rebuild generated docs. Check links, all examples and the exact operation/support matrix.
- [ ] Run targeted tests during iteration, `just validate` at integration boundaries and `just validate-full` on the final integrated tree before merge. Run the documentation gate as well. Report exact commands, revisions and any unrun/blocked checks.
- [ ] Perform the final Slice review: correctness, duplication, obsolete code, ownership, diagnostics, test honesty and absence of speculative constraints scaffolding.
- [ ] Transfer any remaining durable rules, remove this plan and its sequenced roadmap bullet in the completing commit, and retain the bottom-of-roadmap future-design note.

Gate: the final tree implements the three constraints and advertised backend/helper subset with honest unsupported-target reporting, updated authorities and passing required merge evidence. No implementation phase is complete merely because its source compiles.

## 9. Acceptance examples and boundary inventory

The following new syntax is an implementation target, not currently executable documentation:

```moth
$fast_math
$infallible
increment |value I32| -> I32:
    return value + 1
;

$safe_math
$infallible
sum_small |left I8, right U8| -> I16:
    return left + right
;

$safe_math
recover_sum |left I32, right I32| -> I32:
    value = left + right catch then 0
    return value
;
```

Accept those declarations. Reject an otherwise identical `$infallible` function performing unproved `a // b`, even with `$fast_math`. Reject a `$safe_math` wrapper calling increment because its guarantee covers mode use, not whether one observed input wraps. Accept a recovered checked operation only when the selected recovery path has no remaining panic/failure obligation.

Primary numeric boundaries include signed/unsigned narrow MIN/MAX, I32/I64 MIN/MAX, U32/U64 maxima, mixed operand-domain selection, a U32 product whose Number intermediate loses low bits, negative-to-unsigned compound stores, MIN/-1 division and remainder, both signed float zeros, largest finite results, subnormals, operation-level F16 overflow/rounding, modular powers with huge exponents, Dec zero divisors and exactness failures. Do not replicate every equivalent case across every type without a distinct boundary reason.

Primary control-flow boundaries include exactly-once receiver/RHS evaluation, earlier observable work surviving a trap, no store after failed write-back, no later call after failure, catch not handling fast traps, safety eligibility without a failure producer, same-level versus argument-nested call counts, inactive ordinary branches, generic materialisation, same-module recursion, opaque foreign leaves and stale published guarantees. Keep one primary test owner per behaviour and use boundary-role fixtures for additional target/width coverage.

## 10. External specification references

These primary references explain backend primitives. The Moth contract above is the authority for which result a backend must provide. Recheck relevant instruction details at activation.

- [Wasm numeric semantics](https://webassembly.github.io/spec/core/exec/numerics.html) and [instruction execution](https://webassembly.github.io/spec/core/exec/instructions.html).
- [ECMAScript Math](https://tc39.es/ecma262/multipage/numbers-and-dates.html) and [BigInt](https://tc39.es/ecma262/multipage/numbers-and-dates.html#sec-bigint-objects).
- [LLVM language reference](https://llvm.org/docs/LangRef.html) and [undefined behaviour manual](https://llvm.org/docs/UndefinedBehavior.html).
- [RISC-V integer multiplication/division extension](https://docs.riscv.org/reference/isa/unpriv/m-st-ext.html).

## Future constraints reminder

`$pure`, `$no_io` and possible `$no_wrap` remain design exploration. A possible no-wrap restriction could permit fast trapping arithmetic while excluding possible modular overflow. Decide its exact proof and naming rules later. Purity would require an explicit model of observable mutation, ambient reads, nondeterminism and trusted package capabilities. Neither is implied by a no-error signature.

Keep the user-facing model restriction-oriented and small. The compiler and project builders should isolate side-effect origins in designated provided packages and supply explicit contracts. Arbitrary opaque foreign code cannot establish absence guarantees. Future additions must justify their analysis and API costs independently. No full effects system is promised or required.
