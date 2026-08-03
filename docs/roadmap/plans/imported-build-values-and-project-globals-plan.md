# Imported build values and project globals implementation plan

## Purpose

Implement typed project and source build-input contracts, CLI/programmatic input resolution and the immutable `@project` interface after grouped project config is complete.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/imported-build-values-and-project-globals-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh CLI, config-field, header-contract and synthetic-interface owners
REVIEW_BASELINE: 47dbf3fd1dfa3e8df3d02cef05001de695ea80ee
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: CLI, config folding, header syntax, Stage 0 contract barrier, synthetic project interface
```

Keep this block concise. Git history is the implementation record.

## Hard prerequisites

- accepted canonical module Phase 5 closeout
- `docs/roadmap/plans/anonymous-const-records-plan.md`
- `docs/roadmap/plans/project-config-and-recursive-schemas-plan.md`
- stable public-interface provenance and package-boundary ownership

This plan must complete before entry-local config blocks.

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/docs/codebase/language/overview.mtf` and its relevant canonical references
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`

## Vocabulary

Keep these concepts distinct:

- **Package dependency declaration**: future `import @package` preamble in `config.moth`, owned by the later package plan.
- **Source import**: ordinary `.moth` visibility/import syntax.
- **Build-input contract**: `#Import` declaration resolved from project values, explicit command inputs, builder globals or defaults.

This plan implements only build-input contracts and `@project`.

## Accepted build-input surface

Direct project field:

```moth
project #= |
    name = "my_app",
    version #Import of String = "0.1.0",
    entry_root = "src",
|
```

Source contract:

```moth
api_url #Import of String = "http://localhost:8080"
optional_label #Import of String? = none
```

CLI:

```bash
moth build . --input api_url=https://example.com
moth check . --input api_url=https://example.com
moth dev . --input api_url=https://example.com
```

Accepted types:

- `String`
- `Int`
- `Float`
- `Bool`
- `Char`
- optional forms of those types

No runtime `Import` wrapper or HIR category exists.

## Resolution rules

Direct project `#Import` fields resolve during config folding:

1. explicit CLI or programmatic input
2. compatible builder-provided primitive global
3. folded declaration default
4. missing-input diagnostic

Project-wide source contracts resolve after Stage 0 has collected the selected graph:

1. compatible fixed direct project field, which is authoritative and cannot be overridden
2. resolved direct project `#Import` field
3. explicit CLI or programmatic input for a source-only contract
4. compatible builder-provided primitive global
5. shared source default
6. missing-input diagnostic

Same-name source contracts must agree on type, optionality, required/default state and normalised default.

## Source-default restriction

A source default must be one self-contained primitive literal or `none`:

- no name or constant reference
- no template
- no operator expression
- no call or cast
- no field projection
- no collection or record
- no second Stage 0 constant evaluator

## `ProjectGlobalsInterface`

The folded project record produces an immutable synthetic interface at `@project` containing:

- stable field identities
- folded backend-neutral values
- source locations
- field-level fingerprints
- member-level project-context provenance
- no AST, HIR or runtime body

Rules:

- `@project` is permanently reserved.
- Normal project modules and project-owned support packages may import it explicitly.
- It is never implicitly injected.
- It cannot be directly re-exported.
- Child modules, dependency aliases, Core, Builder and binding-backed packages cannot claim the root.
- The external project package facade rejects public or reachable executable dependence on private `@project` context.

## Boundary isolation

Every project or package compilation boundary owns its own input namespace.

- Root-project inputs do not implicitly satisfy dependency contracts.
- A dependency resolves only from its own config, defaults and compatible builder globals.
- Qualified dependency overrides and input forwarding remain package-manager design work.
- Input values persist through dev rebuilds inside their owning boundary.

## Non-goals

- no package dependency declaration syntax
- no package aliases, versions, paths, registry or lockfile
- no environment-variable syntax or env file
- no `-D`, `--define` or JSON input
- no build-input aliasing
- no runtime import wrapper
- no nested project `#Import` fields
- no `#Import` in builder/tooling sections or entry-local config blocks

## Implementation phases

### Phase 0: Refresh and baseline

