# Moth Compiler Diagnostics Improvement Plan

## Purpose

Improve Moth diagnostics without teaching invalid syntax, legacy behaviour or patterns that fight the language design.

Every retained item in this plan has been checked against the current language and compiler contracts. Several findings from the original audit have been corrected, consolidated or removed because their proposed fixes were stale, ambiguous or semantically wrong.

This is an implementation plan, not a research backlog. Each phase should leave one current diagnostic path, remove superseded variants and add end-to-end coverage for the user-visible behaviour it changes.

## Required authority documents

- `docs/compiler-design-overview.md` for diagnostic lanes, type rendering and stage ownership
- `docs/build-system-design.md` when config, graph or output diagnostics are in scope
- `docs/src/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`

## Architecture alignment

This plan does not own config or graph architecture. It consumes the accepted architecture from the two authority documents.

Key alignment rules:

- use graph outcome terminology for diagnosed and blocked modules
- preserve one diagnostic set per canonical module
- keep warnings only on successful artefacts
- keep target-contract diagnostics shared by build and check

Future diagnostic coverage needed for:

- config sections and section-aware schema validation
- `@project` field collisions and provenance
- source input contract conflicts and restricted defaults
- package facade provenance rejection
- output manifest conflicts

## Current state

ACTIVE_PLAN: `docs/roadmap/plans/compiler-diagnostics-improvement-plan.md`
STATUS: active
CURRENT_SLICE: Phase 4.1b+d complete; 4.1c (multiple-success return-shape reason) remains
LAST_ACCEPTED_COMMIT: `d7fb3654f` (Phase 4.1d)
WORKTREE: not refreshed during this documentation-only tidy; record the current branch, HEAD and `git status --short` before Phase 4.1c
REQUIRED_RELOADS: startup files, this plan and current source/diff
RELEVANT_CONTEXT_NOW:
- docs: access-and-aliasing and assignment contracts distinguish immutable root bindings from assignment-target syntax; no general documentation edits are allowed
- code: Phase 3.2 renamed NotMutablePlace→TemporaryNotAssignable and ImmutableVariable→ImmutableBinding, added ImmutableFieldRoot for field writes through an immutable root binding, extended the payload with field_name and root_binding_name, and preserved the authored binding location in the scope-frame so ImmutableBinding and ImmutableFieldRoot carry a secondary label at the original immutable declaration; ExpectedAssignmentOperator says binding not variable
ACCEPTANCE_CRITERIA:
- temporary, immutable direct-binding and immutable field-root assignment targets use distinct reasons and clean wording without `place`/`rvalue`/`variable` terminology
- field-write diagnostics name the field and root binding; a generic fallback exists when the root cannot be named
- `~` is never suggested on assignment targets; reassignment guidance uses ordinary `=`
- stable `MOTH-RULE-0044` is preserved; no new diagnostic code
VALIDATION_STATE:
- Phases 1–2.6b and 3.1a–3.1b: every checkpoint passed its focused checks and `just validate`; completed phase notes retain design and correction details
- Phase 3.2a Ollama patch: passed `cargo fmt`, 3,609 Rust tests, 1,777 integration cases and library Clippy
- Phase 3.2a parent probes: `p.x = 99` (immutable root) renders `ImmutableFieldRoot` naming field `x` and root `p`; `value = 200` (immutable direct) renders `ImmutableBinding` with ordinary `=` guidance and no `~`; `Point(1,2).x = 5` (temporary) renders `TemporaryNotAssignable`; mutable `p ~= Point(1,2)` + `p.x = 99` compiles cleanly
- Phase 3.2a `just validate`: passed cross-target Clippy, 3,609 Rust tests, 1,777 integration cases, docs check and 28 benchmark cases
DOCS_IMPACT: diagnostics-plan tracking only; general design and language docs remain unchanged unless they explicitly name a diagnostic being changed
BLOCKERS_OR_OPEN_DECISIONS: none
DELEGATION_DECISION: Ollama - explicitly required for every implementation worker slice
NEXT_WORKER_ORDER: Ollama only for implementation slices
STOP_REASON: none
NEXT_RESUME_ACTION: launch Phase 4.1c (multiple-success return-shape reason + ScopeContext boundary fact), then Phase 4.2
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

