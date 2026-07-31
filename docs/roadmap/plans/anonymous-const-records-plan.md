# Anonymous const records implementation plan

## Purpose

Implement anonymous compile-time records as a small field-access-only value surface that can be reused by project config, `@project`, ordinary constants and builder metadata without introducing runtime anonymous types.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/anonymous-const-records-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh const-record parsing, folding, projection and interface owners
REVIEW_BASELINE: 47dbf3fd1dfa3e8df3d02cef05001de695ea80ee
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: AST constant expressions, folded values, const-record field projection, public folded interfaces
```

Keep this block concise. Git history is the implementation record.

## Hard prerequisites

- accepted final TIR ownership and exact-view boundaries
- accepted canonical module and immutable public-interface architecture
- stable source locations, semantic identities and folded-value ownership

This plan must complete before grouped project config, imported build values and entry-local config blocks.

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`

## Accepted design

Anonymous const records use ordinary expression syntax in a compile-time receiving context:

```moth
metadata #= |
    channel = "alpha",
    nested = |
        enabled = true,
    |,
|

channel #= metadata.channel
```

Rules:

- `|...|` in expression position forms an anonymous const record only when the receiving context requires a compile-time value.
- Fields are named, ordered and unique.
- Every field initializer must fully fold.
- Nested anonymous const records are allowed.
- The record is a compile-time member group, not a runtime value or structural source type.
- Field projection may participate in later constant folding.
- The complete record cannot be assigned to a runtime binding, passed, returned, stored in a runtime aggregate or used through a receiver method.
- Anonymous const records have no receiver methods, conformance, runtime constructor, HIR node or backend representation.
- Different literal sites do not unify into a runtime type by shape.
- Exported anonymous const records are allowed when every reachable field is representable by the public folded-value vocabulary.
- Field order, names, folded values and provenance determine deterministic interface and fingerprint encoding.

Use the existing `ConstRecordState`, const facts and field-projection path where practical. Do not manufacture a runtime `TypeId` only to represent compile-time field access.

## Data ownership

Use one compact folded representation equivalent to:

```rust
pub struct AnonymousConstRecord {
    pub fields: Vec<AnonymousConstRecordField>,
}

pub struct AnonymousConstRecordField {
    pub name: StringId,
    pub value: FoldedConstValue,
    pub location: SourceLocation,
}
```

Exact names may change. Required invariants:

- deterministic authored field order
- one transient name-to-index map while validating or projecting fields
- no final map-of-maps representation
- no cloned AST subtree after folding
- provenance merged once while the value is built
- public projection copies owned backend-neutral values, not AST expressions

## Non-goals

- no runtime anonymous records
- no structural typing or shape equality
- no named anonymous-record type syntax
- no methods or conformance
- no pattern matching over anonymous records
- no anonymous-record collection/map element type
- no anonymous-specific HIR or backend node
- no config schema implementation in this plan

## Implementation phases

### Phase 0: Refresh owners and freeze the boundary

- Record the current branch, revision and worktree state.
- Inventory `ConstRecordState`, const facts, struct-backed const records, expression dispatch, field projection, public folded values and interface fingerprints.
- Confirm no TIR authority crosses the folded-value boundary.
- Run baseline validation and record only current failures in the state capsule.

Stop if the feature requires a second const evaluator or a runtime type-system change.

### Phase 1: Add the folded record vocabulary

- Add the anonymous const-record value to the canonical const-fact vocabulary.
- Store ordered fields and field locations in contiguous vectors.
- Add one transient duplicate-name index during construction.
- Preserve nested values and synthetic-interface provenance.
- Add deterministic remapping and equality helpers required by existing const and interface owners.

Review gate: verify one folded value owner and no parallel config-only record representation.

### Phase 2: Parse expression-position record literals

- Recognise `|...|` only when expression parsing owns a compile-time receiving context.
- Reuse existing field declaration syntax where it matches without routing through named struct declaration parsing.
- Reject positional fields, duplicate names, missing values, malformed separators and unterminated records with structured diagnostics.
- Keep parameter lists, struct declarations, choice payloads, receiver signatures and template grammar on their existing owners.
- Reject runtime-context anonymous record literals with a targeted deferred/unsupported diagnostic until the runtime follow-up plan lands.

### Phase 3: Fold and project fields

- Fold every field through the ordinary constant evaluator.
- Reject the complete record when any field is non-constant.
- Reuse const-record field projection for `record.field` and nested chains.
- Preserve the use-site location for diagnostics while retaining declaration provenance.
- Ensure field projection never rebuilds or clones the whole record.

Review gate: inspect parser context separation, nested folding and projection ownership.

### Phase 4: Export and import anonymous const records

- Extend the public folded-value vocabulary with anonymous record values.
- Validate every exported field recursively.
- Preserve deterministic field order, canonical scalar/collection/record values and provenance.
- Import provider values without reparsing or refolding.
- Reject any value that would require a runtime anonymous type or private semantic identity.

### Phase 5: Delete temporary paths and migrate documentation

- Remove any config-only or test-only anonymous record representation introduced during implementation.
- Add focused integration cases for local, nested, exported and imported field projection.
- Update language, constants and progress documentation.
- Rebuild generated documentation.

## Required tests

Cover:

- empty and non-empty anonymous const records
- nested records
- deterministic field order
- duplicate fields
- malformed and unterminated syntax
- non-constant field rejection
- field projection and nested projection
- complete-record runtime-use rejection
- exported and imported folded records
- provenance preservation
- no anonymous-record HIR node
- no runtime `TypeId` created solely for a const record

## Validation

Every code-bearing phase runs:

```bash
cargo fmt
just validate
```

Run the documentation release build when source docs change.

## Final audit

Before completion, verify:

- one anonymous const-record folded representation exists
- the parser is context-specific
- no runtime type or HIR path was introduced
- public interfaces carry only owned folded values
- field projection performs no whole-record cloning
- runtime anonymous records remain deferred to their separate plan
