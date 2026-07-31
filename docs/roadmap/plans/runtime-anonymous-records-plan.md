# Runtime anonymous records implementation plan

## Purpose

Add narrowly scoped runtime anonymous record literals after anonymous const records are stable, reusing nominal struct lowering, borrow validation and lifetime analysis without introducing structural typing.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/runtime-anonymous-records-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - complete a read-only runtime-record ownership and escape review
REVIEW_BASELINE: 47dbf3fd1dfa3e8df3d02cef05001de695ea80ee
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: AST expressions, hidden nominal types, HIR struct lowering, borrow and lifetime validation
```

Keep this block concise. Git history is the implementation record.

## Roadmap position and prerequisites

This plan runs after the number/numeric plan and before the HTML mixed JavaScript/Wasm backend plan.

Hard prerequisites:

- `docs/roadmap/plans/anonymous-const-records-plan.md`
- canonical `TypeEnvironment` nominal identity
- validated HIR struct construction and field access
- borrow validation and lifetime-region/escape analysis for ordinary structs
- stable public-surface escape diagnostics

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/language-overview.md`
- memory-management design under `docs/src/docs/codebase/memory-management/`
- style, testing and validation guides
- progress matrix

## Accepted source model

```moth
point = |
    x = 10,
    y = 20,
|

x = point.x
```

Rules:

- Every runtime literal site creates one hidden nominal type.
- Two sites with identical fields remain different types.
- Field order and field types are fixed by the literal site.
- Runtime anonymous records support local binding, shared access, explicit copy and field projection through ordinary struct semantics.
- They have no source-visible type name, constructor name, receiver methods or conformance.
- They cannot escape through public interfaces.
- They cannot appear in function signatures, returns, aliases, struct/choice fields, trait evidence or exported constants.
- The first implementation also rejects collection/map storage and generic instantiation involving runtime anonymous records.
- HIR and backends receive ordinary nominal struct construction and field access. No anonymous-specific HIR node survives AST.
- Borrow and lifetime rules are exactly those of the lowered hidden nominal struct.

## Data ownership

- AST assigns a hidden nominal `TypeId` from stable source-site identity.
- `TypeEnvironment` owns ordered fields and field lookup.
- Diagnostic spelling identifies the anonymous record and source site without becoming semantic identity.
- HIR consumes the resolved `TypeId` and ordinary struct fields.
- Public-interface projection rejects the hidden identity rather than trying to canonicalise it.

## Non-goals

- no structural typing or shape unification
- no named type extraction or inference across literal sites
- no anonymous-record methods or conformance
- no public or cross-module anonymous type
- no anonymous record in generic arguments, collections or maps in the first implementation
- no pattern matching or destructuring syntax
- no backend-specific representation
- no changes to anonymous const-record semantics

## Implementation phases

### Phase 0: Mandatory architecture review

Before code:

- inventory expression delimiter ownership, hidden nominal registration, ordinary struct construction, field access, HIR lowering, copy semantics, borrow validation and lifetime escape checks
- decide the exact stable source-site key for hidden identities
- enumerate every prohibited escape boundary
- prove ordinary struct HIR can represent the feature without a second runtime record IR

Stop if the implementation needs structural type comparison, donor-local identity export or anonymous-specific backend nodes.

### Phase 1: Parse runtime record literals

- Add context-specific expression parsing for `| field = value, ... |`.
- Keep parameter, struct, choice, receiver and config grammar on existing paths.
- Reject duplicate fields, malformed separators, missing values and unterminated literals with structured diagnostics.
- Reuse anonymous const-record parsing helpers only where syntax is genuinely identical. Keep value semantics separate.

### Phase 2: Register hidden nominal types

- Create one hidden nominal identity per literal site.
- Register ordered fields after their value types resolve.
- Use a transient name index for duplicate detection and field lookup construction.
- Do not unify or intern by shape.
- Add readable diagnostics for mismatched anonymous sites.

Review gate: verify identity is source-site-based and cannot cross a public boundary.

### Phase 3: Enforce the local-only surface

- Allow local declarations, assignments compatible with the same site identity, copy and field reads.
- Reject returns, explicit signature use, aliases, receiver methods, conformances, exported surfaces, aggregate storage and generic requests.
- Report the error at the first escaping use rather than during backend lowering.

### Phase 4: Lower through ordinary struct HIR

- Reuse ordinary nominal struct construction and field projection.
- Preserve the hidden `TypeId` in the module-local type environment.
- Add no anonymous-specific HIR expression or statement variant.
- Validate HIR through existing nominal member checks.

### Phase 5: Borrow, lifetime and copy validation

- Reuse ordinary struct root/projection alias rules.
- Validate mutable field access through the existing place model.
- Ensure deep `copy` handles the record graph exactly as an ordinary struct.
- Verify hidden records cannot escape module/public lifetime summaries.

Review gate: run a read-only memory and HIR audit before backend-facing acceptance.

### Phase 6: Backend parity, tests and docs

- Confirm JS and Wasm lowerers consume only ordinary struct HIR/layout facts.
- Add integration cases for construction, projection, copy, mutability and every escape rejection.
- Update language and progress documentation.
- Rebuild generated docs.

## Stop conditions

Pause when:

- a second anonymous runtime representation appears necessary
- field lookup requires repeated shape scans
- hidden types enter public canonical identities
- generic or collection support expands the phase unexpectedly
- borrow/lifetime rules differ from ordinary structs
- a backend needs source-level anonymous-record knowledge

## Validation

Every code-bearing phase runs:

```bash
cargo fmt
just validate
```

## Final audit

Verify:

- each literal site owns one hidden nominal identity
- no structural equality exists
- no anonymous-specific HIR/backend path exists
- every public and unsupported escape is diagnosed before HIR/backend handoff
- ordinary struct borrow, lifetime and copy owners are reused
