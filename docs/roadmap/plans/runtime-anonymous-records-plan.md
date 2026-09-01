# Runtime anonymous records implementation plan

## Purpose

Enable `|...|` in a runtime receiving context as a local hidden nominal struct after numeric semantics are stable. Reuse ordinary struct construction, field access, copy, borrow and lifetime owners. Do not add structural typing, a second record grammar or a second record IR.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/runtime-anonymous-records-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - complete a read-only runtime-record ownership and escape review
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: AST expressions, hidden nominal structs, HIR struct lowering, borrow and lifetime validation
```

Keep this block concise. Git history is the implementation record.

## Roadmap position and prerequisites

This plan runs after number/numeric semantics and before the HTML mixed JavaScript/Wasm backend.

Hard prerequisites:

- anonymous compile-time `|...|` records: one context-specific `name = value` parser, nested-list rejection, a shared compile-time marker `TypeId`, and folded public values
- canonical `TypeEnvironment` nominal identity and ordinary struct field lookup
- validated HIR struct construction and field access
- borrow validation and lifetime-region/escape analysis for ordinary structs
- public-surface validation that already names runtime anonymous-record types as prohibited exports
- `Number` / `NumberN` identity so hidden field types are the post-numeric types

Name those delivered capabilities. Do not cite a retired plan file.

## Required authorities

- `docs/compiler-design-overview.md` for public-surface escape, TypeId ownership, const-record folding and HIR struct lowering
- `docs/src/developer-docs/language/overview.mtf` plus `docs/src/docs/constants/const-records.mtf`, `docs/src/docs/structs/struct-declarations.mtf` and `docs/src/docs/structs/construction-and-fields.mtf`
- memory-management design under `docs/src/developer-docs/memory-management/`
- style, testing and validation guides
- `docs/src/docs/progress/@page.moth`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` as orientation only

## Current compiler state

Do not add a second `|...|` grammar.

Already landed:

- `|...|` is parameters syntax. Context decides what the list becomes: a compile-time receiving context (`#=`) is an anonymous const record; a named type shell `Name = | field Type |` is a struct, including empty `Name = | |`; a runtime receiving context currently reports `DeferredFeatureReason::RuntimeAnonymousRecord` for `| name = value |`
- `parse_anonymous_const_record_expression` owns `name = value` fields. The expression dispatcher refuses the runtime arm before that parser runs
- nested `|...|` is `InvalidExpressionReason::NestedAnonymousConstRecord`
- the generic token scanner balances `|...|` regions without deciding struct versus record. `pipe_opens_value_record` in the AST owner uses binding mode: empty `#= | |` is a const record; empty ordinary `= | |` is a struct shell
- `TypeDefinition::AnonymousConstRecordMarker` is one module-local compile-time `TypeId`. It is fieldless, public-surface-legal and never a runtime type. Semantic fields live on folded values
- `ExpressionKind::AnonymousConstRecord` exists so lowering never treats a const record as `StructInstance`. HIR projects const-record fields only through the folded store
- fixture `anonymous_const_record_runtime_rejected` is `stats = | count = 1 |` inside a function and expects `MOTH-DEFERRED-0001`

This plan flips the runtime arm from deferred to accepted and registers a hidden nominal struct. It does not reuse the const-record marker, AST variant, public folded-value path or HIR special case.

Grouped project config may still compose compile-time records under its own schema contract. That work is out of this plan.

## Accepted source model

```moth
start:
    point = |
        x = 10,
        y = 20,
    |

    io.line(point.x)
;
```

Rules:

- Functions, structs, const records and runtime anonymous records share `|...|` parameters syntax. Context decides the meaning.
- A runtime receiving context (`=`, mutable `~=` or another runtime receiver) turns the list into a runtime value.
- Every parameter has a value. Types are inferred from those values. The list does not declare a source-visible constructable type.
- Field values are ordinary runtime expressions. They do not have to fold. That is the opposite of a const record.
- A parameter list is one `|...|` group. It does not contain another `|...|` list. Declare an inner record as its own local, then name that binding as a field value.
- Runtime anonymous records require at least one named field. Empty `| |` in an ordinary `=` binding is a named struct shell. Empty `#= | |` remains an empty anonymous const record.
- Each literal site creates one hidden nominal `TypeId`. Two sites with identical fields remain different types.
- Reassignment is compatible only with that same site identity, not with another literal of the same shape.
- Runtime anonymous records support local binding, shared access, explicit `copy` and field projection through ordinary struct semantics.
- They have no source-visible type name, constructor name, receiver methods or conformance.
- They cannot escape through public interfaces.
- They cannot appear in function signatures, returns, aliases, struct/choice fields, trait evidence or exported constants. Passing one as a function argument is an escape: the parameter type cannot name the hidden identity.
- The first implementation also rejects collection/map storage and generic instantiation involving runtime anonymous records.
- After type assignment, AST and HIR see ordinary nominal struct construction and field access. No anonymous-specific HIR node survives AST.
- Borrow and lifetime rules are exactly those of the lowered hidden nominal struct.

## Split from const records

Keep these owners separate. Mixing them is a stop condition.

- Compile-time `#=` produces `AnonymousConstRecord` and the shared marker `TypeId`. The complete value is not a runtime object and may export as folded fields.
- Runtime `=` produces a per-site hidden `Struct` `TypeId`. The complete value is an ordinary runtime object and must not export.
- Const-record fields live on folded values. Runtime-record fields live on a `TypeEnvironment` struct definition so `fields_for` works.
- Const records needed an HIR special case because they are not structs. Runtime records must not.

