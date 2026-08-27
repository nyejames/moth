# Named memory groups and general block removal implementation plan

## Purpose

Adopt bare named blocks as the final declared-memory-group syntax, remove the general source-level `block:` construct, keep anonymous `_:` blocks and control-flow labels out of the language and preserve keyword-led semantic scopes such as `async:`.

This is a source-surface and documentation migration. It does not implement declared memory groups, lifetime-region validation, group allocation or bulk reclamation. Until the memory implementation lands, the compiler should recognise `name:` in executable statement position as reserved declared-group syntax and report the existing deferred-feature lane.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/named-memory-groups-and-block-removal-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh after the path/file-value branch lands
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
PLAN_BASE: main
PREFERRED_IMPLEMENTATION_BASE: updated main after path-values-file-only-paths merges
STACKED_FALLBACK_BASE: path-values-file-only-paths
IMPLEMENTATION_SCOPE: keyword policy, executable statement dispatch, scoped-block source removal, deferred group syntax, diagnostics, fixtures and documentation
```

## Branch and integration decision

The plan and low-conflict documentation corrections belong on `main` now.

The full compiler migration should not be implemented independently on current `main` while `path-values-file-only-paths` remains unmerged. At the time this plan was written, that branch is 14 commits ahead of and one commit behind `main`. It changes several files this migration also needs:

- `src/compiler_frontend/ast/statements/body_symbol.rs`
- `src/compiler_frontend/keywords.rs`
- `src/compiler_frontend/tokenizer/tokens.rs`
- compiler diagnostic payload and rendering owners
- `docs/compiler-design-overview.md`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/src/docs/progress/@page.moth`
- `tests/cases/manifest.toml`

Preferred order:

1. Merge this plan and its independent memory-documentation corrections to `main`.
2. Merge or rebase `main` into `path-values-file-only-paths` as normal.
3. Finish and merge the path/file-value work.
4. Start this implementation from the resulting updated `main`.

If implementation must start before the path branch lands, branch from `path-values-file-only-paths` and stack the implementation on it. Do not create a parallel implementation from current `main`. The overlapping edits are small in some files, but duplicated parser, keyword, diagnostic and documentation cutovers create needless conflict risk.

This feature is otherwise independent of file-value semantics. It does not change path tokenisation, `Path`, resource identity, dependency clauses or expression-position file paths.

## Required authority documents

Read these before implementation:

- `docs/compiler-design-overview.md`
- `docs/src/developer-docs/language/overview.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/src/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf`
- `docs/src/developer-docs/memory-management/lifetime-regions-and-escape-validation/lifetime-regions-and-escape-validation.mtf`
- `docs/src/developer-docs/memory-management/declared-memory-groups/declared-memory-groups.mtf`
- `docs/src/docs/memory/declared-memory-groups.mtf`
- `docs/src/docs/design-scope/`
- `docs/src/docs/async/@page.moth`
- `docs/src/docs/progress/@page.moth`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/roadmap/plans/final-memory-management-redesign-and-implementation-plan.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`

The progress matrix remains the authority for current compiler support. The memory references describe accepted end-state semantics.

## Current implementation facts

The current compiler has an implemented source `block:` statement:

- `block` lexes to `TokenKind::Block`
- `block` is reserved against identifier use
- executable statement dispatch routes it to `parse_scoped_block_statement`
- parsing creates `NodeKind::ScopedBlock`
- HIR lowering creates a child lexical region then rejoins the parent
- focused parser, AST, HIR and integration tests cover this surface

The current compiler rejects `name:` after a statement-leading symbol as an unexpected colon and explains that bare labelled blocks are unsupported.

The accepted memory design currently spells declared groups as `group name:`. Group parsing and `into` placement are deferred.

Static Bool `if` specialisation also uses `NodeKind::ScopedBlock` internally. It replaces a selected authored `if` with a scoped wrapper so the chosen branch keeps its lexical scope while no runtime `if` reaches HIR. Removing the source `block:` construct must not delete that internal scope-preservation mechanism.

## Accepted final source design

### Named declared groups

A declared memory group uses a bare value-like identifier followed by `:`:

```moth
request:
    parsed ParsedPost into request = parse_post(post)
    html String into request = render_post(parsed)
