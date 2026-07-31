# Language migration pre-B1 documentation closeout

## Purpose

Finish the bounded documentation-status and generated-output work that remains after Stage A and Stage W, then prove the repository is ready for Stage B compiler changes.

This plan does not reopen the language migration. It owns only:

- current-versus-accepted status corrections
- final terminology and link cleanup
- generated documentation regeneration
- generated-page audits
- final validation evidence
- migration-plan handoff to B1

The direct source corrections already prepared on `agent/docs-status-link-corrections` are the starting point. Preserve them.

The separate benchmark and output-system correction plan remains the owner of output manifests, profile ownership, dev output planning, scaffold manifests, filesystem preflight and benchmark baselines:

```text
docs/roadmap/plans/benchmark-correctness-follow-up-implementation-plan.md
```

Do not duplicate that work here.

## Current state

```text
STATUS: ready for implementation
BRANCH: agent/docs-status-link-corrections
BASELINE: latest main after the benchmark/output correction work is accepted
CURRENT_SLICE: Phase 0 - rebase and inventory
NEXT_ACTION: preserve the direct source corrections, reconcile the progress matrix, regenerate docs and run the full gate
STAGE_B: B1 remains blocked until this plan reports a green final checkpoint
```

Keep this capsule concise. Git history is the implementation record.

## Required reading

Read before editing:

- `AGENTS.md`
- `docs/roadmap/plans/docs-language-migration.md`
- `docs/roadmap/plans/docs-language-migration-parity-ledger.md`
- `docs/roadmap/plans/benchmark-correctness-follow-up-implementation-plan.md`
- `docs/roadmap/plans/project-config-and-recursive-schemas-plan.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`

## Non-goals

Do not:

- start B1 or any other Stage B compiler change
- implement grouped config, `@project`, build inputs or entry-local config
- remove `package_folders` or the scaffold's legacy `lib/` directory
- implement the benchmark/output correction plan here
- switch documentation authority
- edit `AGENTS.md`
- edit `docs/release/**` manually
- perform another broad writing-style pass
- add compatibility wording that presents removed design as accepted language

## Accepted status split

Keep these authorities distinct:

- Focused Advanced pages describe the accepted final source contract.
- Basic pages teach current stable syntax unless they clearly label accepted deferred design.
- The progress matrix describes what the compiler implements today.
- The parity ledger separates documentation completeness from implementation status.
- Roadmap plans own sequencing and implementation drift.

The current compiler may legitimately differ from accepted design, but the difference must be stated accurately.

# Phase 0 - Rebase and inventory

1. Record branch, revision and `git status --short`.
2. Rebase the working branch onto the accepted benchmark/output correction commit.
3. Preserve the direct source fixes already on `agent/docs-status-link-corrections`:
   - deferred Project Config status notes
   - corrected `nyejames/moth` repository links
   - current scaffold and `tests --terse` wording
   - exact scoped support-package visibility
   - entry-selected Markdown fragment terminology
4. Inspect conflicts rather than accepting either side mechanically.
5. Search the final worktree for:

```text
sjmorig/moth
package_folders
active module root
active-root
active root
lib/ folder
lib/ package
```

Classify every match. Design references and implementation code may intentionally mention legacy `package_folders`. Public current-status claims must remain accurate.

Stop if the benchmark/output work changed documentation routing, generated-output ownership or the same source files materially. Reconcile ownership before continuing.

# Phase 1 - Reconcile the progress matrix

Update:

```text
docs/src/docs/progress/@page.moth
```

The matrix describes current implementation, not only accepted end state.

## 1.1 Entry activation terminology

Replace the current row titled:

```text
Active module roots and implicit start
```

with an entry-activation contract equivalent to:

```text
Entry-selected module roots and implicit start
```

The row must state:

- selected normal modules compile dormant compiler-synthesised `start` work
- entry assembly activates an entry-selected module's `start` exactly once
- imported roots expose public interfaces without activating root work
- API-only roots remain importable and emit no HTML page artefact
- `start` is build-system-owned and not user-importable or callable

Remove coverage wording that implies compilation itself executes root work.

