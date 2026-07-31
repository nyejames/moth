# Normal module root `@` marker migration plan

## Current state

```text
WORK_ID: module-root-at-marker
STATUS: active
BASE_REVISION: 545acab7f72b068e2f6e13d5ade463d106399f1a
CURRENT_PHASE: ready for implementation
NEXT_ACTION: run the read-only inventory and baseline gates before editing
COMMIT_POLICY: one implementation commit containing the complete migration
```

Update this capsule when the migration lands. Keep it concise. Git history is the implementation record, so do not append command transcripts or worker journals.

## Purpose

Replace `#*.moth` with `@*.moth` as the filename convention for normal module roots across the compiler, build system, repository sources, fixtures, scaffolding, documentation and current roadmap plans.

The new marker deliberately connects a module's root file to its import-root role:

```text
src/
├── @site.moth
├── accounts.moth
└── pages/
    └── @pages.moth
```

```moth
import @pages
```

The root file itself is not imported by filename. The directory remains the module identity and import facade. The suffix after `@` remains cosmetic.

This is a hard Alpha migration. Do not retain the old marker through compatibility paths, fallback discovery or duplicate terminology.

## Locked design

These decisions are final for this migration.

- `@*.moth` defines a normal module root.
- `+*.moth` continues to define a scoped support root or project package facade.
- `config.moth` remains the project configuration filename and is not a module root.
- Import syntax remains `@path`. This migration does not add or alter import grammar.
- Do not invent `@@name`, escaped root-file imports or another spelling for directly importing an `@*.moth` file.
- A module root remains importable only through the directory's public `export:` surface.
- The root filename suffix remains cosmetic and must not enter stable module identity.
- `#` keeps its existing source-language meaning for compile-time constants and const templates.
- Do not perform a broad `#` to `@` text replacement.
- Existing `#*.moth` files are invalid legacy root-like filenames after the migration. They must not be treated as normal roots, ordinary importable source files or a compatibility form.
- Prefer a focused structured diagnostic that tells the author to rename `#name.moth` to `@name.moth` when Stage 0 encounters a legacy root-like filename.
- Do not preserve hash-root helper names, comments, test names or diagnostic wording after their behaviour has become normal-module-root behaviour.
- Internal names should describe semantic ownership, such as `normal_module_root`, rather than encoding the punctuation as `at_root`.
- Do not change module visibility, root-relative import resolution, support-package scope, facade rules, entry activation or stable semantic identity.

## Required authorities

Read these files before implementation and again before the final audit:

- `AGENTS.md`
- this plan
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/language/overview.mtf`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/#page.moth`, or its renamed path after the migration
- `docs/roadmap/roadmap.md`

The compiler overview owns semantic module identity and stage boundaries. The build-system design owns Stage 0 discovery, source ownership and project topology. This plan changes the normal-root filename marker without moving those responsibilities.

## Execution model

The coordinating agent owns all edits, integration, validation and the final commit. Use subagents for independent read-only audits and verification. Do not let subagents commit, push, rewrite shared files or create overlapping edits.

Every subagent must work from the same current worktree and report:

- exact file paths
- relevant symbols or test cases
- current behaviour
- required changes
- risks or uncertainty
- negative checks that should pass after migration

### Required initial subagents

Start these audits before editing.

#### Subagent A: compiler and Stage 0 ownership

Inspect root classification, source-tree discovery, module identity, import-environment handling, diagnostics and scaffolding.

Begin with:

```text
src/compiler_frontend/source_packages/root_file.rs
src/build_system/create_project_modules/module_identity.rs
src/build_system/create_project_modules/source_tree_index.rs
src/build_system/create_project_modules/compilation.rs
src/build_system/create_project_modules/module_namespace.rs
src/compiler_frontend/headers/import_environment/
src/compiler_frontend/compiler_messages/
src/projects/html_project/new_html_project/
```

Determine whether hash-root import-component helpers remain meaningful once `@` is both the filesystem marker and the import introducer. If the parser cannot express a root filename as an import component without inventing new syntax, recommend deleting those helpers and proving that facade bypass remains impossible.

#### Subagent B: tests, fixtures and tracked root files

Inventory every tracked `#*.moth` filename, every `hash_root` test or helper name, all fixture generators, test goldens and expected diagnostics.

Cover at least:

```text
src/compiler_frontend/source_packages/tests/
src/compiler_frontend/headers/tests/
src/compiler_frontend/compiler_messages/tests/
src/build_system/tests/
tests/cases/
benchmarks/
tmp/
examples/ if present
```