- Emit user mistakes as typed `CompilerDiagnostic` values. Do not route malformed source through `CompilerError` or `MOTH-INFRA-0001`.
- Keep diagnostic payloads factual. Carry names, `TypeId`s, operators, counts, locations and reason enums rather than pre-rendered prose.
- Render type names through `DiagnosticRenderContext`.
- Reuse an existing stable code when the diagnostic category and contract remain the same. Add a new code only for a genuinely new diagnostic family.
- Preserve the source location that identifies the actual mistake. Add a secondary label for the related declaration, delimiter, conflicting directive or active access when available.
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

The terse renderer now uses one render-boundary fallback from an empty payload message to the
descriptor title. Focused and all-kind invariant tests protect non-empty messages without changing
stable codes, locations or the terminal and dev-server formats.

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
- Do not create a function-scope `CompilerError`.
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
- Preserve the exact authored-key payload and ordinary unknown-key message established in Phase
  2.6.

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

Call arguments now preserve distinct named-target, value and authored-`~` locations. One
call-boundary classifier distinguishes fresh values, existing immutable places and existing mutable
places without repeated expression walks. Plain fresh and explicit-copy values satisfy mutable
parameters without `~`, while invalid markers and immutable sources retain precise reasons and
source-facing guidance. Unnamed host parameters render one-based positions only.

Receiver calls now share one source-state classifier across source methods, collection builtins and
map builtins. Temporary receivers, immutable places, mutable places missing `~` and invalid authored
markers remain distinct. Authored-marker failures point at `~`, named `~binding.method(...)`
guidance is emitted only from factual payload data, and const-record runtime calls use
`ConstRecordNoRuntimeCalls` with the current `const record` source term.

Mutable access diagnostics must distinguish these source states:

1. existing mutable place, missing `~`
2. existing immutable place
3. explicit `~` on an immutable place
4. explicit `~` on a fresh or computed non-place
5. plain fresh value passed to a mutable parameter
6. mutable receiver call missing `~`
7. immutable receiver for a mutating method

State 5 is valid and must not be diagnosed.

The current shared-argument/mutable-parameter branch checks only whether the expression is a
place. An immutable existing place without `~` therefore reaches the same marker-only repair as an
already-mutable place. The validator must classify place mutability before choosing the reason.

**Additional confirmed fresh-value gap:** `copy source` currently enters the existing-place branch
and receives missing-`~` guidance. `copy` creates an independent value, so a plain copied value is a
valid fresh argument for a mutable parameter. The call-boundary classifier must treat
`ExpressionKind::Copy` as `FreshMutableValue`, while authored `~copy source` remains invalid because
`~` accepts only an existing mutable place.

**Additional confirmed rendering gaps:** the shared unnamed-parameter fallback currently exposes
the internal zero-based slot before translating it to a one-based position. It must render only the
one-based source position. The authored-`~` non-place branch also says `non-place expression` and
`variable`, even though a plain fresh value is valid for the mutable parameter. Render this as an
invalid marker on a fresh or computed value and tell the author to remove `~`. Generic immutable
fallbacks must describe an immutable binding or field without compiler-facing place terminology.

The common Rust-style `&` syntax-mistake guidance has the same terminology drift: it says shared
borrows are automatic and tells the author to prefix a place. Add a final narrow Phase 3.1 wording
slice that says existing values use shared access automatically, and that `~` is valid only on an
existing mutable binding or field at a mutable call or receiver boundary.

#### Implementation

Replace broad reasons with explicit call and receiver facts. The exact enum layout may remain split between `InvalidCallShapeReason` and `InvalidReceiverCallReason`, but both paths must use the same access classification from `value_mode` and place analysis.

Carry:

- callee or method name
- parameter name/index where applicable
- receiver or argument place name when it is a simple source place
- whether the place is mutable
- whether the marker was authored
- source location of the place and marker

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

Show a concrete receiver call only when the payload owns the simple receiver name.

Fallible collection and map examples must include `!` or `catch` when a complete example is shown.

Const-record receiver diagnostics now use `ConstRecordNoRuntimeCalls` without an alias and render
the current source term `const record`.

#### Required AST correction

`receiver_access.rs` no longer merges non-place and immutable-place receivers. A temporary mutating
receiver and an immutable named receiver use distinct reasons and repairs.

#### Tests

Use one matrix covering user functions, source receiver methods, collection builtins and map builtins. Include positive fresh-rvalue calls, including `copy source`, to prevent the change from requiring illegal `~` on fresh values.

