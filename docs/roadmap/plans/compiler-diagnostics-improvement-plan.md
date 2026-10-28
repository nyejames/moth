# Moth Compiler Diagnostics Improvement Plan

## Purpose

Improve Moth diagnostics without teaching invalid syntax, legacy behaviour or patterns that fight the language design.

Every retained item in this plan has been checked against the current language and compiler contracts. Several findings from the original audit have been corrected, consolidated or removed because their proposed fixes were stale, ambiguous or semantically wrong.

This is an implementation plan, not a research backlog. Each phase should leave one current diagnostic path, remove superseded variants and add end-to-end coverage for the user-visible behaviour it changes.

## Required authority documents

- `docs/compiler-design-overview.md` for diagnostic lanes, type rendering and stage ownership
- `docs/compiler-data-layout-design.md` for source identities and spans, diagnostic schemas and storage, type-display capture, report rendering and failure lanes
- `docs/build-system-design.md` when config, graph or output diagnostics are in scope
- `docs/src/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`

## Architecture alignment

This plan owns source-facing diagnostic improvements, not config or graph architecture. It consumes those contracts from the build/compiler authorities and the current data-layout authority.

Diagnostic construction and storage:

- Build user-facing failures through owning family constructors as typed `DiagnosticDraft` values, accumulated in `DiagnosticBag`.
- Family schemas define stable codes and typed facts. Preserve factual names, source spans, operators, counts and reason enums rather than storing rendered prose.
- Boundary freezing compacts drafts into `DiagnosticStore`. Keep each common `DiagnosticRecord` within 32 bytes and its four fact words. Put variable labels, lists, foreign sites and additional facts only in existing schema-declared typed side stores when needed. Do not widen the common record or add unbounded diagnostic-specific storage.
- Capture display-relevant types into `TypeDisplaySnapshot` and `TypeDisplayId` at the owning boundary. Render frozen family data through `DiagnosticView`, not from raw `TypeId`s.
- Store related labels through schema-defined `LabelMessageCode` values and source spans. Put variable label data in typed side stores, not string-valued label fields.
- Preserve current stable codes, reason keys, severities and user-visible wording unless a named finding changes them.
- Keep operational failures in the `InfrastructureFailure` / `InfrastructureErrors` lane. Do not route source mistakes through infrastructure failures.

Build-system alignment:

- use graph outcome terminology for diagnosed and blocked modules
- preserve one diagnostic set per canonical module
- keep warnings only on successful command outcomes
- keep target-contract diagnostics shared by build and check

Future diagnostic coverage needed for:

- directive placement, signature, required-input and unknown/unavailable-name diagnostics
- root-purpose diagnostics for unpurposed authored runtime or direct output
- `@project` field collisions and provenance
- source input contract conflicts and restricted defaults
- package facade provenance rejection
- output manifest conflicts

Do not implement config-block syntax, inactive-section folding or legacy-name migration scanners as future work. Historical completed-phase evidence below remains evidence of past cleanup.

## Current state

```text
ACTIVE_PLAN: docs/roadmap/plans/compiler-diagnostics-improvement-plan.md
STATUS: ready to resume; the compact diagnostic layout is delivered
NEXT_SLICE: Phase 4.2, catch-recovery type context
BLOCKERS: none; bounded recovery is not a prerequisite
TRANSFERRED: Phase 4.1c propagation-context and Phase 4.4 result-arity work are delivered and must not be rescheduled
NUMERIC_CATCH_HANDOFF: exact-receiver conflict/default-range, narrow-operation and authored safety-catch diagnostics are delivered with primary tests; consume their schema-typed facts without duplicating coverage
RESUME_ACTION: start Phase 4.2 with `DiagnosticDraft`, `DiagnosticBag`, schema-typed facts, `DiagnosticStore` side stores and `DiagnosticView`; capture display types in `TypeDisplaySnapshot`/`TypeDisplayId` and keep `InfrastructureFailure`/`InfrastructureErrors` separate
```

The native-result transfers below remain delivered. Their coverage and source-visible messages belong to the compact typed-expression and native result-channel work.
This plan retains their design record and owns the remaining diagnostic improvements.

Delivered numeric semantics supply deterministic conflicting-receiver diagnostics with both source
locations, out-of-range default diagnostics, diagnostics for statically invalid selected-width
operations and normal handler/fallback type diagnostics for authored safety catches. Preserve their
classification, authored-order witnesses and primary source coverage. This plan consumes the
resolved diagnostic facts through the current family schema and typed report views, and owns only the
remaining presentation and recovery-context improvements. Catch eligibility remains separate from
actual failure effects, and an unreachable handler still receives ordinary type validation.

### Historical implementation and validation

Phase 4.1b+d completed, with the last accepted diagnostic slice recorded at `d7fb3654f`
(Phase 4.1d). The following records describe those earlier implementation checkpoints, not the
current workspace or validation of this documentation change.

Phase 3.2 renamed `NotMutablePlace` to `TemporaryNotAssignable` and `ImmutableVariable` to
`ImmutableBinding`. It added `ImmutableFieldRoot`, field/root names and the authored binding
location so immutable-binding failures carry a related declaration label. `ExpectedAssignmentOperator`
uses binding terminology. Those changes preserve `MOTH-RULE-0044`, distinguish temporary/direct/field
targets, name field roots when available and give ordinary `=` guidance without suggesting `~` on
assignment targets.

- Phases 1–2.6b and 3.1a–3.1b passed their focused checks and `just validate` at each recorded
  checkpoint. Completed phase notes retain the design and correction details.
- The Phase 3.2a implementation passed `cargo fmt`, 3,609 Rust tests, 1,777 integration cases and
  library Clippy.
- Phase 3.2a independent probes confirmed that immutable `p.x = 99` reports `ImmutableFieldRoot`
  naming `x` and `p`, immutable `value = 200` reports `ImmutableBinding` with ordinary assignment
  guidance, `Point(1,2).x = 5` reports `TemporaryNotAssignable` and a mutable `p` permits `p.x = 99`.
- Phase 3.2a `just validate` passed cross-target Clippy, 3,609 Rust tests, 1,777 integration cases,
  the docs check and 28 benchmark cases.

## Confirmed design decisions

- Quoted strings support exactly these escapes:
  - `\\`
  - `\"`
  - `\n`
  - `\r`
  - `\t`
- Every other quoted-string escape is rejected.
- Raw backtick strings do not process escapes. Backslashes and newlines remain literal.
- Discontinued syntax receives no migration-specific compatibility diagnostics.
- Incompatible template directives are rejected.
- A directive-conflict diagnostic must name both conflicting items and explain the semantic reason they cannot be combined.
- Existing mutable places require explicit `~place` only at mutable call and receiver-call boundaries.
- A binding must already be declared mutable before it can be reassigned or written through.
- Reassignment and field assignment use ordinary `=`. `~` is not written on assignment targets.
- Fresh values passed to mutable parameters remain valid without source `~`.
- External package namespace access and delimiter-free direct selections are both valid for direct package exports. Diagnostics must not claim otherwise.
- Error handling uses Moth's `Error!`, postfix `!` and `catch` terminology. User-facing diagnostics must not describe the source feature as a first-class `Result`.
- Multi-return calls cannot be received by a single declaration or assignment target. Moth has no implicit dropped return slot and no documented discard syntax.

## Required implementation rules