## Data ownership

- AST assigns a hidden nominal `TypeId` from stable source-site identity. Phase 0 decides the exact key. Shape is not the key.
- `TypeEnvironment` owns ordered fields through `StructTypeDefinition` and ordinary field lookup.
- Diagnostic spelling identifies the anonymous record and source site without becoming semantic identity. It must not render as `anonymous const record`.
- HIR consumes the resolved `TypeId` and ordinary struct fields.
- Public-interface projection rejects the hidden identity rather than canonicalising it. Do not follow the const-record marker path, which public-surface validation currently allows.

## Non-goals

- no structural typing or shape unification
- no named type extraction or inference across literal sites
- no anonymous-record methods or conformance
- no public or cross-module anonymous type
- no anonymous record in generic arguments, collections or maps in the first implementation
- no nested `|...|` lists
- no pattern matching or destructuring syntax
- no backend-specific representation
- no changes to const-record marker, folding, export or `AnonymousConstRecord` AST/HIR
- no treating a complete const record as a runtime value

## Implementation phases

### Phase 0: Mandatory architecture review

Before code:

- inventory expression delimiter ownership, `pipe_opens_value_record`, the runtime deferred arm, hidden nominal registration, ordinary struct construction, field access, HIR lowering of ordinary structs, copy semantics, borrow validation and lifetime escape checks
- decide the exact stable source-site key for hidden identities
- treat empty runtime `| |` as already decided: not a runtime record
- enumerate every prohibited escape boundary, including private function signatures
- decide whether to share one `name = value` field-list parser and branch on receiving context, or keep the const parser and add a thin runtime entry that produces `StructInstance`
- prove ordinary struct HIR can represent the feature without a second runtime record IR and without `AnonymousConstRecord`

Stop if the implementation needs structural type comparison, donor-local identity export, `AnonymousConstRecordMarker` reuse or anonymous-specific backend nodes.

### Phase 1: Accept runtime record literals

- Parse `| field = value, ... |` in a runtime receiving context through the existing `name = value` grammar.
- Keep parameter, struct, choice, receiver and config grammar on existing paths.
- Reject duplicate fields, nested `|...|`, malformed separators, missing values and unterminated literals with structured diagnostics.
- Retarget `anonymous_const_record_runtime_rejected`. Accepted runtime syntax must not keep `MOTH-DEFERRED-0001`.
- Leave compile-time `#=` on `AnonymousConstRecord`.

### Phase 2: Register hidden nominal types

- Create one hidden `StructTypeDefinition` per literal site.
- Register ordered fields after their value types resolve.
- Use a transient name index for duplicate detection and field lookup construction.
- Do not unify or intern by shape.
- Do not intern through `AnonymousConstRecordMarker`.
- Add readable diagnostics for mismatched anonymous sites.

Review gate: verify identity is source-site-based and cannot cross a public boundary.

### Phase 3: Enforce the local-only surface

- Allow local declarations, assignments compatible with the same site identity, copy and field reads or writes through ordinary struct rules.
- Reject returns, explicit signature use, aliases, receiver methods, conformances, exported surfaces, aggregate storage and generic requests.
- Report the error at the first escaping use rather than during backend lowering.

### Phase 4: Lower through ordinary struct HIR

- Reuse ordinary nominal struct construction and field projection.
- Preserve the hidden `TypeId` in the module-local type environment.
- Add no anonymous-specific HIR expression or statement variant.
- Do not lower a runtime-typed record as `AnonymousConstRecord`.
- Validate HIR through existing nominal member checks.

### Phase 5: Borrow, lifetime and copy validation

- Reuse ordinary struct root/projection alias rules.
- Validate mutable field access through the existing place model.
- Ensure deep `copy` handles the record graph exactly as an ordinary struct.
- Verify hidden records cannot escape module/public lifetime summaries.

Review gate: run a read-only memory and HIR audit before backend-facing acceptance.

### Phase 6: Backend parity, tests and docs

- Confirm JS and Wasm lowerers consume only ordinary struct HIR/layout facts.
- Add integration cases for construction, projection, copy, mutability, two-site mismatch, nested-list rejection, empty `| |`, `#=` remaining const-only and every escape rejection.
- Update the cheatsheet deferred runtime-record section, the progress matrix and the compiler-design sentence that still calls runtime records deferred.
- If a canonical language leaf is required, add it under structs. Do not fold runtime records into `const-records.mtf`.
- Rebuild generated docs.

## Stop conditions

Pause when:

- a second anonymous runtime representation appears necessary
- field lookup requires repeated shape scans
- hidden types enter public canonical identities
- the const-record marker, folded-value export path or `AnonymousConstRecord` HIR arm is reused for runtime values
- generic or collection support expands the phase unexpectedly
- borrow/lifetime rules differ from ordinary structs
- a backend needs source-level anonymous-record knowledge

## Validation

Every code-bearing phase runs:

```bash
cargo fmt
just validate
```

Run the documentation release build when source docs change.

## Final audit

Before completion, verify:

- each literal site owns one hidden nominal identity
- no structural equality exists
- no anonymous-specific HIR/backend path exists
- runtime-typed records are not `AnonymousConstRecord` and do not use the const-record marker
- every public and unsupported escape is diagnosed before HIR/backend handoff
- nested `|...|` remains rejected
- ordinary struct borrow, lifetime and copy owners are reused
- compile-time `#=` records keep field-access-only const-record semantics