### 3.2 Correct immutable assignment and field-write guidance

**Original finding:** DIAG-007
**Rejected original proposal:** `~p.x = 10` is not Moth assignment syntax.

**Phase 3.2a status:** Complete in this checkpoint. Reason taxonomy, message wording and field-write distinction are done. The secondary declaration label (scope-frame binding location) is Phase 3.2b.

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

#### Implementation

- Keep assignment syntax marker-free.
- When the target is a field, carry the field name and mutable root binding where available.
- Render:

  > Cannot assign to field `x` because root binding `p` is immutable. Declare `p` as mutable before this assignment.

- A secondary label should point to the immutable declaration.
- Body-local lookup currently retains only `Declaration`, whose expression location identifies the
  initializer rather than the binding target. Preserve the authored binding location in the
  scope-frame's local declaration entry and expose it through `ScopeDeclarationRef`. Do not add a
  location field to the shared `Declaration` type solely for this diagnostic.
- Keep a generic immutable-place fallback for projections whose root cannot be named cleanly.
- Rename the temporary-assignment fallback away from `NotMutablePlace` and remove `place` and
  `rvalue` from its renderer. A temporary value cannot be assigned through: receive it in a mutable
  binding first, then assign through that binding.
- Replace the vague direct-binding `ImmutableVariable` message in the same assignment-target
  family. It currently says only to use `~`, which can be mistaken for assignment-target syntax or
  an illegal redeclaration. Carry the original binding location and render without assuming whether
  the declaration inferred or explicitly named its type:

  > Cannot reassign `value` because its binding is immutable. Make the original binding mutable, then reassign it with ordinary `=`.

- Do not fabricate a replacement declaration without the original initializer, authored type and
  binding-mode facts. Focused labels may still identify the original declaration.
- In the same assignment family, say `binding` rather than `variable` when an assignment operator
  is missing after a resolved source binding.
- Strengthen the existing direct immutable reassignment and immutable struct-field integration
  cases so the current declaration and assignment guidance is contractual. Remove fixture
  comments that call an ordinary immutable runtime binding a constant.

### 3.3 Diagnose `~` on assignment targets accurately

**Original finding:** DIAG-021
**Rejected original proposal:** `x ~= 2` is a declaration form, not ordinary reassignment.

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

Add a dedicated assignment-target reason. Do not reuse `MutableMarkerOnNonReceiverCall`, whose message is call-specific.
Do not universally tell the author to redeclare the binding: the marked target may already be
mutable. Phase 3.2 owns the separate diagnostic for an ordinary assignment whose root binding is
immutable.

### 3.4 Diagnose `copy ~place` as an unnecessary access marker

**Original finding:** DIAG-020

`copy` accepts an existing place and creates independent value semantics. It does not take mutable-access syntax.

Add `InvalidCopyTargetReason::MutableMarkerNotAllowed`:

> `copy` does not take `~`. Copy the existing binding or field projection without the access marker.

Use this reason only when the operand after `~` is otherwise a valid binding or field-projection
copy target. A marked literal, call or computed expression still needs the factual non-place
diagnostic because removing `~` would not make it copyable.

Keep the internal place classification, but make every rendered correction source-visible:

- `NonPlace` for literals and computed expressions:

  > `copy` requires an existing binding or field projection. Bind this value first, then copy that binding.

- Replace `FunctionValue`, which is inaccurate because function values aren't part of the current
  surface, with separate function-name and function-call facts when the following `(` makes the
  distinction available.
- A function name isn't a copyable value. A call should explain that its returned value must be
  received by a binding before that binding can be copied.
- Do not use compiler-facing `place` terminology or call source bindings variables in these
  messages.

Add focused coverage for a literal, computed expression, function name, function call and
`copy ~binding`. Keep one integration owner where the stable code and wording are user-visible.

### 3.5 Improve borrow-conflict explanations without inventing lifetimes

**Original finding:** DIAG-056

Borrow diagnostics should explain source ordering and aliases, not tell users to manually end a borrow or mention internal analysis state.

#### Implementation

- Preserve the conflicting place and add its originating access location where known.
- `StatementAccessTracker` currently retains only `AccessKind` per root even though both record
  sites own the current source location. Retain the first conflicting location with the access
  kind so same-statement conflicts can label the earlier access without changing transfer rules.