- Emit user mistakes through owning family constructors as typed `DiagnosticDraft` values and accumulate them in `DiagnosticBag`. Keep `InfrastructureFailure` / `InfrastructureErrors` for operational failures, not malformed source.
- Keep family-schema facts typed and factual. Preserve names, semantic `TypeId`s, operators, counts, `SourceSpan`s and reason enums rather than pre-rendered prose or unbounded diagnostic-specific fields.
- Capture display-relevant types in `TypeDisplaySnapshot` and use `TypeDisplayId` in frozen diagnostics. Render through `DiagnosticView`, never from raw `TypeId`s.
- Reuse an existing stable code when the diagnostic category and contract remain the same. Add a new code only for a genuinely new diagnostic family.
- Preserve the source span that identifies the actual mistake. Add related sites with schema-defined label message codes and spans where useful; use typed side-store data for variable label content.
- Do not generate exact replacement source unless the compiler has enough structured facts to make it valid. Prefer a generic correction over a fabricated snippet.
- Keep suggestions source-visible. Do not mention internal AST, HIR, parameter receiver placeholders, borrow lattice states or compiler-introduced locals.
- Suggestions must favour normal Moth patterns:
  - explicit mutable declarations followed by ordinary assignment
  - explicit `~place` at mutable call boundaries
  - templates for mixed textual interpolation
  - choices for runtime heterogeneity
  - concrete types or bound-provided receiver methods for generic behaviour
  - delimiter-free direct selections or namespace qualification according to the actual visible package surface
- Do not preserve obsolete diagnostics for removed syntax.
- Prefer one primary integration case per user-visible contract. Add unit tests only for renderer, tokenizer, scanner or compatibility invariants that integration output cannot isolate.

## Phase 1: Diagnostic render and taxonomy integrity

### 1.1 Guarantee non-empty terse messages

**Original findings:** DIAG-013, DIAG-027
**Status:** Complete in `df65c9ced`

The terse renderer uses the family descriptor title when the rendered message is empty. Focused and all-kind invariant tests protect non-empty messages without changing stable codes, locations or the terminal and dev-server formats.

### 1.2 Remove stale source terminology and misleading guidance

**Original findings consolidated:** DIAG-015, DIAG-025, DIAG-030, DIAG-043, DIAG-044
**Status:** Complete in `fb4d26a3b` and `99ec971b9`

Phase 1.2a replaced category-only operator facts with diagnostic-owned `DiagnosticOperator`
values and exact authored spellings. Range renders as `to`. Generic-parameter operator failures
now explain that operators are compiler-owned and aren't granted by trait bounds.

Phase 1.2b removed source-facing `Result` and `Option<Type>` terminology, stale
top-level-only template guidance, internal AST/finalization and const-model wording plus
implementation-stage language around deferred `async`. Standalone unreceived templates now use
the existing standalone-statement family. Superseded reason variants, payload paths and renderer
branches were removed rather than preserved as compatibility aliases.
### 1.3 Remove obsolete compatibility-only diagnostic paths

**Original finding:** DIAG-039
**Decision:** Strict removal
**Status:** Complete in `2189b2107`

Compatibility-only diagnostic kinds, payloads, constructors, recognisers, pre-scans and fixtures
were deleted. Current binding, constant, template and module-root syntax retained its existing
positive coverage.

The Phase 2.2 hard-removal audit also made const-record diagnostics use the source-visible
`const record Config` terminology.

## Phase 2: Tokenizer and structural syntax diagnostics

### 2.1 Implement the quoted-string escape contract

**Original finding:** DIAG-028
**Status:** Complete in `979703ea7`

Quoted strings now decode exactly `\\`, `\"`, `\n`, `\r` and `\t`. Typed
`MOTH-SYNTAX-0034` reasons distinguish unsupported escapes, physical LF/CRLF continuation and a
trailing backslash. Spans cover two authored characters for complete unsupported escapes and only
the backslash for continuation or trailing cases. Control characters render source-visibly.

Raw-token behavior is unchanged: backslashes and physical newlines stay literal, no escape
diagnostic is emitted and expression-position raw backticks remain rejected by the existing
ordinary syntax path. Canonical string docs, generated docs and the progress matrix record the
accepted contract. Tokenizer, renderer and integration coverage protect decoding, reasons, spans
and raw-token preservation.
### 2.2 Make symbolic spacing diagnostics exact

**Original findings:** DIAG-004, DIAG-005
**Status:** Complete in `4e2b3e31c`

`SymbolicSpacingConstruct` now distinguishes binary operators, ordinary assignment, compound
assignment and the adjacent `~=` mutable-declaration marker. It carries the exact operator plus
`MissingWhitespace::{Before, After, Both}`, including complete `//=` recognition. Outer spacing
uses `MOTH-SYNTAX-0031`, while the declaration parser still owns whitespace inside `~ =` through
`InvalidMutableBindingSpacing`. Valid `name ~= value` and ordinary operator tokenization are
unchanged. Table-driven tokenizer coverage protects every compound assignment and each missing-side
classification, with integration owners for source-visible assignment and mutable-marker wording.

### 2.3 Superseded missing-`@` import-prefix diagnostic

**Original finding:** DIAG-038
**Status:** Complete in `3b9fdd914`

This completed checkpoint described the former `import @...` grammar. The dependency-clause
cutover superseded that spelling and its exact replacement guidance. Current diagnostics must
target top-level `@provider` clauses, must not suggest `@./...`, and must not revive grouped
selection delimiters. General dependency parsing remains in headers.

### 2.4 Improve incomplete expression and declaration boundaries

**Original findings:** DIAG-012, DIAG-016, DIAG-018, DIAG-024, DIAG-053, DIAG-054

- `69cdd7831`: missing statement, value-producing and inferred multi-bind `if` conditions now use
  `ExpectedConditionAfterIf` at colon, `then`, `else`, newline, end and EOF boundaries.
- `04f9f4431`, `66e4d7bce` and `8fda8c728`: missing `then`/`else`/catch values and absent inline
  value-`if` branches now use exact structured reasons at the first missing-value boundary. Real
  multiline forms keep their distinct diagnostics and unrelated later `else` tokens cannot attach.
- `60e032672`: definite adjacent operands use `ExpectedOperatorBeforeExpression` at dispatch entry.
  Comment templates stay value-less, adjacent value templates share the same path and the defensive
  evaluator fallback no longer claims that no operands were authored.
- `d3a76a61e`: postfix field and namespace access ending after `.` uses `ExpectedNameAfterDot` at
  the authored dot while other invalid member boundaries retain their offending-token locations.
- `2485d961d`: function signatures and trait requirements share the missing-return boundary check
  but retain source-accurate function-body versus bodyless-requirement guidance.
- `bcbdd73aa`: authored `=` with no declaration value uses `MissingInitializerExpression`. The
  no-`=` family is `MissingDeclarationInitializer`, keeps `MOTH-RULE-0031` and has no compatibility
  aliases.
- `a21396762`: parameter and struct-field `=` boundaries use `MissingDefaultValue` and
  `MOTH-RULE-0028`. Reactive, trait and choice prohibitions still win, while complete and
  operator-continued multiline defaults remain valid.

Focused boundary tables and integration owners protect exact messages, stable codes and source
locations across EOF, newline, delimiter and block-end paths. DIAG-034 remains removed because the
existing `BinaryRight` reason already owns a missing right operand.

### 2.5 Convert user-input infrastructure failures

