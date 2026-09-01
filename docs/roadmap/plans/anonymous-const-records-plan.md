# Anonymous const records implementation plan

## Purpose

Implement anonymous compile-time records as a small field-access-only value surface that can be reused by project config, `@project`, ordinary constants and builder metadata without introducing runtime anonymous types.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/anonymous-const-records-plan.md
STATUS: complete
CURRENT_SLICE: final review accepted
LAST_GOOD_COMMIT: closeout in this commit
BRANCH: main
IMPLEMENTATION_SCOPE: AST constant expressions, folded values, const-record field projection, public folded interfaces
BASELINE_VALIDATE: passed at fabafbb88 (0 failures)
BOUNDARY: GO with folded-record field authority. No second evaluator. No per-site runtime TypeId. TIR does not cross folded values. Fingerprints planned, not implemented.
```

Keep this block concise. Git history is the implementation record.

## Phase 0 freeze

Anonymous const records are compile-time member groups, not types. Field names, order, locations and values live on `ConstValuePayload::Record` and `PublicFoldedValue::Record`. `.field` uses `ConstValueStore::field_value` and the existing HIR store-backed path in `hir_expression/places.rs`. Do not intern a `TypeDefinition` or hidden nominal `TypeId` to answer field access.

Complete anonymous records carry `ConstRecordState::ConstRecord` and must not lower to runtime HIR (`StructConstruct`, bindings, arguments, returns). `hir_statement/declarations.rs` `resolve_const_struct_id` stays struct-backed only.

`Expression.type_id` and `ConstValueMetadata.type_id` stay total. A complete anonymous record uses one module-local compile-time-only `TypeDefinition` marker interned once per `TypeEnvironment`. It is not a struct, has no fields or methods, is not source-writable, does not unify literal sites, and is never lowered. Semantic field types live on field values.

Public identity for an exported anonymous const record is `CanonicalTypeIdentity::AnonymousConstRecord`. Nested records are nested `PublicFoldedValue::Record` values only. `PublicFoldedField` carries `type_identity` projected from the field value's store metadata so import can materialize without `fields_for`. Nested anonymous fields project to `AnonymousConstRecord`; nested named structs project to their source nominal identity. Import restores `ConstRecordState::ConstRecord` for anonymous records and never calls `fields_for` on the marker type. Ordinary folded struct defaults keep today's struct `TypeId` path. `PublicFoldedValue::Record` alone does not imply const-record status.

One const evaluator: `ast/const_eval` (`constant_fold`, `fold_compile_time_expression`) plus `ConstValueStore::insert_expression`. Additional owners: `expressions/eval_expression/evaluator.rs`, `const_values/resolver.rs`, `hir/constants.rs`, `build_system/resource_unions.rs`. Expression `|` is `TokenKind::TypeParameterBracket` and is currently unexpected.

Phase 3 must replace the eager whole-record clone in `reference_expression_from_declaration` with borrowed or store-backed single-field projection. Phase 4 must reject imported complete records in runtime use and cover nested imported projection.

## Hard prerequisites

- accepted final TIR ownership and exact-view boundaries
- accepted canonical module and immutable public-interface architecture
- stable source locations, semantic identities and folded-value ownership

This plan must complete before grouped project config, build configuration values and entry-local config blocks.

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/developer-docs/language/overview.mtf` and its relevant canonical references
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`

## Accepted design

Anonymous const records use ordinary expression syntax in a compile-time receiving context:

```moth
inner #= |
    enabled = true,
|

metadata #= |
    channel = "alpha",
    nested = inner,
|

channel #= metadata.channel
```

Rules:

- `|...|` in expression position forms an anonymous const record only when the receiving context requires a compile-time value.
- Fields are named, ordered and unique.
- Every field initializer must fully fold.
- Nested anonymous const records are allowed by declaring the child record first and naming it as a parameter value. Do not nest one `|...|` list inside another.
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
- `|...|` is shared parameters syntax with structs and functions. A const record requires a value for every parameter and does not declare a constructable type.
- Do not parse a nested `|...|` list inside another. Declare the child struct or record first, then name it as a parameter value.
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

### Phase 4: Export and bind anonymous const records

- Extend the public folded-value vocabulary with anonymous record values.
- Validate every exported field recursively.
- Preserve deterministic field order, canonical scalar/collection/record values and provenance.
- Bind provider values without reparsing or refolding.
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