- Add a secondary label at the access that remains live.
- Delete the stale `ExistingBorrow` label variant and its `existing borrow here` rendering. These
  diagnostics describe an earlier conflicting source access, so use one source-visible
  `ConflictingAccess` label such as `earlier conflicting access here` for both shared/mutable and
  duplicate-mutable conflicts.
- Keep the stable borrow diagnostic codes, but make their rendered descriptor titles use the same
  source-visible access vocabulary. `Borrow conflict`, `Multiple mutable borrows`, `Move while
  borrowed` and `Whole-object borrow conflict` currently expose analysis terminology before the
  payload message is rendered. Prefer `Access conflict`, `Conflicting mutable access`, `Ownership
  transfer conflicts with active access` and `Whole-value access conflict` respectively.
- Render by access pair.

Mutable alias blocks shared read:

> Cannot read `data` while mutable alias `first` is still needed later. Read through `first`, or move the later use of `first` before this access.

Shared alias blocks mutation:

> Cannot mutably access `data` while shared alias `shared` is still needed later. Move the mutation after the last use of `shared`, or create an explicit copy when independent data is required.

Second mutable access:

> Cannot create another mutable access to `data` while `first` is active. Reuse `first`, or move the new access after its last use.

- Keep generic fallbacks when a conflicting source place or location is unavailable.
- Do not describe `~` as a move.
- Do not expose exact lifetimes or ownership flags.

#### Tests

Add passing counterparts that resolve the conflict by reordering the last use, reusing the active alias, narrowing a scope or using explicit `copy`.

### 3.6 Remove migration state from collection and map assignment diagnostics

**Additional audit finding:** `InvalidAssignmentTargetReason::{CollectionIndexedWriteRemoved,
MapIndexedWriteRemoved, MapPropertyWriteRemoved}` and the
`collection_indexed_write_removed` fixture encode implementation history instead of the current
source rule. These are ordinary invalid assignment targets, not compatibility diagnostics.

- Rename the reasons to factual current states such as `CollectionGetTargetNotWritable`,
  `MapGetTargetNotWritable` and `ReadOnlyMapProperty`.
- Render collection access as:

  > Cannot assign through collection `get(...)`. Call `set(index, value)` with explicit `~` access on the collection instead, then recover with `catch` or propagate with `!` inside a compatible `Error!` function.

- Render map access with the equivalent `set(key, value)` and explicit `~` access guidance. Do
  not fabricate a receiver name that the diagnostic payload does not own. Keep the correction valid
  at top level, where postfix `!` cannot propagate because there is no `Error!` return slot.
- Keep the read-only `length` message, but remove migration wording from its reason name.
- Rename the integration fixture to describe current rejection rather than removal.
- Preserve the existing stable diagnostic code and valid `set` coverage. Do not add a parser path
  that recognises an older assignment feature.

## Phase 4: Error, option and value-flow diagnostics

### 4.1 Replace the umbrella `NotResultExpression` path

**Original findings:** DIAG-025, DIAG-043, DIAG-044

**Additional confirmed terminology gap:** incompatible custom error propagation still uses
`TypeMismatchContext::ResultError`, which renders `Type mismatch in result error`. A direct
`FirstError!` through `SecondError!` probe reaches this path. The source contract calls this the
function's error return or error slot, never a Result error.

**Additional confirmed descriptor gap:** the stable `MOTH-RULE-0051` title still renders
`Invalid result handling` even when its message correctly names an unhandled `Error!` call. Keep
the code but change the user-facing descriptor title to `Invalid fallible handling` when the
operand matrix lands.

**Additional confirmed operand-family gap:** `MOTH-TYPE-0004` still renders the descriptor title
`Invalid result operand`, and its internal kind, payload and reason family repeat the same stale
term even though the only production path is an unhandled fallible operand. The
`OptionalValueNotInspected` reason has no compiler production site and exists only in renderer/model
tests. Rename the diagnostic family outright to fallible-operand terminology, render a title such
as `Unhandled fallible operand` and delete the dead optional reason rather than preserving an
unused parallel path. Keep `MOTH-TYPE-0004`.

**Additional confirmed propagation-shape gap:** postfix `?` requires exactly one optional success
return slot. The current validator collapses top-level code, zero-return functions, non-optional
functions and functions with multiple success slots into `FunctionHasNoOptionalReturn`. Add a
distinct multiple-success return-shape reason carrying the authored success-slot count. Preserve a
narrow enclosing-boundary fact in `ScopeContext` so `?` and `!` inside nested branches can still
distinguish a real function from top-level module work without guessing from the immediate
`ContextKind`.