- Record current revision, branch and worktree state.
- Inventory CLI parsing, grouped config fields, header syntax, Stage 0 source graphs, dev rebuild state and synthetic-interface provenance.
- Confirm project config and recursive schemas are accepted.
- Run baseline validation.

### Phase 1: Add typed input carriers

- Define one primitive/optional `BuildInputType` and one normalised `PrimitiveBuildValue`.
- Define `BuildInputName`, source locations and deterministic input maps.
- Use existing numeric text parsing for `Int` and `Float`.
- Reject non-finite floats.
- Preserve authored strings and chars without backend conversion.

### Phase 2: Parse command inputs

- Add repeated `--input name=value` to build, check and dev.
- Validate lower_snake_case input names.
- Preserve explicit input values in programmatic command options rather than global state.
- Reject duplicate explicit names deterministically.
- Delay unknown-input diagnostics until selected source contracts are known.

Review gate: verify all commands share one parser and typed carrier.

### Phase 3: Implement direct project `#Import`

- Allow `#Import of T` only on direct `project` fields.
- Reject nested project, builder and tooling occurrences.
- Resolve explicit input, builder global, default or missing diagnostics while config folds.
- Keep fixed direct fields separate from imported fields so fixed values block later overrides.
- Preserve field locations and fingerprints.

### Phase 4: Emit source contract shells

- Parse module-wide source `#Import` declarations during header syntax preparation.
- Normalise each declaration into `SourceBuildInputContract`.
- Reject body-local or unsupported-type declarations before AST.
- Restrict defaults to the accepted primitive literal/`none` forms.
- Keep contract shells out of imported symbol bindings and local declaration-ordering edges except where their resolved constant participates normally.

### Phase 5: Build the project-wide contract barrier

- Collect contracts from the selected semantic graph before any module AST uses them.
- Validate same-name compatibility once in deterministic source order.
- Resolve one value per input name through the accepted order.
- Diagnose unknown explicit inputs only after the complete selected contract set is known.
- Run the barrier separately for every project or package boundary.

Review gate: audit boundary isolation and prove no second constant evaluator exists.

### Phase 6: Feed resolved values into AST

- Convert each resolved source contract into an ordinary folded constant fact.
- Create no runtime node, wrapper type or HIR category.
- Preserve source declaration locations for diagnostics.
- Ensure build, check and dev produce identical frontend values.

### Phase 7: Implement `ProjectGlobalsInterface`

- Build stable field identities from project identity and field path.
- Project folded values, locations, fingerprints and provenance.
- Register the reserved `@project` synthetic interface in Stage 0 visibility.
- Bind explicit source imports through the ordinary imported-binding boundary.
- Track dependencies at field granularity.

### Phase 8: Validate facade provenance

- Carry project-field provenance through constant folding, public facts, HIR and generated functions.
- Reject direct or transitive project-context dependence selected by an external project package facade.
- Allow private internal project use where no external package export can reach it.

### Phase 9: Thread command and dev state

- Thread typed inputs and `ProjectGlobalsInterface` through build, check, dev and benchmark entry points.
- Preserve values across every dev rebuild.
- Remove duplicated defaulting or parsing logic from command-specific paths.

### Phase 10: Migrate fixtures and docs

- Add integration cases for project, source and boundary-isolation contracts.
- Update scaffolds and project-structure/build-input documentation.
- Update the progress matrix and rebuild generated docs.

## Required tests

Cover:

- every primitive and optional type
- numeric grammar and non-finite float rejection
- repeated and duplicate CLI inputs
- required and defaulted project fields
- fixed project field authority
- source contract compatibility and conflicts
- restricted source defaults
- unknown explicit inputs
- module-wide-only declaration placement
- build/check/dev parity
- dev rebuild retention
- `@project` explicit import and collisions
- field-level fingerprints and dependencies
- no direct re-export
- facade provenance rejection
- consuming-project input isolation from dependencies

## Validation

Every code-bearing phase runs:

```bash
cargo fmt
just validate
```

Run the documentation release build when source docs change.

## Final audit

Verify:

- one typed input vocabulary exists
- one CLI/programmatic resolution path exists
- source contracts resolve before AST without a second evaluator
- resolved inputs become ordinary folded constants
- `@project` is immutable, explicit and permanently reserved
- package dependency declarations remain outside this plan