**Original findings:** DIAG-011, DIAG-012, DIAG-035
**Additional confirmed gap:** duplicate struct field names currently survive the shared record-body
parser and fail as a HIR invariant. Choice payload fields already run a later duplicate scan, so
adding another struct-only validator would preserve duplicate ownership.
**Additional confirmed gap:** duplicate function parameter names also reach `MOTH-INFRA-0001` from
function-scope local registration. Both failures must move to the shared signature-member parser.
**Additional confirmed gap:** duplicate trait-requirement parameter names currently compile
successfully. The same shared signature-member owner must reject them before trait registration.
**Additional confirmed gap:** the reusable `DuplicateDeclaration` renderer currently describes
every duplicate as a top-level function, struct or compile-time constant. That wording is already
wrong for choice payload fields and local bindings and would make an earlier member diagnostic
misleading.

#### Malformed `$children(...)`

**Phase 2.5a status:** Complete in `636ea3070`.

The shared directive-argument parser now classifies a closing parenthesis, leading-newline-only
argument and template boundary before an expression as `EmptyArguments`. The `$children` owner
reclassifies that fact to `InvalidChildrenArgument` with `MOTH-SYNTAX-0021`. True file EOF remains
the header balancer's unclosed-template diagnostic and begun argument expressions keep their own
delimiter failures. Focused parser coverage protects generic versus `$children` wording, with one
integration owner for the template-boundary case. Phase 6.2 owns adding opening context to the
general unclosed-template lane.

#### Duplicate function parameters

**Phase 2.5b status:** Complete in `b763b7069`.

Duplicate parameter and record-field names are source declarations, not HIR invariants.

- Detect duplicate member paths in the shared signature-member parser used by function parameters,
  struct fields, choice payload fields and trait requirements.
- Reuse `DuplicateDeclaration` and include:
  - duplicate name
  - current member location
  - previous member location
- Make the shared renderer scope-neutral and accurate for top-level declarations, visible local
  bindings, parameters, fields and variants. Prefer: `Cannot declare 'name' because that name is
  already visible in this scope. Moth does not allow duplicate names or shadowing.`
- Remove the later choice-payload-only duplicate scan once the shared owner covers it.
- Do not route a user mistake through the infrastructure-failure lane.
- Do not let duplicate struct fields reach the HIR duplicate-field invariant.

The shared signature-member parser now owns ordinary member-name uniqueness for function
parameters, struct fields, choice payload fields and trait-requirement parameters. It keeps the
current member primary and the first member secondary through `DuplicateDeclaration`. The later
choice-payload-only scan is removed. Reserved `this` and `This` receiver forms retain their more
specific receiver-position and duplicate-receiver validation.

#### Value-producing `if`

The incomplete-value cases in 2.4 must return typed diagnostics before expression evaluation.

#### Acceptance

The retained malformed-source fixtures produce no `MOTH-INFRA-0001`.

### 2.6 Delete config-key migration diagnostics

**Status:** Complete in `996fb8b17`.

**Additional audit finding:** `InvalidConfigReason::{DeprecatedSrcKey, ReplacedLibrariesKey,
ReplacedRootFoldersKey, ReplacedPackageFoldersKey}` and their Stage 0 name checks exist only to
teach pre-Alpha config-key migrations. The config registry already has a structured `UnknownKey`
path for every unregistered name. The canonical module plan also records this deletion as
accepted cleanup.

- Delete the four migration-only reason variants and render branches.
- Delete the special name checks for `src`, `libraries`, `root_folders` and `library_folders`.
- Let those unregistered names use the ordinary `UnknownKey` diagnostic without suggesting a
  historical replacement.
- Remove migration-only unit tests, integration fixtures and manifest entries.
- Preserve current `entry_root` positive coverage plus generic unknown-key
  coverage.
- Do not reserve the removed names or add aliases, compatibility parsing or replacement
  diagnostics.
- Render current config key names exactly as authored in `EmptyProjectSetting` and
  `InvalidProjectSettingValue`.
- Apply the same cleanup to compiler comments and test expectations that name current config keys.
  Comments should name the `origin` key directly.
- Search compiler sources and tests after the change to confirm every current config key is rendered
  exactly as authored.

### 2.6b Point config-key diagnostics at the authored key

**Status:** Complete in `efbffa183`.

**Additional confirmed location gap:** the ordinary `UnknownKey` diagnostic currently uses the
folded declaration value's location. A direct config probe underlines the initializer, even though
the invalid source fact is the authored key name. Header parsing already owns the exact
`Header::name_location`, but config parsing discards it when sorted headers are consumed by AST.

- Preserve authored config constant name locations in `ParsedConfigFile` before `Ast::new` consumes
  the sorted headers. Key the narrow Stage 0 map by the existing full declaration
  `InternedPath`.
- Use the authored name location for key-identity diagnostics such as `UnknownKey` and the
  validation-layer duplicate fallback. Keep a defensive value-location fallback for declarations
  without an authored header entry.
- Do not change `Config::setting_locations`: builder-owned invalid-value diagnostics should
  continue to point at the authored initializer value.
- Do not add a location to the shared AST `Declaration` type or create a global header-to-AST side
  table for this Stage 0-only need.
- Strengthen generic unknown-key coverage to assert the key span and keep `MOTH-CONFIG-0001`.
- Preserve the exact authored-key fact and ordinary unknown-key message established in Phase 2.6.

`ParsedConfigFile` now preserves authored constant-name locations before AST construction consumes
the sorted headers. Config validation uses that Stage 0-local map for `UnknownKey` and its duplicate
fallback, keyed by the declaration's existing full path. `Config::setting_locations` remains
value-based for downstream builder diagnostics. Focused coverage asserts the key span and direct
probes protect both diagnostic lanes.

## Phase 3: Mutability, assignment and explicit copy

### 3.1 Consolidate mutable call and receiver diagnostics

**Original findings:** DIAG-001, DIAG-008, DIAG-046
**Original DIAG-007 corrected below**

**Phase 3.1a call-argument status:** Complete in `bd2fa6328`.
**Phase 3.1b receiver status:** Complete in this checkpoint.

Call arguments preserve distinct named-target, value and authored-`~` locations. One call-boundary classifier distinguishes fresh values, existing immutable places and existing mutable places without repeated expression walks. Plain fresh and explicit-copy values satisfy mutable parameters without `~`, while invalid markers and immutable sources retain precise reasons and source-facing guidance. Unnamed host parameters render one-based positions only.

Receiver calls share one source-state classifier across source methods, collection builtins and map builtins. Temporary receivers, immutable places, mutable places missing `~` and invalid authored markers remain distinct. Authored-marker failures point at `~`, named `~binding.method(...)` guidance comes from factual schema facts, and const-record runtime calls use a dedicated reason and the source term `const record`.

Mutable access diagnostics must distinguish these source states:

1. existing mutable place, missing `~`
2. existing immutable place
3. explicit `~` on an immutable place
4. explicit `~` on a fresh or computed non-place
5. plain fresh value passed to a mutable parameter
6. mutable receiver call missing `~`
7. immutable receiver for a mutating method

State 5 is valid and must not be diagnosed.

**Status: Call-access classification is delivered; `RustBorrowPrefix` wording remains.** The shared-argument path classifies place mutability before choosing its repair. Fresh values, including `copy` results, satisfy mutable parameters without `~`; an authored marker on a fresh value remains invalid. Unnamed parameter messages use one-based positions. The `function_call_mutable_param_copy_arg` integration case covers an explicit copy passed to a mutable parameter.

The remaining wording item is the `CommonSyntaxMistakeReason::RustBorrowPrefix` renderer in `src/compiler_frontend/compiler_messages/render/syntax.rs`: the full and terse messages describe shared access as implicit and recommend `~` for mutation without specifying that `~` is valid only on an existing mutable binding or field at a mutable call or receiver boundary.

#### Delivered diagnostic facts