**Additional confirmed terminology gap:** `InvalidBuiltinCallReason::MustHandleFallibleResult`
still names an internal Result despite rendering a fallible call, and cast diagnostics say
`Error! result` and `result slot`. Rename the builtin reason to `UnhandledFallibleCall` without an
alias. Tell authors to handle the fallible operand before casting and call multi-value outputs
`success return slots`.

The current `InvalidResultHandlingReason::NotResultExpression` hardcodes postfix `!` wording and calls the source carrier a Result even when the authored construct is `catch` or the operand is optional.

#### Implementation

Replace the umbrella reason with explicit source cases:

```rust
enum InvalidFallibleHandlingReason {
    CatchOnNonFallible,
    CatchOnOptional,
    BangOnNonFallible,
    BangOnOptional,
    QuestionOnNonOptional,
    QuestionOnFallible,
    ErrorPropagationAtTopLevel,
    ErrorPropagationInNonFallibleFunction,
    OptionPropagationAtTopLevel,
    OptionPropagationInNonOptionalFunction,
    OptionPropagationInMultipleSuccessFunction { success_count: usize },
    // retain other structurally distinct catch and propagation cases
}
```

Names may differ, but each branch must encode the authored handler, operand carrier and propagation boundary.

Delete `RemovedBangFallbackSyntax` and `RemovedBangCatchHandlerSyntax`, their dedicated parser
recognisers, unit tests and `result_removed_err_bang_syntax_rejected` fixture. Those paths exist
only to identify discontinued handling syntax. The ordinary current grammar may reject the token
sequence without a migration-specific reason. Do not carry either reason into the replacement
matrix.

Rename `TypeMismatchContext::ResultError` to `ErrorReturn` and render `error return`. Apply the
same source-visible context to incompatible postfix `!` call propagation and `cast!` propagation.
Keep the expected and found error types as semantic `TypeId`s and preserve `MOTH-TYPE-0001`.

Render the `MOTH-RULE-0051` descriptor title as `Invalid fallible handling`. Internal reason-family
identifiers must also change coherently in this slice: rename `InvalidResultHandlingReason`, its
payload, constructor and kind to fallible-handling terminology without aliases or compatibility
wrappers. AST expression and HIR type names outside the diagnostic family are not part of this
mechanical rename. No renderer title or message may call the source feature result handling.

#### Messages

- `catch` on plain value:

  > `catch` handles fallible `Error!` expressions, but this expression is not fallible.

- `catch` on optional:

  > `catch` does not recover an optional value. Inspect the optional value with an `if ... is |present| ... else ...` expression.

- `!` on optional:

  > Postfix `!` propagates an `Error!` return, but this expression is optional. Use postfix `?` only inside a compatible optional-returning function, or inspect the option explicitly.

- `?` on fallible call:

  > Postfix `?` propagates absence from an optional value, but this call can return `Error!`. Recover with `catch`, or use `!` only inside a compatible `Error!` function.

- top-level `!`:

  > Top-level code has no `Error!` return slot, so `!` cannot propagate here. Recover with `catch`, or call this from a function that returns `Error!`.

- `!` in a real non-fallible function:

  > This function does not declare an `Error!` return slot. Add one or recover locally with `catch`.

Keep the stable `MOTH-RULE-0051` family unless a branch belongs to an existing more precise code.

#### Reclassify invalid public carrier type spellings

**Additional confirmed design mismatch:** the canonical generic type-application contract states
that `Option of T` and `Result of T, E` are not Moth language syntax. The compiler instead
routes both through `DeferredFeatureReason` and `MOTH-DEFERRED-0001`, which falsely reserves a future
public carrier syntax. First-class public result values are outside language design scope, while
optional types use the current `T?` suffix.

- Delete `DeferredFeatureReason::{PublicOptionTypeSyntax, PublicResultTypeSyntax}` and the helper
  that classifies these authored forms as deferred.
- Route the two generic-position spellings through factual structured
  `InvalidGenericInstantiationReason` variants and existing `MOTH-RULE-0057` ownership.
- Render:
  - `` `Option of T` is not Moth type syntax. Use the `T?` optional suffix. ``
  - `` `Result of T, E` is not Moth type syntax. Fallible functions declare a final `E!`
    error return slot. ``