Report destination filenames before any `git mv`. Identify collisions where an `@*.moth` destination already exists.

#### Subagent C: documentation and roadmap

Inventory current positive references to `#*.moth`, `#page.moth`, `#mod.moth`, `hash root` and `hash-root` across authored documentation, indexes and roadmap plans.

Cover at least:

```text
AGENTS.md
CONTRIBUTING.md
README.md
index.md
docs/language-overview.md
docs/compiler-design-overview.md
docs/build-system-design.md
docs/src/**
docs/roadmap/roadmap.md
docs/roadmap/plans/**
```

Separate current design and path references from historical descriptions. Current and future-facing text must migrate. Historical notes may retain an old filename only when the exact legacy spelling is required to explain an old commit, and must label it as legacy.

Do not edit `docs/release/**` directly.

#### Subagent D: import grammar and ambiguity audit

Trace tokenizer, parser and resolver handling of `@` in imports and filenames. Prove the new root marker does not require:

- `@@name` syntax
- path escaping
- ordered fallback
- filesystem probing from the frontend
- new namespace precedence
- changes to ordinary package imports

Report the smallest clean deletion or replacement of old direct-hash-root import detection.

### Required final subagent

After all edits and targeted tests, dispatch a fresh read-only audit subagent. It must inspect the complete diff and search for:

- old implementation paths
- stale hash-root terminology
- missed root filenames
- accidental changes to compile-time `#` syntax
- duplicate root classification
- import grammar drift
- weak or missing diagnostics
- fixture and documentation gaps

The coordinating agent must resolve every actionable finding before the final gate and commit.

## Required tools

Use repository-local command-line tools from the worktree root.

### Inventory and review

Use:

```sh
git status --short
git rev-parse HEAD
git log --oneline --decorate -20
git ls-files
find
rg
git grep
git diff
git diff --check
```

Use `git ls-files` and `find` for filenames. Content search alone cannot find every `#*.moth` path.

Recommended initial inventory:

```sh
git ls-files | rg '(^|/)#[^/]*\.moth$|hash[-_ ]root'
find . -type f -name '#*.moth' -not -path './.git/*' -print | sort
rg -n --hidden \
  --glob '!.git/**' \
  --glob '!docs/release/**' \
  '(#\*\.moth|#[A-Za-z0-9_+.-]+\.moth|hash[-_ ]root|file_name_is_hash_root_file|import_component_is_hash_root_file|hash_root_file_name_from_import_component|import_path_references_hash_root_file)'
```

Treat this as a starting query, not an exhaustive allowlist. Search related enum variants, diagnostics, comments, statistics fields, fixture names and generated strings discovered from the first results.

### Renames

Use `git mv` for tracked files and directories. Do not copy files to the new path and leave delete/add pairs for the final cleanup.

A small temporary script under `/tmp` may produce an inventory or validate one-to-one rename destinations. Do not commit migration scripts. Do not use an unreviewed global replacement over source or documentation.

### Validation

Use:

```sh
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --terse
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
just validate
```

Use narrower test filters during iteration where useful, but they do not replace the full gate.

Do not use benchmark recording commands. `just validate` owns the non-recording benchmark sanity check.

## Scope

This migration owns:

- normal module-root filename classification
- Stage 0 discovery statistics and terminology
- root-role derivation
- direct special-file import rejection where still semantically required
- legacy `#*.moth` rejection
- related structured diagnostics and rendering
- repository root-file renames
- generated-project scaffolding
- unit and integration tests
- fixture and case directory names
- canonical compiler and build-system documentation
- public docs source and page-root filenames
- the progress matrix
- file indexes and contributor guidance
- every current roadmap plan that uses the old marker as current or future design
- generated documentation produced by a successful release build
- final plan and roadmap status updates

This migration does not own:

- support-root marker changes
- import syntax redesign
- package naming changes
- module identity redesign
- root-relative import topology changes
- export semantics
- the unrelated docs styles support-package blocker
- broad documentation rewriting
- compiler performance work
- compatibility with existing Alpha projects

If an unrelated correctness issue blocks validation, isolate and report it. Do not absorb it into this commit unless the marker migration directly caused it or the user separately expands scope.

## Known starting points

The current implementation centralises filename policy in `src/compiler_frontend/source_packages/root_file.rs`, but marker-specific terminology has leaked into callers, tests and documentation.

Expected hotspots include:

```text
src/compiler_frontend/source_packages/root_file.rs
src/build_system/create_project_modules/module_identity.rs
src/build_system/create_project_modules/source_tree_index.rs
src/build_system/create_project_modules/compilation.rs
src/build_system/create_project_modules/module_namespace.rs
src/compiler_frontend/headers/import_environment/builder.rs
src/compiler_frontend/headers/import_environment/diagnostics.rs
src/compiler_frontend/compiler_messages/render/mod.rs
src/compiler_frontend/compiler_messages/render/import_config.rs
src/projects/html_project/new_html_project/start_page_scaffolding.rs
src/compiler_frontend/source_packages/tests/root_file_tests.rs
src/build_system/tests/stage0_filesystem_identity_tests.rs
src/build_system/tests/create_project_modules_tests.rs
src/build_system/tests/compile_project_frontend_tests.rs
src/compiler_frontend/headers/tests/parse_file_headers_tests.rs
src/compiler_frontend/compiler_messages/tests/display_messages_tests.rs
tests/cases/**
```

This list is not exhaustive. The inventory is authoritative.

# Phase 0: baseline and complete inventory

## Goals

- prove the worktree starts clean
- pin the implementation base
- capture all tracked root files and marker terminology
- record current validation blockers before changes
- reconcile all subagent findings into one checklist

## Steps

- [ ] Read every required authority.
- [ ] Confirm `git status --short` is empty. Do not overwrite unrelated user work.
- [ ] Record `git rev-parse HEAD` in the plan capsule if it differs from the listed base.
- [ ] Run all four initial subagents.
- [ ] Run the filename and content searches under Required tools.
- [ ] Inspect every matching implementation owner rather than replacing from search output alone.
- [ ] Classify every match as one of:
  - [ ] normal-root implementation
  - [ ] tracked root filename
  - [ ] test or diagnostic expectation
  - [ ] current documentation or roadmap design
  - [ ] generated documentation
  - [ ] legitimate compile-time `#` syntax
  - [ ] historical legacy reference that should remain labelled
- [ ] Check for existing `@*.moth` files and destination collisions.
- [ ] Run targeted baseline tests for root-file classification, Stage 0 discovery, module graph construction, import diagnostics and project scaffolding.
- [ ] Run `cargo run --quiet -- check docs --terse` once to capture the current docs blocker precisely.
- [ ] Do not edit until the coordinating checklist includes every subagent's findings.

## Exit criteria

- [ ] Every tracked `#*.moth` file has one planned `@*.moth` destination.
- [ ] Every marker-specific symbol and diagnostic owner is identified.
- [ ] Current validation failures are recorded before implementation.
- [ ] No destination collision or unresolved ownership question remains.

# Phase 1: replace the filename-policy owner

## Goals

- make one central classifier recognise `@*.moth` normal roots
- keep support and config classification unchanged
- remove punctuation-specific internal terminology
- preserve directory-based semantic identity

## Steps

- [ ] Replace `file_name_is_hash_root_file` with a semantic normal-root name.
- [ ] Recognise one leading `@`, a non-empty cosmetic suffix and the normal `.moth` extension.
- [ ] Keep `file_name_is_support_root_file` and `file_name_is_config_file` as separate policies.
- [ ] Make the combined module-root classifier accept normal `@*.moth` and support `+*.moth` roots only.
- [ ] Rename callers, statistics and comments from `hash_root` to `normal_root` or `normal_module_root`.
- [ ] Keep `ModuleRootRole::Normal` unchanged unless the current code contains a separate marker-specific type that is now obsolete.
- [ ] Preserve stable module origin derivation from the directory and root role. The cosmetic filename must not enter identity or fingerprints.
- [ ] Keep root discovery in Stage 0. Do not add frontend filesystem probing.
- [ ] Remove duplicated or obsolete marker checks rather than layering a second classifier beside the first.

## Direct-root import handling

The old code recognises `#name` import components so it can issue a focused diagnostic for direct root-file imports. Do not mechanically replace that with `@name` component handling.

- [ ] Trace what import components the tokenizer and parser can represent.
- [ ] Keep the invariant that consumers import the module facade, never the root file.
- [ ] If an `@*.moth` filename cannot be expressed as an import component without new syntax, delete the obsolete hash-root import-component and filename-reconstruction helpers.
- [ ] Do not add `@@name`, quoting or escaping to preserve an obsolete diagnostic route.
- [ ] Keep or simplify special-file import rejection only where the existing grammar can reach it, such as `config.moth`.
- [ ] Add tests proving an importer cannot bypass a child module or support facade by naming its root file.