The call and receiver schemas retain callee or method names, parameter or receiver names, mutability, authored-marker state and source spans where available. Fresh values use `FreshMutableValue`; invalid access states retain distinct reasons and source-facing repairs.
#### Messages

Existing mutable place, missing marker:

> Call to `consume` requires explicit mutable access for parameter `values`. Prefix the existing mutable place with `~`.

Immutable place:

> Call to `consume` requires mutable access for parameter `values`, but `values` is immutable. Declare the binding as mutable, then pass `~values`.

Mutable receiver missing marker:

> Mutable receiver method `move` requires explicit mutable access. Prefix the receiver with `~`.

When the receiver is a simple named place, guidance may show `~p.move(...)`. Do not render internal placeholders such as `~this receiver`.

Immutable collection or map receiver:

> `push` requires a mutable collection receiver. Declare the collection binding as mutable, then call it with explicit `~` access.

Show a concrete receiver call only when the family facts include the simple receiver name.

Fallible collection and map examples must include `!` or `catch` when a complete example is shown.

Const-record receiver diagnostics use `ConstRecordNoRuntimeCalls` without an alias and render the source term `const record`.

#### Delivered receiver classification

The shared receiver classifier distinguishes temporary mutating receivers from immutable named receivers and selects their respective reasons and repairs.

#### Tests

Use one matrix covering user functions, source receiver methods, collection builtins and map builtins. Include positive fresh-rvalue calls, including `copy source`, to prevent the change from requiring illegal `~` on fresh values.

### 3.2 Correct immutable assignment and field-write guidance

**Original finding:** DIAG-007
**Rejected original proposal:** `~p.x = 10` is not Moth assignment syntax.

**Status: Delivered.** Assignment-target reasons distinguish temporary values, immutable bindings and immutable field roots. The diagnostics preserve authored binding locations through `ScopeDeclarationRef` and use the schema-defined immutable-binding declaration label.

For:

```moth
p = Point(x = 1, y = 2)
p.x = 10
```

the root binding is immutable. The correct pattern is:

```moth
p ~= Point(x = 1, y = 2)
p.x = 10
```

#### Delivered implementation

The assignment-target family carries field and mutable-root names where available, preserves authored binding locations through `ScopeDeclarationRef`, and attaches the schema-defined immutable-binding declaration label. `TemporaryNotAssignable`, `ImmutableBinding` and `ImmutableFieldRoot` are the current factual reasons. Rendered guidance names bindings and fields, uses ordinary `=`, and avoids compiler-facing place or rvalue terminology.

### 3.3 Diagnose `~` on assignment targets accurately

**Original finding:** DIAG-021
**Rejected original proposal:** `x ~= 2` is a declaration form, not ordinary reassignment.

**Status: Delivered.** `InvalidAssignmentTargetReason::MutableMarkerOnAssignmentTarget` and the `mutable_marker_on_assignment_target_rejected` integration case protect the source-facing marker error and its stable rule code.

For:

```moth
x = 1
~x = 2
```

render:

> `~` is not written on assignment targets. Reassignment uses ordinary `=` and requires an already-mutable binding.

A complete correction is:

```moth
x ~= 1
x = 2
```

The marker error uses its dedicated assignment-target reason rather than call-specific `MutableMarkerOnNonReceiverCall`. An ordinary assignment with an immutable root has separate diagnostic ownership.

### 3.4 Diagnose `copy ~place` as an unnecessary access marker

**Original finding:** DIAG-020

`copy` accepts an existing place and creates independent value semantics. It does not take mutable-access syntax.

**Status: The reason model and rendering are delivered; computed-expression coverage remains.** Current reasons distinguish a function name, a function call, a non-place expression and a mutable marker on an otherwise valid copy target.

The `MutableMarkerNotAllowed` reason applies only when removing `~` leaves a valid binding or field-projection copy target. A marked literal, call or computed expression uses `NonPlace`, because removing the marker does not make it copyable.

Current messages distinguish function names and calls and direct authors to existing bindings or field projections. Integration cases cover a literal, function name, function call and `copy ~binding`. Add a computed-expression non-place case to complete the focused coverage.

### 3.5 Improve borrow-conflict explanations without inventing lifetimes

**Original finding:** DIAG-056

Borrow diagnostics should explain source ordering and aliases, not tell users to manually end a borrow or mention internal analysis state.

#### Implementation

**Status: Delivered.** `StatementAccessTracker` retains the first available source span per root. Active conflicts use the schema-defined `ConflictingAccess` label, rendered as `earlier conflicting access here`. Descriptor titles use source-visible access terms, and renderers distinguish shared and mutable access pairs without exposing lifetime or ownership-analysis state.
Renderers select conflict wording from the existing and requested access kinds.

Mutable alias blocks shared read:

> Cannot read `data` while mutable alias `first` is still needed later. Read through `first`, or move the later use of `first` before this access.

Shared alias blocks mutation:

> Cannot mutably access `data` while shared alias `shared` is still needed later. Move the mutation after the last use of `shared`, or create an explicit copy when independent data is required.

Second mutable access:

> Cannot create another mutable access to `data` while `first` is active. Reuse `first`, or move the new access after its last use.

When source-place or location facts are unavailable, renderers use generic fallbacks. The messages do not describe `~` as a move or expose exact lifetimes or ownership flags.

#### Tests

Passing coverage includes reordering (`borrow_conflict_resolved_by_reordering`), a narrowed scope (`borrow_checker_alias_not_live_after_scope`) and explicit `copy` (`borrow_conflict_resolved_by_copy`). Add a positive case that reuses the active alias.

### 3.6 Remove migration state from collection and map assignment diagnostics

**Status: Delivered.** The current `InvalidAssignmentTargetReason` schema uses `CollectionGetTargetNotWritable`, `MapGetTargetNotWritable` and `ReadOnlyMapProperty`. Renderers explain the current `get`/`set` rule and read-only `length` property. Collection and map statement tests assert the stable `MOTH-RULE-0044` reason keys.

## Phase 4: Error, option and value-flow diagnostics

### 4.1 Fallible handling: delivered facts and remaining presentation

**Original findings:** DIAG-025, DIAG-043, DIAG-044

**Delivered terminology and operand families:** `MOTH-RULE-0051` renders `Invalid fallible handling`. `InvalidFallibleHandlingReason` distinguishes authored handlers and operand channels, replacing the umbrella `NotResultExpression` path. Error-type mismatches carry `TypeMismatchContext::ErrorReturn` as semantic context. Capture displayable types in `TypeDisplaySnapshot` and render through `DiagnosticView`. Builtin calls use `InvalidBuiltinCallReason::UnhandledFallibleCall`. The discontinued bang-handler recognisers and their dedicated diagnostic reasons are absent.

The native result-channel work also deleted `InvalidFallibleOperand` and its reason family
and `MOTH-TYPE-0004`. Unhandled error-returning operands use
`InvalidFallibleHandlingReason::UnhandledErrorReturn` under `MOTH-RULE-0051`.
The `result_operand_without_handler_in_operator` fixture expects that code and the
`invalid_fallible_handling.unhandled_error_return` reason at `input/@page.moth:5:9`.

**Phase 4.1c transfer, delivered in `7f87109d9`:** the compact typed-expression and native result
channel work supplies one enclosing callable/entry contract fact for postfix `?` and `!`, including
nested branches. Consumers don't guess from the immediate `ContextKind` or rebuild signatures.
Postfix `?` requires exactly one compatible optional success slot. Its
`invalid_fallible_handling.function_has_multiple_success_returns` reason carries the authored
success-slot count, separately from a missing optional return. The selected entry contract has
built-in `start` `Error!`, so compatible top-level `!` remains legal. An incompatible entry error
uses `invalid_fallible_handling.entry_has_incompatible_error`, separately from a function without
an error slot. Both reasons retain `MOTH-RULE-0051`.