Update the HTML-Wasm row and any other matrix wording that still says ordinary "active-root execution". Use entry-selected roots, dormant start work and entry activation.

## 1.2 Project-local package status

The current compiler supports both:

- accepted structural `+*.moth` support packages and the project-root facade
- legacy config-driven `package_folders` discovery, defaulting to `lib/`

The existing matrix incorrectly says no `package_folders` setting exists.

Update the Project-local packages row to `Partial` and state both facts. Suggested note:

> Structural `+*.moth` support packages and the optional project-root facade define the accepted package model. The current compiler also retains legacy config-driven `package_folders` discovery, defaulting to `lib/`. The queued Project Config migration removes that compatibility path.

Coverage should mention structural roots and legacy config-defined folders while both paths exist.

## 1.3 Project config status

Add or update one concise Project config model row.

It must state:

- current compiler support remains the transitional flat config shape
- current config parsing and validation are implemented
- grouped `project` and builder records are accepted deferred design
- `#Import`, `@project` and entry-local `config:` remain separately queued
- legacy `package_folders` remains current implementation drift
- the queued Project Config and recursive schemas plan owns replacement

Do not imply the grouped examples compile today.

## 1.4 Preserve Stage B drift rows

Do not remove the current Stage B implementation-gap notes for:

- source-authored return aliases
- source string `+`
- construction-origin string equality and map-key checks
- general full-match capture
- String relational patterns
- Moth Template implicit-scope precedence
- option payload equality inside choices
- nested-block `return!`
- block value-producing `if`
- stored named inserts
- Core Text scalar length
- Core Math finite-result validation

Only Stage B implementation commits remove those notes.

# Phase 2 - Finish source terminology and status cleanup

## 2.1 Active-root terminology

Search:

```text
docs/src/docs/**
docs/language-overview.md
docs/compiler-design-overview.md
docs/build-system-design.md
```

Replace source-facing claims that root work runs because a module is being compiled.

Use the accepted model:

1. compilation produces dormant root work
2. entry selection chooses an already compiled module
3. entry assembly activates its `start` exactly once
4. imports expose interfaces without activation

Do not blindly replace unrelated uses such as active builder, active config section or active tooling overlay.

## 2.2 Deferred config presentation

Verify:

```text
docs/src/docs/project-structure/project-config.mtf
docs/src/docs/project-structure/project-config-basic.mtf
docs/src/docs/project-structure/build-inputs.mtf
docs/src/docs/project-structure/entry-config.mtf
docs/src/docs/project-structure/project-package-facade.mtf
```

Required outcomes:

- grouped Project Config begins with an accepted-deferred note
- Basic does not present the grouped shape as current compiler support
- Build Inputs and Entry Config remain clearly deferred
- Project Package Facade remains accurately partial
- links point to `nyejames/moth` or a real public docs route

## 2.3 Legacy scaffold wording

Verify Getting Started says the current scaffold creates an empty legacy `lib/` placeholder and that it has no accepted package semantics.

Do not claim the scaffold merely may create it while `SCAFFOLD_DIRECTORIES` always includes `lib`.

Verify `--terse` is documented for both `check` and `tests`.

## 2.4 Support package visibility

Verify summary pages do not say a support package is visible everywhere in its owner's subtree.

The concise exact rule is:

- visible to its owner normal module
- visible to normal sibling modules and their descendants
- unavailable from its own private implementation subtree
- unavailable from another support package in the same scope

The dedicated Packages page remains the complete owner.

## 2.5 Repository-link audit

Search all documentation sources for:

```text
github.com/sjmorig/moth
```

Required result: zero matches.

Review every explicit GitHub URL touched by this migration. Prefer public docs routes where available. Otherwise use `https://github.com/nyejames/moth/blob/main/...`.

# Phase 3 - Update parity and migration state

## 3.1 Parity ledger

Update:

```text
docs/roadmap/plans/docs-language-migration-parity-ledger.md
```

Required rows:

- Project Config: documentation complete, implementation `Partial` or `Compiler drift`, grouped schema deferred and flat config current
- Project Layout: documentation complete, implementation partial while legacy `package_folders` and scaffold `lib/` remain
- Project-local packages: documentation complete, implementation partial while both structural and legacy discovery paths exist
- Template slots: partial until stored named inserts land in Stage B

Do not merge documentation parity and implementation status into one value.

## 3.2 Migration plan capsule

Do not update the validation SHA before the validated source and generated-doc commit exists.

After Phase 5:

- set `CURRENT_STAGE` to Stage B
- keep `NEXT_ACTION: B1 remove source-authored return aliases`
- record the exact clean commit that passed the full gate
- note any later metadata-only commit separately

Do not claim the branch HEAD was validated when the final metadata update came afterward without rerunning the relevant docs check.

# Phase 4 - Regenerate documentation

Run:

```sh
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
```

Retain the generated changes under `docs/release/**`.

Do not edit generated HTML manually.

Inspect the generated diff for accidental broad churn. Every generated change must trace to a source edit or compiler-generated formatting change already accepted by the task.

# Phase 5 - Automated and manual audits

## 5.1 Generated link and fragment audit

Run the existing generated-site link checker used during Stage A.

Required result:

```text
0 broken links
0 missing fragments
```

The audit must include local generated routes and fragments. Separately search source for stale external repository ownership because external links may be excluded from the generated checker.

## 5.2 H1 audit

Commit `267a6c6ae8192c73f015e5557ce9ff7ddac83667` established one H1 per generated page. Verify regeneration preserves it.

Required result:

```text
one H1 on every generated HTML page
```

Do not rework headings that already pass.

## 5.3 Manual route inspection

Inspect at minimum:

- `/docs/getting-started/`
- `/docs/project-structure/`
- `/docs/packages/`
- `/docs/markdown/`
- `/docs/progress/`
- `/docs/memory/`

Verify:

- Basic remains selected by default
- Advanced content remains complete
- structural package wording is accurate
- deferred config status is visible
- `lib/` is described only as current legacy scaffold behaviour
- entry activation wording is correct
- tables and code blocks render correctly
- pagers work
- dark mode and narrow layout remain usable on changed routes

# Phase 6 - Final validation and commits

The benchmark/output correction plan must be accepted first when it changes code or output ownership.

From a clean committed worktree run:

```sh
cargo fmt --check
just validate
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
```

Then rerun:

- generated link and fragment audit
- one-H1-per-page audit

Recommended commit sequence:

1. `docs: reconcile pre-B1 status and links`
   - source documentation
   - progress matrix
   - parity ledger
2. `docs: regenerate pre-B1 reference output`
   - generated `docs/release/**`
3. run the full gate on the clean second commit
4. `docs: record Stage B validation checkpoint`
   - migration-plan capsule only
5. rerun docs check after the metadata-only commit

Do not squash benchmark baselines or output-system implementation into these commits.

# Completion criteria

This plan is complete when:

- progress matrix matches current compiler behaviour
- grouped Project Config is labelled deferred
- legacy `package_folders` support is described as current drift, not absent
- no public docs claim `lib/` has accepted automatic package semantics
- no stale `sjmorig/moth` links remain
- entry activation terminology is consistent
- generated docs are rebuilt
- generated links and fragments have zero failures
- every generated page has one H1
- required routes were inspected
- `just validate` passes from a clean commit
- migration plan records the tested commit
- B1 is the next active action

# Required report

Report:

## Repository state

- starting commit
- benchmark/output correction commit
- final source commit
- generated-doc commit
- validation checkpoint
- branch and starting worktree state

## Corrections

- progress rows changed
- terminology matches changed
- deferred-status notes changed
- links changed
- parity-ledger rows changed

## Generated output

- files regenerated
- unexpected churn found or none
- manual routes inspected

## Validation

Give exact results for:

```text
cargo fmt --check
just validate
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
link and fragment audit
H1 audit
```

## Remaining uncertainty

List every unresolved:

- inaccurate status claim
- broken link or fragment
- generated route defect
- validation failure
- ownership conflict with benchmark/output or config work

Do not start B1 while any item remains.