- Rename the `_deferred` unit tests and integration fixtures around current rejection.
- Delete the old reason names and deferred prose without aliases, reserved-future comments or a
  compatibility diagnostic path.
- Preserve ordinary user-defined generic type application and canonical `T?` / `E!` coverage.
- Do not edit the generic type-application documentation. It already states the accepted design.

### 4.2 Add catch-recovery type context

**Original finding:** DIAG-026

Add `TypeMismatchContext::CatchRecovery`.

The shared produced-value checker already carries `ValueReceiverKind::CatchHandler`, but
`mismatch_context_for_receiver` currently collapses it into `TypeMismatchContext::General`.
Change that one mapping and update the existing fallible-handling unit test that currently
asserts `General`. Keep declaration, assignment, return, multi-bind and nested-`then` contexts
unchanged.

Message:

> Type mismatch in catch recovery: expected `Int`, found `String`.

Guidance may explain that each `then` value must match the corresponding success slot. Keep expected and found types as `TypeId`s.

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

### 4.4 Reject multi-return values received by one target

**Original finding:** DIAG-031

A direct probe confirms `value = pair()` currently compiles when `pair` returns two success
slots. Call expressions retain those slots in `ExpressionKind::*Call::result_type_ids`, but their
general `Expression::type_id` becomes an internal tuple. Declaration, assignment, return and
produced-`then` receivers must check the typed slot count before coercion or lowering can treat that
tuple as one source value.

Do not silently discard success slots.

#### Implementation

Add a dedicated value-receiver diagnostic family rather than misusing `InvalidMultiBind`, because the invalid source has no multi-bind.

Carry:

- receiver kind: declaration, assignment or return
- target count
- produced slot count
- call or value-block location

Message:

> This expression produces 2 values, but the declaration has 1 target. Use one target per return slot with a multi-bind declaration.

Do not suggest `_` or another discard syntax. None is part of the current language.
Do not fabricate target or callee names that the diagnostic payload does not own.

Tests should cover declaration, assignment and nested value-producing block receivers if those paths are distinct.
Keep `left, right = pair()` as positive multi-bind coverage. This diagnostic slice does not add
tuple values, implicit slot discards or multi-return forwarding.

Implement this as two reviewable slices: introduce the diagnostic and protect declaration plus
assignment receivers first, then extend the accepted family to return and nested `then` receivers.

### 4.5 Use the same `then` diagnostic at top level and in functions

**Original finding:** DIAG-032

Current routing already uses `ThenWithNoActiveValueTarget` in both module and function body
dispatch. The remaining change is source-visible wording and ownership cleanup: the existing
`result_then_outside_catch_rejected` fixture and `rejects_then_outside_catch_block` test name still
describe `then` as catch-specific even though value-producing `if`, full match and catch all own it.

Route top-level `then` through the existing structured `ThenWithNoActiveValueTarget` reason:

> `then` is only valid inside a value-producing `if`, full match or `catch` recovery that has an active receiving site.

Do not leave top-level parsing on generic `UnexpectedToken`.

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
`reject_removed_in_loop_syntax` and their migration-only unit test exist only to recognise the
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

Extend `ExtraPositionalArgument` with `provided_count` and the extra argument location.

Message:

> Call to `add` has 3 positional arguments, but accepts 2.

Guidance:

> Remove the extra argument starting here.

Do not suggest named arguments. Renaming arguments does not fix excess arity.

Cover user functions, struct constructors and choice constructors where they share or intentionally differ in call-shape ownership.

### 5.5 Enrich choice match-pattern diagnostics

**Original findings:** DIAG-049, DIAG-050, DIAG-051

#### Unknown variant

Carry:

- source-visible choice name
- requested variant
- available variants

Message:

> Unknown variant `Color::Gren`. Did you mean `Color::Green`? Available variants: `Red`, `Green`, `Blue`.

Do not offer a close-name suggestion when the threshold is not met.

#### Qualifier mismatch

Carry the authored qualifier and the scrutinee's source-visible choice name:

> Match arm uses qualifier `Size`, but the scrutinee is `Color`. Use a `Color` variant or omit the qualifier.

Dependency aliases must render using the visible source name where possible.

#### Payload captures

Carry expected field names, provided capture names and expected/found counts.