**Remaining catch presentation scope:** refine eligibility and recovery guidance through the
existing reason family. Plain-value rejection should explain the absence of compatible recoverable
failure and authored safety-catch eligibility. Optional-value rejection should guide authors
towards explicit inspection. Keep authored handler, operand channel and expression boundary clear
without describing a source Result carrier. This scope adds no propagation-boundary owner or
replacement reason matrix. Section 4.2 retains the catch-recovery type-context work.

#### Reclassify invalid public carrier type spellings

**Status: Delivered.** Type lookup rejects `Option` and `Result` generic applications through `InvalidGenericInstantiationReason::OptionTypeSyntaxNotSupported` and `InvalidGenericInstantiationReason::ResultTypeSyntaxNotSupported`, under `MOTH-RULE-0057`. Rendered messages identify unsupported type syntax and direct authors to `T?` or a final `E!` slot. The focused owners in `src/compiler_frontend/declaration_syntax/tests/type_syntax_tests.rs` protect both cases.

### 4.2 Add catch-recovery type context

**Original finding:** DIAG-026

Add `TypeMismatchContext::CatchRecovery`.

The receiver mapping in `src/compiler_frontend/ast/statements/value_production/parse_values.rs` maps `ValueReceiverKind::CatchHandler` to `TypeMismatchContext::General`. Add `CatchRecovery` to the enum in `src/compiler_frontend/compiler_messages/diagnostics/reasons.rs` and assign it a new compact schema code in `src/compiler_frontend/compiler_messages/diagnostics/families/types_borrows_targets.rs` without changing existing codes.

Update `unit_fact_round_trips_every_type_mismatch_context` in `src/compiler_frontend/compiler_messages/diagnostics/tests/codec_tests.rs` to include code 18 and round-trip `CatchRecovery`. Add the `CatchRecovery` arm to `type_mismatch_context_name` in `src/compiler_frontend/compiler_messages/render/context.rs` so the message uses “catch recovery.”

Update the existing `General` assertion in `src/compiler_frontend/ast/statements/tests/fallible_handling_tests.rs` to expect `CatchRecovery`. Keep declaration, assignment, return, multi-bind and nested-`then` contexts unchanged.

Message:

> Type mismatch in catch recovery: expected `I32`, found `String`.

Guidance may explain that each `then` value must match the corresponding success slot. `TypeMismatchContext` remains a semantic fact. Keep expected and found `TypeId`s as typed inputs at the owning boundary, capture display data as `TypeDisplayId`s in `TypeDisplaySnapshot`, and render through `DiagnosticView`.

### 4.3 Reject statement-match expression bodies at the expression

**Original finding:** DIAG-003

A statement match and a value-producing match are different source shapes. Do not suggest inserting `then` without also moving the match to a receiving site.

Message:

> This is a statement match, so this arm body must contain statements. To compute a value, place the match at a declaration, assignment, return, multi-bind or nested `then` receiver and use `then` in every producing arm.

The primary label belongs on the bare expression body, not the next arm header.

Tests need:

- statement match with a bare expression body
- valid statement match
- valid value-producing match at a declaration receiver
- exhaustive choice coverage or `else` so the fixture does not fail for an unrelated reason

### 4.4 Single-target/multiple-result rejection delivered through native results

**Original finding:** DIAG-031

**Disposition: transferred and delivered in `00e4ad555`.** The compact typed-expression and native
result-channel work enforces receiver arity before coercion or lowering through `InvalidValueReceiver`
under `MOTH-RULE-0087`, without using `InvalidMultiBind` for source that has no multi-bind.
The family facts record receiver kind, target count, produced success-slot count and authored producer
location. Guidance asks for one target per success slot without `_`, implicit discards, fabricated
names or a source-tuple suggestion.

The delivered coverage includes declaration, assignment, return and nested value-producing
receiver rejection, matching multi-bind and multi-result forwarding. Ordered production computes
all right-hand results before writing existing assignment targets.

This plan has no remaining implementation slice for DIAG-031. Its compact diagnostic schema
consumes the delivered family and primary tests rather than recreating tuple-based checks or
duplicating coverage.

### 4.5 Use the same `then` diagnostic at top level and in functions

**Original finding:** DIAG-032

**Delivered routing:** Module and function body dispatch emit `ThenWithNoActiveValueTarget`. The remaining work is source-visible wording and ownership cleanup: the `result_then_outside_catch_rejected` fixture ID, path and contract still describe `then` as catch-specific even though value-producing `if`, full match and catch all own it.

Render the structured reason with source-visible guidance:

> `then` is only valid inside a value-producing `if`, full match or `catch` recovery that has an active receiving site.

Preserve the structured reason rather than falling back to a generic unexpected-token diagnostic.

### 4.6 Reject bodyless non-`else` match arms

**Original finding:** DIAG-052

A direct exhaustive-choice probe confirmed that `Ready =>` followed by `else =>` currently
compiles successfully. This is a missing rejection, not only weak wording.

Only bodyless `else =>` is the explicit statement no-op arm.

Add `InvalidMatchArmReason::MissingBody`:

> This match arm has no body. Add a statement after `=>`. Only `else =>` may be bodyless.

Primary label: the arm arrow or empty body boundary.

Keep positive coverage for bodyless `else =>`. Value-producing matches must continue to require produced values on every selected path.

### 4.7 Delete legacy match-arm compatibility diagnostics

**Additional audit finding:** `InvalidMatchArmReason::{LegacyColonSyntax, LegacyElseSyntax}` and
the `current_line_contains_top_level_colon` pre-scan exist only to identify discontinued match-arm
grammar. They preserve migration history in the current parser and renderer.

- Delete both reason variants and their render branches.
- Delete the colon pre-scan and its one-caller helper when it becomes unused.
- Remove the migration-only unit test for colon arms.
- Let `pattern:` reach the ordinary current missing-`=>` or invalid-arm-header diagnostic.
- Let `else:` reach the ordinary current expected-`=>` diagnostic.
- Preserve `InvalidArrow` for authored `->`. That reason factually identifies the current token
  mistake and suggests `=>` without describing a former language feature.
- Do not add replacement recognisers, fixtures or compatibility messages.

### 4.8 Delete the removed `in` loop compatibility diagnostic

**Additional audit finding:** `InvalidLoopHeaderReason::RemovedInSyntax`,
`reject_removed_in_loop_syntax_cursor` and their migration-only unit test exist only to recognise the
discontinued `loop <binding> in ...` grammar. The ordinary loop-header parser already owns malformed
current headers.

- Delete the reason variant, renderer branch and dedicated pre-scan.
- Delete the migration-only unit test.
- Let the authored tokens reach the ordinary current loop-header diagnostic path.
- Preserve factual current diagnostics for missing pipes, missing sources and invalid binding
  shapes.
- Do not add a replacement recogniser or describe any loop syntax as old or removed.

## Phase 5: Names, dependencies, calls and match context

### 5.1 Correct generic constructor guidance

**Original finding:** DIAG-002
**Rejected original proposal:** `x ~Box of String = ...` and `Box of String { ... }` are not the intended constructor pattern.

`Box of String` is type annotation syntax. Construction still uses the nominal constructor name:

```moth
x Box of String = Box(value = "hello")
```

#### Implementation