## Legacy marker diagnostic

- [ ] Reserve `#*.moth` as an invalid legacy root-like filename.
- [ ] Diagnose it during Stage 0 discovery with a structured project/source diagnostic that includes the path and replacement filename.
- [ ] Do not treat it as an unrooted ordinary source file.
- [ ] Do not silently ignore it.
- [ ] Do not auto-rename user files at runtime.
- [ ] Use a new diagnostic descriptor/code if this is a new semantic family. Do not repurpose an unrelated stable code.

## Exit criteria

- [ ] One owner classifies all root filenames.
- [ ] No production symbol or comment uses `hash_root` for normal module roots.
- [ ] Direct-root import protection requires no new import syntax.
- [ ] Legacy `#*.moth` input fails clearly.
- [ ] Stable module identity remains directory-based.

# Phase 2: rename every tracked normal root

## Goals

- migrate the repository itself atomically
- preserve file history
- leave no tracked positive use of `#*.moth`

## Steps

- [ ] Use the Phase 0 inventory as the rename manifest.
- [ ] Rename every tracked normal root with `git mv`:

```text
#name.moth -> @name.moth
```

- [ ] Include compiler fixtures, integration cases, docs source, built-in source packages, benchmarks, examples, temporary tracked test inputs and scaffold expectations.
- [ ] Rename directories or case IDs containing `hash_root` when they describe the current feature. Prefer semantic names such as `normal_module_root`.
- [ ] Do not rename unrelated source identifiers that use `#` for constants or Markdown headings.
- [ ] Verify every renamed root is still owned by the same directory and receives the same root role.
- [ ] Verify support roots remain `+*.moth`.
- [ ] Verify `config.moth` remains unchanged.

## Exit criteria

- [ ] `git ls-files | rg '(^|/)#[^/]*\.moth$'` returns no current root file.
- [ ] Every planned destination exists.
- [ ] No `@*.moth` collision was resolved through deletion or precedence.
- [ ] Git records the changes as renames where content is otherwise unchanged.

# Phase 3: update tests, diagnostics and scaffolding

## Goals

- protect the new marker at the correct layers
- remove tests of obsolete punctuation details
- verify user-visible project creation

## Unit and subsystem coverage

Update focused tests for:

- [ ] `@home.moth` and another non-empty suffix are normal roots.
- [ ] `@.moth`, wrong extensions and unprefixed files are not roots.
- [ ] `+*.moth` support roots remain unchanged.
- [ ] combined module-root classification accepts exactly normal and support roots.
- [ ] Stage 0 discovery statistics use normal-root terminology.
- [ ] deterministic module identity and ordering are unchanged by cosmetic suffixes.
- [ ] duplicate normal roots in one directory are rejected.
- [ ] mixed normal and support roots in one directory remain rejected according to current design.
- [ ] `@config.moth` is a normal module root while `config.moth` remains project config.
- [ ] a legacy `#config.moth` receives the legacy-marker diagnostic rather than config semantics.
- [ ] direct root-file import cannot bypass an `export:` facade.
- [ ] comments and rendered diagnostics say `normal module root` or `module root`, not `hash root`.

Use unit tests for classifier and deterministic table invariants. Use integration cases for user-visible discovery, topology and diagnostics.

## Integration fixtures

- [ ] Rename all root files in `tests/cases/**`.
- [ ] Update `expect.toml`, source excerpts and path assertions.
- [ ] Rename current-feature case directories containing `hash_root`.
- [ ] Add or update a canonical legacy-marker rejection case.
- [ ] Keep fixture coverage for nested modules, support roots, project facades, root-relative imports and source-package roots.
- [ ] Do not duplicate the same behaviour across several near-identical cases.

## Project scaffolding

- [ ] Make `moth new html` generate `src/@page.moth`.
- [ ] Rename helper comments, tests and exact path expectations.
- [ ] Run the existing scaffold test suite.
- [ ] Create a project under `/tmp` with the real CLI using its current accepted arguments.
- [ ] Verify the generated tree contains `src/@page.moth`, no `src/#page.moth`, and passes the appropriate check/build command except for independently established blockers.

## Exit criteria

- [ ] Targeted unit and integration tests pass.
- [ ] Diagnostic snapshots contain no stale hash-root wording.
- [ ] New-project generation uses the new marker end to end.
- [ ] No test protects an obsolete helper shape instead of observable behaviour or a real subsystem invariant.

# Phase 4: migrate authored documentation and roadmap state