Count mismatches must point at the authored pattern's capture-list boundary. The current too-few
path uses the choice variant declaration location, which identifies the related declaration rather
than the mistake. Keep the declaration as secondary context when useful, but make the closing `)`
or first extra capture the primary location.

Messages:

- name mismatch:

  > Capture `missing` does not match payload field `value` in variant `Ok`. Use `Ok(value)` or rename it with `Ok(value as missing)`.

- count mismatch:

  > Variant `Ok` has 1 payload field, `value`, but this pattern captures 0.

Keep exact field lists bounded and deterministic.

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

### 5.6 Make scalar type-call diagnostics factual

**Additional audit finding:** `InvalidBuiltinCallReason::ScalarConstructorRemoved`, several
`*_constructor_removed` fixtures and the dead `InvalidCastReason::ScalarConstructorRemoved`
encode history rather than the authored mistake. Unlike a discontinued compatibility-only parser
path, the current expression parser must still handle a builtin type token followed by `(`.

- Replace the builtin-call reason with a factual type-as-call reason such as
  `ScalarTypeCalledAsFunction`.
- Render:

  > `Int` is a type, not a conversion function. Use `cast` at an explicit typed boundary.

- Carry the exact builtin type name already available at the emission site.
- Remove the unused `InvalidCastReason::ScalarConstructorRemoved` variant and render branch.
- Rename the focused fixtures and tests around current rejection, then remove redundant
  per-scalar coverage where one table test or one primary integration case protects the same
  parser contract.
- Preserve positive `cast` coverage. Do not recognise a former scalar-constructor language
  feature or describe anything as removed.

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
    location: SourceLocation,
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

Diagnostic payload:

- earlier item name and location
- incoming item name and location
- structured conflict reason

Examples:

> `$md` and `$raw` cannot be combined because both control template body formatting. Choose one formatter.

> `$note` cannot be combined with `$children` because a discarded comment directive must be the template's only meaningful head item.

- Primary label: incoming item
- Secondary label: earlier conflicting item
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

Track the opening `[` location in the template-mode stack.

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

**Additional confirmed gap:** ordinary body-local typed and mutable redeclarations still use the
parallel `ShadowedName` family and `MOTH-RULE-0038`. That path duplicates `DuplicateDeclaration`,
labels the first initializer rather than the authored binding and leaves the scope-neutral message
from Phase 2.5 unused for normal local bindings.

**Additional confirmed dispatch gap:** the existing-symbol branch recognises only builtin type
tokens and the mutable marker as declaration starts. A redeclaration with a nominal, alias,
generic, qualified or collection annotation, the current postfix compile-time marker or a reactive
marker can fall through to expression parsing. Declaration-shape detection must reuse the canonical
binding-target parser rather than grow another token whitelist. Plain `name = value` remains
reassignment, not a second inferred declaration.

A second reactive declaration is not invalid because `$` is unexpected. It is invalid because the visible name already exists.

- Delete `RuleDiagnosticKind::ShadowedName`, its descriptor, payload, constructor, remapping and
  renderer branch.
- Route ordinary body-local typed and mutable redeclarations through `DuplicateDeclaration`.
- Parse every unambiguous current body-local binding declaration shape before duplicate
  registration rejects it, including user-defined/constructed type annotations and postfix
  compile-time or reactive binding markers. Keep plain `name = value` on reassignment.
- Reuse `DuplicateDeclaration`.
- Reuse the scope-neutral duplicate-name message established in Phase 2.5. Do not reintroduce a
  reactive-specific renderer branch.
- Use the authored binding location preserved by Phase 3.2 and label both declarations.
- Update the existing `MOTH-RULE-0038` fixtures to `MOTH-RULE-0002`. Do not reserve or retain the
  superseded code through an alias.
- Do not invent a separate reactive uniqueness rule.

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

The exact operator payload and the `not`, string concatenation and generic-parameter messages now belong to Phase 1.2a. The generic-bound terminology fix already requires exact operator facts, so retaining a later category-only-to-exact migration would create transitional API and duplicate renderer work.

### 7.3 Add actionable collection-loop guidance

**Original finding:** DIAG-055

Keep the found semantic `TypeId`.

Message:

> Collection loop source must be a collection, found `Int`. Use a collection after `loop`. For numeric iteration, use range syntax such as `loop 0 to 10 |i|:`.

Do not imply every non-collection source was intended as a range.