When a generic type application is parsed in value position, report:

> `Box of String` is a type annotation, not a value expression. Construct the value with `Box(...)` and provide a concrete receiving type when inference needs it.

Keep ordinary type-as-value diagnostics for non-generic cases.

### 5.2 Add scope-correct type-name suggestions

**Original finding:** DIAG-047
**Original DIAG-033 disposition:** Removed. An unknown uppercase name is not enough evidence that the author forgot a generic `type` parameter.

Add candidate data to `UnknownTypeName` from the active type-resolution scope:

- builtin types
- visible local nominal types
- visible aliases
- visible dependency-bound and external types
- active generic parameters

Use the existing bounded edit-distance policy. Suggest only a sufficiently close visible candidate.

Examples:

- `Strng` -> `String`
- close local type typo -> local type
- unrelated unknown name -> no suggestion

Do not search private, dependency-unbound or out-of-scope types.

### 5.3 Preserve namespace context and suggest direct members

**Original findings corrected:** DIAG-009, DIAG-048
**Original DIAG-040 disposition:** Removed. `math.PI` is valid namespace access.
**Original DIAG-022 disposition:** Removed. Delimiter-free direct selections of external exports are valid.

Add a namespace-member diagnostic that carries:

- namespace path or local alias
- requested member
- direct visible member names
- package/source namespace kind

Messages:

- unqualified direct member that exists in a dependency namespace:

  > Unknown value `PI`. It is available as `math.PI` from the `@core/math` namespace. Use `@core/math PI` if you want the bare name.

- qualified typo:

  > Unknown member `pi` on namespace `math`. Did you mean `PI`?

- unrelated qualified member:

  > Unknown member `nope` on namespace `math`.

Rules:

- Source namespace records remain shallow.
- External namespace records may expose recursive package-local namespaces.
- Suggest only members of the exact resolved namespace node.
- Filter candidates by the source role at the failing segment. A dotted path with another `.`
  may suggest child namespaces only, while a final value-position member may suggest value members
  only. Do not suggest a type or child namespace as though it were a value.
- For an unqualified unknown value, offer namespace qualification only when exactly one visible
  namespace has that direct value member. If several namespace aliases expose the same name, keep
  the ordinary unknown-value diagnostic rather than choosing one arbitrarily.
- Do not suggest receiver methods as namespace fields.
- A direct-selection suggestion is valid only for a direct export.
- Sort and deduplicate candidate names before rendering so hash-map iteration cannot affect the
  chosen suggestion or available-member list.

Extend `MissingPackageSymbol` with the same bounded direct-export suggestion policy so `@core/math pi` may suggest `PI`.
Build those direct-selection candidates from one-component function, constant and type paths on the
matched external package. Nested package-local paths are not direct exports and must not be
offered as direct selections.

### 5.4 Include actual and expected argument counts

**Original finding:** DIAG-045

Extend `ExtraPositionalArgument` with `provided_count`. The extra argument is already the
diagnostic's primary span.

Message:

> Call to `add` has 3 positional arguments, but accepts 2.

Guidance:

> Remove the extra argument starting here.

Do not suggest named arguments. Renaming arguments does not fix excess arity.

Cover user functions, struct constructors and choice constructors where they share or intentionally differ in call-shape ownership.

### 5.5 Enrich choice match-pattern diagnostics

**Original findings:** DIAG-049, DIAG-050, DIAG-051

#### Unknown variant

Keep the source-visible choice name and requested variant as schema-typed facts; store available variants in schema-declared typed side-store list data, bounded and deterministic.

Message:

> Unknown variant `Color::Gren`. Did you mean `Color::Green`? Available variants: `Red`, `Green`, `Blue`.

Do not offer a close-name suggestion when the threshold is not met.

#### Qualifier mismatch

Keep the authored qualifier and scrutinee's source-visible choice name as schema-typed facts:

> Match arm uses qualifier `Size`, but the scrutinee is `Color`. Use a `Color` variant or omit the qualifier.

Dependency aliases must render using the visible source name where possible.

#### Payload captures

Keep expected and found counts as fixed schema-typed facts. Store expected field names and provided capture names in schema-declared typed side-store lists.

Count mismatches must point at the authored pattern's capture-list boundary. The current too-few
path uses the choice variant declaration location, which identifies the related declaration rather
than the mistake. Keep the declaration as secondary context when useful, but make the closing `)`
or first extra capture the primary location.

Messages:

- name mismatch:

  > Capture `missing` does not match payload field `value` in variant `Ok`. Use `Ok(value)` or rename it with `Ok(value as missing)`.

- count mismatch:

  > Variant `Ok` has 1 payload field, `value`, but this pattern captures 0.

Keep exact field lists bounded, deterministic and in the schema-declared typed side store.

#### Optional-pattern terminology

The same renderer still calls source values and captures `Option` in several match diagnostics.
Rename the affected `InvalidMatchPatternReason` and `NonExhaustiveMatchReason` variants to
optional-value terminology, then render:

> Optional value patterns require the optional value's inner type to support equality.

> An optional-present capture cannot be empty. Use `|name|` to capture the present value.

> Non-exhaustive optional-value match. Add `none =>` or `|name| =>` to cover all cases.

Apply the same terminology to the non-optional-scrutinee, type-annotation and missing-binding
reasons. Internal AST `OptionPresentCapture` node names are outside this diagnostic wording slice
and do not need a semantic refactor.

### 5.6 Keep scalar type-call diagnostics factual

**Additional confirmed audit finding:** `InvalidBuiltinCallReason::ScalarConstructorRemoved`,
several `*_constructor_removed` fixtures and the dead
`InvalidCastReason::ScalarConstructorRemoved` encode historical type names rather than the current
authored mistake. The accepted numeric inventory has no builtin `Int`, `Uint` or `Float`
identities. Those spellings receive ordinary name or parse diagnostics, with no compatibility
reason or alias. The parser still needs a factual diagnostic when an active exact scalar type is
called as a function.

- Use one factual type-as-call reason for active scalar types.
- Render:

  > `I32` is a type, not a conversion function. Use `cast` at an explicit typed receiving boundary.

Keep the exact active builtin type name as a schema-typed fact available at the emission site.
- Delete `InvalidCastReason::ScalarConstructorRemoved` and its render branch.
- Rename focused fixtures and tests around the current rejection. Remove redundant per-scalar
  coverage only when one table test or primary integration case protects the same parser contract.
- Preserve positive `cast` coverage. Do not add a compatibility path for removed scalar names.

## Phase 6: Template and external-JavaScript diagnostics

### 6.1 Report exact template-head conflicts and why they conflict

**Original finding:** DIAG-014
**Confirmed decision:** Every incompatible directive pair is rejected with both names and a semantic explanation.

The current compatibility state stores only aggregate tags. Once a conflict is detected, the parser has lost the identity and location of the earlier item.

#### Implementation

Replace the bitset-only diagnostic path with retained seen-item facts:

```rust
struct SeenTemplateHeadItem {
    display_name: TemplateHeadItemName,
    presence_tags: TemplateHeadTag,
    location: SourceSpan,
}

enum TemplateHeadConflictReason {
    MultipleFormatters,
    DirectiveMustBeOnlyMeaningfulItem,
    DuplicateExclusiveDirective,
    SlotDirectiveMustBeExclusive,
    // add narrow reasons required by registered compatibility rules
}
```

`TemplateHeadState` may still retain aggregate tags for fast checks, but it must also retain enough ordered metadata to identify the first concrete conflicting item.