;
```

The grammar claimed in executable statement position is:

```text
identifier ":" body ";"
```

Rules:

- `name:` declares one named hard lifetime group
- the group is valid only in runtime executable bodies
- the group name uses normal value-like identifier policy
- the group name is semantic lifetime metadata, not a value or type
- the name cannot collide with a visible value, type, dependency binding, constant, reactive source or active group
- exact `_:` is invalid and does not declare an anonymous group or lexical block
- there is no `group` keyword
- `block` is reclaimed as an ordinary identifier spelling
- once groups are implemented, `block:` means a group whose name is `block`
- ordinary declarations written inside the group are not implicitly group-owned
- only a declaration with `into name` places its fresh result or independent graph into the group
- nested groups remain valid
- a group with no direct or nested placement targeting it emits an unused-group warning

The no-placement warning counts explicit placement from a straight-line nested child group into an ancestor. It must not count an ordinary ungrouped declaration merely because that declaration is lexically inside the group.

### `into` placement

The accepted placement shape remains:

```text
name [access/type] into group_name = expression
```

`into group_name` remains attached to declaration receiving boundaries after access or type syntax and before `=`. This plan changes only the group header spelling and the surrounding scope surface.

### Ancestor placement

A declaration may target an ancestor group only through a straight-line nested named group that executes at most once.

```moth
request:
    scratch:
        parsed ParsedPost into scratch = parse_post(post)
        html String into request = render_post(parsed)
    ;

    use(html)
;
```

Remove the old allowance for a straight-line nested general `block:` because that source construct no longer exists.

Ancestor placement remains invalid when any conditional or repeatable construct lies between the declaration and destination group. A statically known `if true:` is still authored conditional syntax for this rule and must not become an ancestor-placement loophole.

### No general lexical block

The final language has no source construct whose only meaning is to introduce an anonymous lexical scope:

```moth
-- invalid final syntax
block:
    temporary = calculate()
;

-- invalid final syntax
_:
    temporary = calculate()
;
```

Do not replace `block:` with braces or another anonymous spelling.

When a rare local-only scope is useful, an ordinary statically selected `if` remains available:

```moth
if true:
    temporary = calculate()
    use(temporary)
;
```

This is not a special block alias. It follows the normal static Bool `if` contract:

- both authored branches complete frontend validation
- the known branch is selected before HIR
- the selected lexical scope is preserved
- no runtime conditional or inactive executable facts remain
- it does not create a declared group
- it does not change group ancestor-placement rules

Substantial isolated work should normally use a named function with explicit parameters, results and failure behaviour.

### No labels

Bare `identifier:` syntax is fully claimed by declared memory groups. Moth does not add:

- arbitrary statement labels
- labelled blocks
- labelled `break`
- labelled `continue`
- `goto`

Unlabelled `break` and `continue` continue to target the nearest enclosing loop. `return` continues to target the current function. This keeps control-flow destinations structural rather than name-addressed.

### Keyword-led semantic scopes

Keyword-led scopes remain distinct from named groups:

```moth
async:
    ...
