# Project config and recursive schemas implementation plan

## Purpose

Replace the transitional flat project config with one grouped, self-contained `config.moth` model using anonymous const records, recursive project/builder/tooling schemas and builder-owned output settings.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/project-config-and-recursive-schemas-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh config bootstrap, flat registry, builder schema and output owners
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
BRANCH: main
IMPLEMENTATION_SCOPE: build-system config bootstrap, folded config values, recursive schemas, builder-owned settings
```

Keep this block concise. Git history is the implementation record.

## Hard prerequisites

- accepted canonical module Phase 5 closeout
- anonymous const records, folded and projected through public interfaces
- command-selected builder capability surface

This plan must complete before build configuration values, `@project` and entry-local config blocks.

## Required authorities

- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/developer-docs/language/overview.mtf` and its relevant canonical references
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`

## Current implementation to replace

The current compiler already tokenises and folds one authored `config.moth`, but validation still consumes flat top-level keys through `ProjectConfigKeyRegistry`. `Config` still owns transitional fields such as `project`, `dev_folder`, `output_folder` and `package_folders`.

Do not rebuild the already-correct single-file parser. Replace the value/schema/storage boundary directly.

## Accepted config design

```moth
default_channel #= "alpha"

project #= |
    name = "moth_docs",
    entry_root = "src",
    metadata = |
        channel = default_channel,
    |,
|

html #= |
    dev_output = "dev",
    release_output = "release",
|
```

Rules:

- The command selects the builder before config validation.
- `config.moth` is one authored compile-time file and produces no HIR, `start`, module or package interface.
- One open `project` const record is required.
- `project.name` is required and provides stable project identity.
- Private helper constants declared before use are allowed.
- Every other top-level const record is a potential builder or tooling section.
- All sections are parsed and folded.
- Only active builder/tooling sections are schema-validated and retained.
- The active artefact-builder project section is required, even when empty.
- Project and entry schemas are separate. No field has a shared project/entry scope.
- Project fields may contain folded scalars, optionals, collections, templates-as-strings and nested anonymous const records.
- Builder and tooling sections consume backend-neutral folded values.
- Output settings belong to the active builder section.
- Named support structs, choices and aliases are not part of config after anonymous const records land.
- Package dependency declarations are not implemented here. Until the later package plan is accepted, every config dependency remains rejected before resolution.

## Recursive schema model

Use a compact data-oriented schema equivalent to:

```rust
pub struct ConfigSchema {
    pub nodes: Vec<ConfigSchemaNode>,
    pub fields: Vec<ConfigSchemaField>,
    pub root: ConfigSchemaNodeId,
}
```

Requirements:

- deterministic vectors and dense IDs in final schema data
- transient field-name indexes built once per validation operation
- explicit folded value shapes
- required/default/closed-domain facts on field records
- no trait-object schema hierarchy
- no map-of-maps final representation
- no string-backed catch-all settings store
- unknown active fields are diagnostics
- inactive sections are folded but not schema-validated

## Non-goals

- no package dependency declarations
- no `#Config` or CLI build-input implementation
- no `@project`
- no entry-local `config:` blocks
- no builder selection inside config
- no config source graph or support files
- no compatibility parser for the old flat shape
- no general runtime anonymous records

## Implementation phases

### Phase 0: Refresh and baseline

- Record current revision, branch and worktree state.
- Inventory config parsing, validation, `ProjectConfigKeyRegistry`, `Config`, builder validation and output-root resolution.
- Confirm canonical package discovery no longer depends on `package_folders`; stop if that Phase 5 deletion is incomplete.
- Run baseline validation.

### Phase 1: Introduce folded config output types

- Define one `FoldedProjectConfig` boundary containing the validated project record, retained active sections and source locations.
- Consume anonymous const records directly rather than converting them into strings.
- Preserve inactive folded sections only until active-schema selection completes, then discard them.
- Keep config AST and const facts inside bootstrap.

Review gate: verify no second config language or duplicate folded-value model exists.

### Phase 2: Implement recursive schemas

- Replace `ProjectConfigKeyRegistry` with the recursive schema owner.
- Register compiler-owned project fields separately from builder and tooling sections.
- Keep project and entry schema roots distinct.
- Validate nested records, collections, optional values, required fields, defaults and closed domains recursively.
- Diagnose duplicate section names and top-level constant collisions.

### Phase 3: Validate the grouped project record

- Require exactly one authored `project` const record.
- Validate `project.name`, `entry_root` and future compiler-owned fields through the project schema.
- Allow additional folded project metadata outside the compiler-owned closed field set.
- Preserve field locations and deterministic field order.
- Reject implicit sibling field scope. Reusable values must be earlier helper constants.

### Phase 4: Validate builder and tooling sections

- Require the selected builder's project section, even when empty.
- Validate active sections through their registered schemas.
- Fold but do not validate or retain inactive sections.
- Reject `#Config` in builder/tooling sections. The later build-configuration-values plan permits `#Config` only on direct project fields and module-wide source contracts.
- Store typed folded section results rather than stringifying them into `Config.settings`.

Review gate: audit active/inactive semantics and schema ownership before storage migration.

### Phase 5: Move output settings to builders

- Move HTML `dev_output` and `release_output` into the `html` project section.
- Preserve defaults `dev` and `release`.
- Validate output roots as relative, project-contained, outside `entry_root` and free of parent traversal.
- Update output resolution to consume typed active builder settings.

### Phase 6: Replace transitional `Config` storage

- Replace flat fields and `settings: HashMap<String, String>` with typed bootstrap results needed by Stage 0 and the active builder.
- Keep source locations beside the values that own their diagnostics.
- Thread the new result through build, check, dev and benchmark bootstrap without parallel APIs.
- Keep builder capability construction before config validation.

### Phase 7: Delete the flat config system

Delete:

- `ProjectConfigKeyRegistry`
- `ConfigKeyOwner`
- flat key validation and fallback string storage
- `project #= "html"`
- global `dev_folder` and `output_folder`
- `package_folders` and `/lib` defaults
- named config support structs, choices and aliases
- old compatibility diagnostics and wrappers

Do not retain a legacy mode.

### Phase 8: Migrate fixtures, scaffolds and docs

- Update `moth new` config output.
- Migrate all config fixtures.
- Update project-config, project-structure, language and progress documentation.
- Rebuild generated documentation.

## Stop conditions

Pause for review when:

- package resolution appears necessary during config folding
- a second config AST/folder scan is proposed
- active and inactive sections require separate parsers
- a schema abstraction exists only to preserve the flat registry API
- typed values are converted to strings before the builder consumes them
- the phase crosses more than two unplanned ownership boundaries

## Required tests

Cover:

- required grouped `project` record
- project name and entry-root validation
- private helper ordering
- nested anonymous const records
- active section validation
- inactive section folding without schema validation
- missing active builder section
- duplicate sections and field collisions
- separate project and entry schemas
- typed builder settings
- output-root defaults and validation
- rejection of dependency declarations, named support types and legacy flat keys
- no config HIR or source graph

## Validation

Every code-bearing phase runs:

```bash
cargo fmt
just validate
```

Run the documentation release build when source docs change.

## Final audit

Verify:

- one self-contained config parse/fold path exists
- one recursive schema owner exists
- no flat key registry or string settings map remains
- project and entry schemas are separate
- output settings are builder-owned
- package dependency declarations, build inputs and runtime anonymous records remain outside this plan