Every compatibility rule that can reject a pair must provide a `TemplateHeadConflictReason`. Registry validation must reject a builder directive specification that blocks another item without declaring a renderable conflict reason.

Diagnostic family facts:

- earlier item name and source span
- incoming item name and source span
- structured conflict reason encoded by the family schema

Examples:

> `$md` and `$raw` cannot be combined because both control template body formatting. Choose one formatter.

> `$note` cannot be combined with `$children` because a discarded comment directive must be the template's only meaningful head item.

- Primary label: incoming item, with a schema-defined label message code and its source span
- Secondary label: earlier conflicting item, with a schema-defined label message code and its source span
- Keep `MOTH-SYNTAX-0022`

#### Tests

- `$md` with `$raw` in both orders
- duplicate formatter
- comment directive with another meaningful item
- `$slot` exclusivity
- builder-registered directive conflict
- compatible combinations remain accepted

### 6.2 Improve unclosed-template EOF context

**Original finding:** DIAG-010

Track the opening `SourceSpan` in the template-mode stack.

Message:

> This template is not closed before the end of the file. Add `]`.

Labels:

- primary at EOF
- secondary at the opening `[` with `Template starts here`

Use the same opening-location support for truncated template heads, bodies and nested templates. Do not create separate ad hoc EOF strings for each parser.

### 6.3 Bind `@moth.sig` to the nearest JavaScript declaration

**Original finding:** DIAG-042

The annotation scanner currently sees only supported exports, so an annotation before a plain function may drift to a later export.

#### Implementation

- Scan an ordered stream of supported exported declarations and supported-looking unexported declarations.
- Bind `@moth.sig` only to the nearest following top-level declaration.
- Permit only whitespace and comments between annotation and declaration.
- Add a typed external-JS reason `MissingExportKeyword`.
- For a plain function:

  > `@moth.sig` for `add` applies to JavaScript function `add`, but the function is not exported. Add `export` before `function`.

- For a block-bodied arrow:

  > `@moth.sig` for `add` applies to JavaScript constant `add`, but it is not exported. Add `export` before `const`.

- Consume the matched unexported declaration after reporting so the annotation cannot drift.
- Keep a separate orphaned-annotation reason when no supported declaration follows.
- Preserve the typed provider reason through `InvalidExternalModule` conversion rather than flattening every failure to an opaque string.
- Primary label: insertion point at the declaration
- Secondary label: annotation

#### Tests

- plain function missing `export`
- block-bodied arrow missing `export const`
- missing-export declaration followed by a valid annotated export
- private helper between annotation and export prevents distant binding
- genuinely orphaned annotation
- provider-level rendering preserves `MOTH-IMPORT-0022` and the JavaScript source span

### 6.4 Keep standalone template-helper diagnostics source-visible

**Additional audit finding:** `InvalidTemplateStructureReason::HelperOutsideWrapperSlot` currently tells the user that a helper reached AST finalization. The source mistake is a standalone `$insert(...)` contribution, not the compiler stage that detected it.

Render:

> `$insert(...)` is a template contribution helper, not a standalone value. Keep it inside a template application that receives the contribution.

- Do not mention AST, TIR, finalization or internal helper-artifact policy.
- Keep the diagnostic at the standalone helper expression.
- Add an integration assertion for the existing standalone-insert rejection so the source-visible wording is contractual.

## Phase 7: Remaining focused quality improvements

### 7.1 Body-local redeclarations should use the normal no-shadowing path

**Original finding:** DIAG-023

**Additional confirmed gap:** ordinary body-local typed and mutable redeclarations still use the `SHADOWED_NAME` family in `src/compiler_frontend/compiler_messages/diagnostics/families/names_declarations.rs` and `MOTH-RULE-0038`. That path duplicates `DuplicateDeclaration` and leaves the scope-neutral message from Phase 2.5 unused for normal local bindings.

**Additional confirmed dispatch gap:** the existing-symbol branch recognises only builtin type
tokens and the mutable marker as declaration starts. A redeclaration with a nominal, alias,
generic, qualified or collection annotation or the postfix compile-time marker can fall through
to expression parsing. Declaration-shape detection must reuse the canonical
binding-target parser rather than grow another token whitelist. Plain `name = value` remains
reassignment, not a second inferred declaration.

- Remove the current `SHADOWED_NAME` family, its descriptor, constructor, remapping and renderer branch.
- Route ordinary body-local typed and mutable redeclarations through `DuplicateDeclaration`.
- Parse every unambiguous current body-local binding declaration shape before duplicate
  registration rejects it, including user-defined/constructed type annotations and postfix
  compile-time markers. Keep plain `name = value` on reassignment.
- Reuse `DuplicateDeclaration`.
- Reuse the scope-neutral duplicate-name message established in Phase 2.5.
- Use the authored binding location preserved by Phase 3.2 and label both declarations.
- Update the existing `MOTH-RULE-0038` fixtures to `MOTH-RULE-0002`. Do not reserve or retain the
  superseded code through an alias.
- Consume the surviving binding grammar after Reactivity V1 retirement. Do not preserve removed
  declaration syntax or a migration-specific uniqueness rule.

### 7.1b Pattern captures should use the normal no-shadowing path

**Additional confirmed gap:** option-present captures and declared choice payload captures
currently use `InvalidMatchPatternReason::CaptureBindingShadowsVariable`. That parallel
reason carries neither the authored capture name nor the previous declaration location, uses
source-inaccurate `variable` terminology and reports `MOTH-RULE-0049` for the ordinary language-wide
no-shadowing rule. General full-match captures were removed by the language migration and are not
part of this diagnostics slice; those forms must remain rejected.

- Delete `CaptureBindingShadowsVariable` and its renderer branch.
- When a capture collides with a visible binding, reuse `DuplicateDeclaration` with the capture name
  and capture location.
- Use the factual authored binding location exposed by `ScopeDeclarationRef` after Phase 3.2 for the
  secondary label. Omit the secondary label when a visible symbol has no authored binding location.
- Preserve distinct `DuplicateCaptureBinding` handling for two captures inside the same pattern.
- Keep the current capture location primary and preserve option-present and declared choice-payload
  capture parsing. Do not preserve or reintroduce general full-match capture parsing.
- Update the existing capture-shadowing integration owners to `MOTH-RULE-0002` rather than adding
  cosmetic duplicate fixtures.

### 7.2 Exact operator diagnostics consolidated into Phase 1.2

**Original findings:** DIAG-015, DIAG-030
**Status:** Moved earlier

The exact operator facts and the `not`, string concatenation and generic-parameter messages now belong to Phase 1.2a. The generic-bound terminology fix already requires exact operator facts, so retaining a later category-only-to-exact migration would create transitional API and duplicate renderer work.

### 7.3 Add actionable collection-loop guidance

**Original finding:** DIAG-055

Keep the found semantic type as a family fact, capture its display data in `TypeDisplaySnapshot` and render through `DiagnosticView`.

Message:

> Collection loop source must be a collection, found `I32`. Use a collection after `loop`. For numeric iteration, use range syntax such as `loop 0 to 10 |i|:`.

Do not imply every non-collection source was intended as a range.

The `RemovedInSyntax` reason and its `reject_removed_in_loop_syntax_cursor` pre-scan are deleted by
Phase 4.8; this slice does not repeat that deletion.

### 7.4 Reject unsupported Wasm variant payloads before lowering

**Status: Delivered.** Wasm backend feature validation records `UnsupportedBackendFeatureReason::VariantPayloadValues` for reachable variant constructions with non-empty payload fields before lowering. Empty/unit variants are not rejected by this check, and the validation pass examines only selected blocks. The `cast_optional_success_wraps_inner_value` integration case preserves HTML success and expects `MOTH-RULE-0064` with `unsupported_backend_feature.variant_payload_values` for HTML-Wasm at the source expression.