;
```

`async:` describes language-defined execution and lifetime behaviour. It is not an anonymous group or general block. The deferred `checked:` surface is also outside this migration except for any shared parser cleanup needed after `block` is removed.

Do not generalise `keyword:` and `identifier:` through one untyped label abstraction.

### Internal lexical scope nodes

The compiler still needs an internal lexical-scope wrapper for selected static branches and any other compiler-generated scope-preserving transformation.

For this migration:

- keep `NodeKind::ScopedBlock` as an internal compiler-generated node unless a focused rename to `LexicalScope` is completed everywhere in the same phase
- no source parser may emit that node after `block:` removal
- comments and tests must stop describing it as an authored `block:` statement
- HIR may keep a child-region lowering path for this internal node
- static `if true:` and `if false:` tests must prove scope preservation and runtime-branch elimination

Do not mechanically delete every use of the word block. HIR basic blocks, value-producing blocks, config blocks, export blocks, async blocks and compiler-generated lexical scopes are unrelated concepts.

## Non-goals

- no implementation of group allocation, group cleanup or group escape validation
- no implementation of `into` placement semantics in this plan
- no change to the one-owner, retained-edge, REC or group-cycle contracts
- no anonymous group syntax
- no replacement general block syntax
- no labels or labelled loop exits
- no special-case `if true` parser or optimiser
- no change to `async:` semantics
- no change to path or file-value syntax
- no broad rename of every internal HIR or parser block concept
- no compatibility mode that keeps authored `block:` as a second spelling

## Risks and invariants

### Static scope preservation

The largest correctness risk is deleting `NodeKind::ScopedBlock` because its name resembles the removed source feature. Static Bool specialisation currently depends on it. Preserve one internal scoped wrapper and prove that selected-branch locals remain invisible outside the branch.

### Statement-header classification order

`identifier:` must be classified before ordinary existing-reference, external-call and declaration parsing. Otherwise an existing local may route into expression parsing and an unknown name may route into type-annotation diagnostics.

The classifier must inspect the immediate statement-header shape, not scan arbitrary later tokens.

### `block` reclamation

After keyword removal:

- `block = value` must be an ordinary declaration
- `block ||:` must be an ordinary function declaration where functions are legal
- `block:` must route through named-group syntax, not the removed scoped-block parser

### `_:` rejection

The exact anonymous spelling must have a focused diagnostic. It must not fall through to a generic naming warning, declaration diagnostic or deferred group diagnostic.

### Deferred syntax boundary

Before declared groups are implemented, valid `name:` headers should produce a stable deferred-feature diagnostic. Do not build partial group AST or HIR nodes that later analyses could mistake for implemented memory semantics.

### Current versus accepted docs

User-facing and developer documentation must distinguish accepted final syntax from current compiler support until the parser cutover lands. The progress matrix must describe current behaviour at every commit.

## Implementation phases

Each phase must leave one coherent parser and documentation path.

### Phase 0: Refresh the implementation base

- Confirm whether `path-values-file-only-paths` has merged.
- Use updated `main` after that merge as the preferred base.
- If it has not merged and implementation must start, rebase this plan branch onto `path-values-file-only-paths` and stack the work there.
- Record `git rev-parse HEAD`, branch and `git status --short` in the current-state capsule.
- Re-read every required authority document.
- Run `git grep` for the stale source spellings and current implementation owners listed under validation searches.
- Run baseline `just validate` and record results.

### Phase 1: Finish the accepted-design documentation migration

Update source documentation before or with parser cutover. Do not edit only generated `docs/release/**` output.

Canonical memory documentation:

- change every declared-group header from `group name:` to `name:`
- change section names such as `group block` to `named group`
- state that `group` is not a keyword
- state that exact `_:` is invalid
- state that the final language has no general `block:` construct
- add the no-placement warning contract
- replace straight-line nested `block:` or `group:` ancestor placement with straight-line nested named group only
- state that `if true:` is an ordinary statically selected branch and not an ancestor-placement route
- update every example without changing the underlying lifetime semantics

Update at least:

- `docs/src/developer-docs/memory-management/declared-memory-groups/overview.mtf`
- `docs/src/developer-docs/memory-management/declared-memory-groups/declared-memory-groups.mtf`
- `docs/src/developer-docs/memory-management/lifetime-regions-and-escape-validation/lifetime-regions-and-escape-validation.mtf`
- `docs/src/developer-docs/memory-management/overview.mtf`
- `docs/src/developer-docs/memory-management/@page.moth`
- `docs/src/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf`
- `docs/src/docs/memory/declared-memory-groups-basic.mtf`
- `docs/src/docs/memory/declared-memory-groups.mtf`
- `docs/compiler-design-overview.md`
- `docs/roadmap/plans/final-memory-management-redesign-and-implementation-plan.md`

Language-surface and rationale documentation:

- update the cheatsheet group example and remove general `block:` from supported syntax
- update the progress matrix current-support row at the same commit as parser behaviour
- update design-scope pages to record no labels, no general lexical blocks and named groups claiming `identifier:`
- keep `async:` as a keyword-led semantic scope in `docs/src/docs/async/@page.moth`
- update any language overview or task-routing text that names `group` as a source keyword
- regenerate release documentation through the normal docs pipeline

### Phase 2: Remove `block` from source keyword policy

- Remove `TokenKind::Block`.
- Remove the `"block" -> TokenKind::Block` keyword classification.
- Remove `block` from reserved keyword-shadow policy.
- Update token display, diagnostic render context and highlighter classification exhaustiveness.
- Update keyword and lexer tests.
- Add tests proving `block`, `_block` and ordinary names containing `block` follow normal identifier policy.
- Add parser tests proving `block = 1` is a normal declaration and a legal function named `block` parses where function declarations are legal.
- Do not remove `checked`, `async` or unrelated block-shaped keywords.

### Phase 3: Claim `identifier:` for deferred named groups

Add one focused executable statement-header classifier before ordinary symbol-use and declaration dispatch.

Required behaviour:

- `Symbol(name)` followed immediately by `Colon` is named-group syntax
- classification occurs before existing-reference lookup, external-call lookup and `new_declaration`
- exact `_:` returns a focused invalid-anonymous-group diagnostic
- any other valid name currently returns a declared-memory-group deferred-feature diagnostic
- group-name collision and placement semantics remain for the later full group implementation
- the parser consumes enough of the header to report the correct source location without pretending the body was semantically implemented

Diagnostics:

- add a stable deferred reason for declared memory groups if one does not already exist
- render the accepted final spelling as `name:` and placement as `into name`
- remove the current unexpected-colon explanation that says bare `name:` blocks are unsupported and suggests `block:`
- keep `name: Type` declarations invalid because `name:` now starts a group and declarations retain `name Type = value`
- keep labelled `break` and `continue` invalid through their existing statement rules
- add reason keys and structured-payload tests where required by diagnostic policy

Do not add `TokenKind::Group` or `TokenKind::Into` solely to reserve the header. `into` tokenisation belongs to the full declared-group implementation unless a shared declaration-shell owner needs to reserve it earlier for a precise deferred diagnostic.

### Phase 4: Remove authored general scoped-block parsing

- Remove the `TokenKind::Block` body-dispatch arm.
- Delete or repurpose `src/compiler_frontend/ast/statements/scoped_blocks.rs` so no source `block:` parser remains.
- Remove source-specific reserved-block-name diagnostics.
- Remove `ContextKind::Block` if no remaining parser context needs it.
- Audit `ThenCrossesBlockedConstruct` and other context matches so they continue to describe real blocked constructs rather than the removed source scope.
- Delete focused authored-block parser tests.
- Remove or replace `tests/cases/block_scoped_success` and any invalid-block fixtures.
- Update `tests/cases/manifest.toml`.
- Remove dead parser counters, helpers and module exports.

Do not delete the compiler-generated `NodeKind::ScopedBlock` path in this phase.

### Phase 5: Reframe the internal scoped node

- Make the AST node documentation say it is compiler-generated lexical scope preservation.
- Ensure only compiler transformations such as static Bool specialisation create it.
- Update HIR labels and test names from authored `block:` wording to compiler-generated lexical-scope wording where practical.
- Keep the child-region and rejoin lowering unless a smaller equivalent representation already exists.
- Add an invariant assertion or focused test that the executable source parser cannot emit this node directly.
- Audit terminality, const collection, static-if normalisation, template flow, type validation and HIR lowering visitors for the retained node.

A full `ScopedBlock` to `LexicalScope` rename is allowed only if it is completed across every exhaustive visitor, test and comment in the same phase. Do not leave two equivalent internal node kinds.

### Phase 6: Harden static Bool scope behaviour

Add focused cases for:

- `if true:` local declarations are unavailable after the branch
- `if false:` without `else` introduces no visible bindings
- selected branches retain terminality behaviour
- nested selected branches retain correct scope ancestry
- inactive generated requests remain discarded
- inactive link, target and effect facts remain absent
- HIR receives no statically decided runtime `if`
- HIR still receives a child lexical region or equivalent scoped representation
- ordinary `if true:` inside a declared-group example does not become legal ancestor placement in the accepted design docs

These tests protect the practical lexical-scope escape hatch without creating a new source feature.

### Phase 7: Update the deferred declared-group implementation handoff

Update the group phase in `final-memory-management-redesign-and-implementation-plan.md` so the future implementation starts from the reserved header rather than the removed keyword form.

The later group implementation must:

- replace the deferred `identifier:` diagnostic with real group parsing
- create explicit AST group metadata separate from `TypeId`
- validate group names and collisions
- parse `into group_name` at declaration receiving boundaries
- count direct and nested placements for the unused-group warning
- allow ancestor placement only through straight-line nested named groups
- reject exact `_:`
- emit HIR group metadata and exits
- preserve all existing group topology, cycle, escape and cleanup contracts

This plan must not pre-implement a partial version of those semantics.

### Phase 8: Progress matrix, fixtures and generated docs cutover

At parser cutover:

- rename the progress row from `Variables, assignment, and scoped blocks` to a surface that does not claim general blocks
- state that general `block:` is removed
- state that bare labels remain unsupported
- add or update the declared-memory-group row as accepted deferred syntax using `name:` and `into name`
- keep current target coverage truthful
- update the cheatsheet to avoid presenting named groups as implemented
- regenerate `docs/release/**`
- remove stale integration expectations for authored blocks
- add deferred named-group fixtures, `_:` rejection and `block` identifier success fixtures

### Phase 9: Validation and stale-surface audit

Run:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
just validate
```

Run the repository docs build and anchor validation through the normal `just validate` path.

Required stale searches:

```text
git grep -n "group name:"
git grep -n "group request:"
git grep -n "Use `block:` for a scoped block"
git grep -n "bare labeled blocks"
git grep -n "Variables, assignment, and scoped blocks"
git grep -n "TokenKind::Block" -- src
git grep -n "parse_scoped_block_statement" -- src
git grep -n "block_scoped" -- src tests docs
```

Interpret results rather than blindly deleting every match. Generated release output should match regenerated sources. Historical discussion or an explicit migration note may retain old syntax only when clearly marked historical.

Recompare the implementation branch against `path-values-file-only-paths` if that branch still exists. Any remaining overlap should be resolved once rather than accepted as duplicate parser logic.

## Required tests

### Lexer and identifier policy

- `block` lexes as `Symbol`, not a keyword
- `block = 1` succeeds
- a function named `block` succeeds
- keyword-shadow tests no longer reject `block`
- `async` and `checked` remain keyword tokens

### Statement classification

- `request:` routes to declared-group syntax before declaration parsing
- `request:` produces the deferred group diagnostic until implementation
- an existing local followed by `:` still routes to group syntax rather than expression parsing
- `_:` produces the focused anonymous-group rejection
- `name Type = value` remains a declaration
- `Name must:` and function-header colons remain unaffected

### Removed source block

- authored `block:` no longer creates a general scoped block
- no parser test or fixture claims general blocks are supported
- `block:` follows the same named-group deferred path as another identifier

### Static scope preservation

- selected static branches keep child lexical scope
- branch locals do not leak
- no static Bool `if` reaches HIR
- internal scoped-node HIR lowering remains valid

### Documentation

- canonical memory examples use `name:`
- no canonical source describes `group` as a keyword
- no canonical source offers `block:` or `_:` as a general scope
- labels remain explicitly outside the language
- async remains a keyword-led semantic scope

## Completion criteria

The migration is complete when:

1. The accepted docs use `name:` and `into name` consistently.
2. `group` is not a source keyword.
3. `block` is not a source keyword and ordinary identifiers may use it.
4. The compiler has no authored general `block:` parser.
5. Exact `_:` is rejected.
6. `identifier:` is reserved for declared groups and reports a targeted deferred diagnostic until group implementation.
7. Labels and labelled loop exits remain unsupported.
8. Static Bool `if` still preserves selected lexical scope and emits no runtime branch.
9. The internal scoped wrapper remains coherent and is no longer documented as an authored statement.
10. The final memory-management plan hands future group implementation the new syntax and no obsolete block dependency.
11. The progress matrix and generated docs match current compiler behaviour.
12. Full validation passes on the branch that contains the path/file-value changes or on updated `main` after those changes merge.

## Handoff summary

The final design deliberately separates three scope families:

```text
identifier:  programmer-named declared memory group
keyword:     language-defined semantic scope such as async
if/loop/etc  structured control-flow scope
```

There is no fourth general anonymous lexical-block family. `if true:` remains an ordinary statically selected control-flow scope for the rare case where a programmer needs local name isolation without another semantic construct. The compiler keeps an internal lexical wrapper because static branch selection still needs to preserve scope, but source can no longer request that wrapper directly.