Delete `InvalidLoopHeaderReason::RemovedInSyntax`, the dedicated `reject_removed_in_loop_syntax`
pre-scan and its migration-only unit test. Current collection and range loop forms already have
their own structured diagnostics. A discontinued `loop <binder> in ...` token sequence may fail
through the ordinary current grammar without a compatibility message.

### 7.4 Reject unsupported Wasm variant payloads before lowering

**Additional audit finding:** `cast_optional_success_wraps_inner_value` currently expects `MOTH-INFRA-0001` because a reachable optional payload reaches Wasm LIR lowering, where `HirExpressionKind::VariantConstruct` with fields is rejected as a transformation error. This is valid Moth source using a target feature that HTML-Wasm does not yet lower. The existing pre-lowering `UnsupportedBackendFeature` lane owns this failure.

#### Implementation

- Extend Wasm backend feature validation to detect reachable variant constructions with payload fields before LIR lowering.
- Emit the existing `MOTH-RULE-0064` `UnsupportedBackendFeature` diagnostic at the source expression. Name the unsupported feature as variant payload values without exposing HIR or LIR.
- Preserve reachability policy. An unsupported variant payload in an unreachable helper must not block the selected target.
- Keep empty/unit variant construction on the supported path.
- Do not convert the Wasm lowering invariant itself into a user diagnostic. Backend feature validation must prevent valid reachable source from reaching that invariant.

#### Tests

- Update `cast_optional_success_wraps_inner_value` to expect `MOTH-RULE-0064` for HTML-Wasm while preserving HTML success.
- Add a focused backend-feature validation test for reachable and unreachable payload construction if the integration case cannot protect both reachability branches.

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
| DIAG-006 | Removed | The snippet was not a const-required cast and bare `cast` with fallible evidence must first be handled with `cast!` or `catch`. The proposed literal-failure branch conflated evidence selection with const failure. |
| DIAG-007 | Retained, reversed | The binding does need to be mutable. Field assignment is `p.x = ...`, not `~p.x = ...`. |
| DIAG-017 | Removed as stale | Current rendering identifies `|`, not `/`. A broader parser-context change is not justified without a current reproducible failure. |
| DIAG-019 | Removed as ambiguous | `MyList {Int}` is a parseable typed declaration missing an initializer. The compiler cannot know it was intended as a type alias. |
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
4. Phase 4 fallible/optional handling and value receivers
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
- multi-return receiving
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
- diagnostic payload remapping for every new `StringId` or path field
- descriptor-code uniqueness after adding and removing kinds

### Regression requirements

- No negative fixture should pass only because a later unrelated diagnostic fires.
- Positive counterparts must protect the intended valid Moth pattern.
- Do not use benchmark fixtures as diagnostic coverage.
- Remove fixtures tied only to deleted legacy paths.

## Documentation exclusion

Do not update general design docs, language docs, the progress matrix or generated documentation
as part of this plan. A narrow documentation edit is allowed only when the file explicitly names
the exact diagnostic being changed and would otherwise become inaccurate. The active roadmap plan
continues to record slice state and confirmed implementation findings.

## Final validation

Run targeted tests during each phase. Before completion:

```text
cargo fmt
just validate
```

Also perform the manual diagnostic audit required by the compiler style guide:

- every user mistake uses `CompilerDiagnostic`
- every new payload carries structured facts
- every source location points to the actual mistake
- no stale or legacy message path remains
- no suggestion teaches invalid syntax
- no duplicate diagnostic owner remains across tokenizer, headers, AST, HIR or borrow validation
- no type decision compares rendered syntax instead of `TypeId`
- no borrow diagnostic exposes compiler-internal lifetime or ownership machinery

## Completion criteria

- All retained cases have end-to-end coverage.
- All removed findings are absent from the implementation backlog.
- Quoted strings implement exactly the confirmed escape set.
- Raw strings preserve literal backslashes and newlines.
- No deleted syntax has a dedicated compatibility surface.
- Every incompatible directive diagnostic names both items and explains the conflict.
- Mutable diagnostics distinguish missing `~`, immutable places, fresh values and invalid assignment markers.
- Fallible and optional diagnostics name the authored operator and correct Moth carrier.
- Terse diagnostics never have an empty message field.
- Malformed user source in this plan never produces `MOTH-INFRA-0001`.
- `just validate` passes.