## Phase 8: Doc comments and code comments review

Do an extensive review of comments across the codebase that may be stale, drifting from design docs or are enabling misunderstanding features or language surface.

This review should focus on making sure areas where diagnostic improvements have been corrected by this plan are not also commented with incomplete or outdated information.

This report should be created by exploring the codebase in parallel, then coalesing the reports into a file kept in the tmp/ folder and reviewing them for accuracy before implementing the corrections.

Any bad, noisy or pointless comments that don't follow the style guide can be in scope for this review. Ideally, line counts for comments should be reduced, compressed and made more concise without losing important context rather than further bloated with much more information.

**Confirmed cleanup:** `headers/tests/parse_file_headers_tests.rs` still calls current postfix
constant declarations "`#`-prefixed declarations", and `headers/header_dispatch.rs` says dispatch
classifies a declaration by its "leading token" while `#` is the marker after the declaration name.
`headers/hash_items.rs` also describes its separate `#[]` const-template owner as a general
"hash-prefixed top-level forms" path, while another header test describes an ordinary runtime
template as having "no `#` prefix". Rewrite these comments around the exact current constructs:
postfix constant markers, the token after a declaration name and the distinct `#[]` const-template
form. No compiler or test wording may imply the discontinued leading-marker constant form.


## Removed or superseded original findings

These items must not be implemented as originally proposed.

| Original | Disposition | Reason |
|---|---|---|
| DIAG-001 | Retained, rewritten | The original snippet used an immutable collection, so `~values.push(...)` alone was not a valid fix. |
| DIAG-002 | Retained, rewritten | Generic construction uses `Box(...)` at a concrete receiving type, not `Box of T { ... }` or `~Box`. |
| DIAG-006 | Removed, historical rationale superseded | The original snippet was not a const-required cast. Its old handling rationale relied on the removed `cast!` spelling. Plain `cast` now uses ordinary implicit-failure delivery. Keep the rejection of the proposed literal-failure branch, which conflated evidence selection with const failure. |
| DIAG-007 | Retained, reversed | The binding does need to be mutable. Field assignment is `p.x = ...`, not `~p.x = ...`. |
| DIAG-017 | Removed as stale | Current rendering identifies `|`, not `/`. A broader parser-context change is not justified without a current reproducible failure. |
| DIAG-019 | Removed as ambiguous, historical example | At the recorded checkpoint, `MyList {Int}` parsed as a typed declaration missing an initializer. Exact numeric types now replace that old spelling. The compiler still cannot infer that a missing initializer meant a type alias. |
| DIAG-022 | Removed as incorrect | Direct external package exports support delimiter-free direct selections. A missing symbol may receive a close-name suggestion instead. |
| DIAG-029 | Removed as incorrect | Raw strings intentionally preserve backslashes and newlines. |
| DIAG-033 | Removed as ambiguous | Unknown type `A` does not prove that a generic `type A` declaration was intended. |
| DIAG-034 | Removed as resolved | Current expression typing already emits `BinaryRight` when the right operand is absent. |
| DIAG-036 | Removed as resolved | Mixed map entries and missing map values already use separate structured reasons. Rendering a complex key as a fabricated repair is not required. |
| DIAG-039 | Replaced by strict deletion | No legacy-specific rejection remains. |
| DIAG-040 | Removed as incorrect | External package namespace access is supported. The example also used the wrong member casing. |
| DIAG-013 and DIAG-027 | Consolidated | One terse-render fallback fixes the shared root cause. |
| DIAG-012 and DIAG-024 | Consolidated | Both belong to incomplete value-producing `if` parsing. |
| DIAG-025, DIAG-043 and DIAG-044 | Consolidated | One source-aware fallible/optional handling matrix owns the distinction. |

## Implementation order

Implement in this order to avoid parallel diagnostic paths:

1. Phase 1 render fallback, terminology cleanup and compatibility deletion
2. Phase 2 tokenizer and parser boundary reasons
3. Phase 3 mutable access, assignment and copy reasons
4. Remaining Phase 4 fallible/optional handling and value receivers, consuming transferred native
   result-arity and propagation-context diagnostics rather than implementing them again
5. Phase 5 visibility-aware suggestions and match payload facts
6. Phase 6 template compatibility and external-JS scanner binding
7. Phase 7 remaining focused improvements
8. Phase 8 codebase comments audit
9. Final repository-wide diagnostic audit and validation

After each phase:

- remove superseded reason variants and render branches
- search for duplicate messages that encode the same rule
- check that no later stage compensates for a now-earlier diagnostic
- review fixture overlap and keep one primary owner

## Test strategy

### Integration cases

Use `tests/cases/` for:

- quoted-string accepted and rejected source behaviour
- dependency syntax
- malformed value-producing control flow
- mutable calls and receiver calls
- assignment and copy syntax
- result/option handling
- multi-return receiving through its delivered native semantic owner, without a duplicate fixture
- namespace and type suggestions where rendered wording is contractual
- match patterns
- template directive conflicts
- malformed templates
- JavaScript import provider diagnostics
- borrow conflicts

Assert:

- stable diagnostic code
- expected source location when location is part of the fix
- a message fragment only where wording or named context is the feature

### Unit tests

Use focused subsystem tests for:

- quoted-string decoding
- operator-spacing side classification
- terse fallback
- edit-distance thresholds and candidate filtering
- template compatibility conflict selection
- JavaScript annotation-to-declaration binding
- diagnostic side-store ID remapping for every new `StringId` or `PathId` field
- descriptor-code uniqueness after adding and removing kinds

### Regression requirements

- No negative fixture should pass only because a later unrelated diagnostic fires.
- Positive counterparts must protect the intended valid Moth pattern.
- Do not use benchmark fixtures as diagnostic coverage.
- Remove fixtures tied only to deleted legacy paths.

## Documentation exclusion

Do not update general design docs, language docs, the progress matrix or generated documentation
as part of this plan. A narrow documentation edit is allowed only when the file explicitly names
the exact diagnostic being changed and would otherwise become inaccurate. This plan continues to record
slice state and confirmed implementation findings.

## Final validation

Run targeted tests during each phase. Before completion:

```text
cargo fmt
just validate
```

Also perform the manual diagnostic audit required by the compiler style guide:

- every user mistake uses a typed `DiagnosticDraft` through its owning family constructor
- every family's facts fit its schema and any additional data uses typed side stores
- every source location points to the actual mistake
- no stale or legacy message path remains
- no suggestion teaches invalid syntax
- no duplicate diagnostic owner remains across tokenizer, headers, AST, HIR or borrow validation
- no semantic type decision compares rendered text instead of `TypeId`
- no borrow diagnostic exposes compiler-internal lifetime or ownership machinery

## Completion criteria

- All retained cases have end-to-end coverage.
- All removed findings are absent from the implementation backlog.
- Quoted strings implement exactly the confirmed escape set.
- Raw strings preserve literal backslashes and newlines.
- No deleted syntax has a dedicated compatibility surface.
- Every incompatible directive diagnostic names both items and explains the conflict.
- Mutable diagnostics distinguish missing `~`, immutable places, fresh values and invalid assignment markers.
- Fallible and optional diagnostics name the authored operator and correct error channel or
  optional value shape.
- Terse rendering always supplies a non-empty message.
- Malformed user source in this plan never produces `MOTH-INFRA-0001`.
- `just validate` passes.