## Goals

- make every current authority describe `@*.moth`
- rename public docs roots
- preserve the intended connection between root filenames and import roots
- keep active plans executable after the migration

## Canonical documentation

Update at least:

```text
AGENTS.md
CONTRIBUTING.md
index.md
docs/language-overview.md
docs/compiler-design-overview.md
docs/build-system-design.md
docs/src/docs/codebase/compiler-design/**
docs/src/docs/project-structure/**
docs/src/docs/packages/**
docs/src/docs/progress/**
```

Required wording:

- [ ] `@*.moth` is the normal module-root filename convention.
- [ ] `+*.moth` is the support/facade root convention.
- [ ] the suffix is cosmetic.
- [ ] the directory owns module and import identity.
- [ ] imports use the module or package path, not the root filename.
- [ ] the shared `@` visually signals the relationship between a module root and its import root without making the filename an import expression.
- [ ] direct root-file imports remain invalid or unrepresentable by design.
- [ ] `#` remains the compile-time declaration marker.

Rename authored documentation root files such as `#page.moth` to `@page.moth` with `git mv`. Update every path link and reading-list reference.

Do not turn this migration into a broad prose pass. Change only marker-related wording, paths and examples plus immediate clarity needed to avoid ambiguity.

## Roadmap and current plans

Audit all of:

```text
docs/roadmap/roadmap.md
docs/roadmap/plans/**
```

- [ ] Update active, queued and deferred plans where `#*.moth`, `#page.moth`, `#mod.moth`, `hash root` or `hash-root` describes current or future design.
- [ ] Update concrete task paths so another agent can execute each plan after the migration.
- [ ] Update the docs migration plan's root-file paths and wording without changing its unrelated sequencing.
- [ ] Update the canonical module plan and any data-layout, diagnostics, numeric, memory or template plans that cite root filenames or module-root terminology.
- [ ] Preserve an exact old filename only when documenting a historical commit or completed migration fact that would become misleading if rewritten. Label it `legacy` in that sentence.
- [ ] Do not retain stale examples merely because a plan is not currently active.
- [ ] Update `docs/roadmap/roadmap.md` sequencing so this migration is no longer listed as active after completion.
- [ ] Set this plan capsule to `STATUS: complete`, record the migration commit as pending before commit, then replace it with the resulting commit in a follow-up only if project convention permits. Do not create a second migration commit solely to write its own SHA.

## Progress matrix and index

- [ ] Review the progress matrix because supported filename syntax changes.
- [ ] Update the current support row and any examples.
- [ ] Update `index.md` for every renamed authored file or folder.
- [ ] Do not make unrelated progress claims.

## Exit criteria

- [ ] Every current authority uses the new marker.
- [ ] Every executable roadmap task points to current paths.
- [ ] Public docs explain the intentional root/import relationship without implying root files are directly imported.
- [ ] Historical exceptions are explicit and minimal.

# Phase 5: rebuild generated documentation

## Goals

- regenerate public HTML only through the compiler
- prove renamed docs roots are discoverable
- avoid manual generated edits

## Steps

- [ ] Run `cargo run --quiet -- check docs --terse` during iteration.
- [ ] Run `cargo run --quiet -- build docs --release` when the docs graph reaches the build boundary.
- [ ] Do not edit `docs/release/**` directly.
- [ ] Inspect every generated diff that mentions root filenames, module roots, project layouts or import paths.
- [ ] Verify generated links no longer contain obsolete `#page.moth` paths.
- [ ] Verify generated page routes remain unchanged unless the existing builder derives a route from a cosmetic filename, which would be a migration bug to fix.

The base revision has a documented pre-existing docs source graph blocker around the styles directory. Re-check current state rather than assuming it remains. If it still blocks the release build:

- [ ] prove the failure is the same pre-existing issue and not caused by this migration
- [ ] do not edit generated HTML
- [ ] do not absorb the styles-package repair into this commit
- [ ] report the exact remaining uncertainty

If the blocker has already been fixed, the docs release build must pass.

## Exit criteria

- [ ] Docs check and release build pass, or only the independently proven pre-existing blocker remains.
- [ ] Generated output was produced only by the compiler.
- [ ] Route identity and output layout did not change because of cosmetic root filenames.

# Phase 6: final negative search and architecture audit

## Goals

- prove the migration is complete
- prevent accidental source-language changes
- remove all transitional duplication

## Required searches

Run:

```sh
git ls-files | rg '(^|/)#[^/]*\.moth$'
rg -n --hidden \
  --glob '!.git/**' \
  --glob '!docs/release/**' \
  '(hash[-_ ]root|file_name_is_hash_root_file|import_component_is_hash_root_file|hash_root_file_name_from_import_component|import_path_references_hash_root_file)'
rg -n --hidden \
  --glob '!.git/**' \
  '(#\*\.moth|#[A-Za-z0-9_+.-]+\.moth)'
```

Classify every remaining match. Allowed matches are limited to:

- this migration plan's description of the removed form
- the focused legacy-marker diagnostic and its tests
- explicit historical text labelled as legacy

No current positive implementation, fixture, path, example or task may remain.

## Architecture audit

Verify:

- [ ] Stage 0 remains the sole filesystem discovery owner.
- [ ] filename policy still has one classifier owner.
- [ ] no frontend stage probes the filesystem to compensate for the rename.
- [ ] normal and support root roles remain distinct.
- [ ] directory-based stable identity is unchanged.
- [ ] import syntax and namespace precedence are unchanged.
- [ ] no `@@` syntax or escape mechanism was added.
- [ ] no compatibility shim, fallback classifier or duplicate old/new path remains.
- [ ] legacy input uses a structured diagnostic with useful path context.
- [ ] `#` constants, const templates and Markdown headings were not accidentally rewritten.
- [ ] comments describe ownership and purpose rather than narrating the migration.
- [ ] tests protect behaviour and real invariants.
- [ ] documentation and current plans point to renamed files.

Run the required final read-only audit subagent now. Resolve every actionable finding.

# Phase 7: validation and one-commit delivery

## Targeted validation

During implementation run focused tests for the touched owners. Before the final gate run at least:

```sh
cargo fmt
cargo test --workspace --quiet -- --format terse
cargo run --quiet -- tests --terse
cargo run --quiet -- check docs --terse
```

Run the scaffold smoke test under `/tmp` and inspect its generated tree.

## Final gate

This is a mixed code, fixture and documentation change. Run:

```sh
just validate
```

If the independently established docs blocker still prevents the full gate, do not claim full validation. Record:

- exact failing command
- exact failure
- targeted checks that passed
- evidence that the failure predates and is unchanged by this migration
- remaining uncertainty

Do not suppress a failure, remove coverage or edit generated output to make the gate appear green.

## Commit preparation

The implementation must be delivered as one commit.

- [ ] Do not create phase commits.
- [ ] Subagents must not commit.
- [ ] Keep all edits and `git mv` operations unstaged or staged locally until the whole migration is ready.
- [ ] Update this plan and roadmap state in the same implementation commit.
- [ ] Run `git diff --check`.
- [ ] Inspect `git diff --name-status` and `git diff --stat`.
- [ ] Inspect every rename and every non-mechanical code change.
- [ ] Confirm no unrelated user changes are included.
- [ ] Stage the complete coherent migration once.
- [ ] Inspect `git diff --cached --check` and `git diff --cached --stat`.
- [ ] Commit with:

```text
migrate normal module roots from # to @
```

- [ ] Do not amend unrelated history or squash other work into this commit.
- [ ] After commit, verify `git status --short` is empty.
- [ ] Report the commit SHA, validation results and any independently proven blocker.

## Completion criteria

The migration is complete only when:

- every repository normal root is named `@*.moth`
- `#*.moth` is rejected as a legacy root-like filename
- support roots remain `+*.moth`
- import syntax remains unchanged
- no `@@` syntax exists
- directory-based module identity and routes are unchanged
- scaffolding generates `src/@page.moth`
- tests and diagnostics protect the new contract
- authored docs and current roadmap plans use current paths
- generated docs are rebuilt when the docs gate permits it
- stale hash-root helpers, terminology and fixtures are removed
- compile-time `#` syntax remains intact
- the final architecture audit finds no duplicate or transitional path
- the complete migration is contained in one implementation commit

## Stop conditions

Stop and report before editing further if:

- the worktree contains unrelated uncommitted changes that cannot be isolated
- an `@*.moth` destination collision has no design-authorised resolution
- implementing the marker requires changing import grammar or inventing `@@` syntax
- stable module identity currently depends on the cosmetic root filename
- the current branch has materially changed module-root ownership since the plan base and the new owner cannot be reconciled cleanly
- a required diagnostic would need to bypass the structured diagnostic lanes
- the migration cannot be completed without an unrelated architectural change

Do not guess through these conflicts. Report the exact files, current owner and smallest decision required.