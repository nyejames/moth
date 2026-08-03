# Moth agent rules

Resolve every relative path in this file from the current worktree root. Do not read project references from another worktree unless the user explicitly asks you to.

## Reading list

Before any Moth task, read:
- this file
- `docs/compiler-design-overview.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/memory-management/overview.mtf`

Before making or reviewing a non-trivial change, read:
- `docs/src/docs/codebase/style-guide/validation.mtf`

Read `docs/src/docs/codebase/style-guide/testing.mtf` when the task changes or reviews behavior, diagnostics, compiler stages, backend artifacts, tests, fixtures, or test infrastructure.

Read `docs/build-system-design.md` for build-system or project orchestration: Stage 0, config, imports, modules, packages, builders, tooling, link planning, backend project assembly, outputs, incremental builds and the dev server.

For memory, ownership, borrow checking, allocation, GC, drops, or runtime-handle work:
1. Read `docs/src/docs/codebase/memory-management/overview.mtf`
2. Use its task-reading guide
3. Read the selected memory leaf documents

For language syntax, semantics and user-visible behavior, read:
1. `docs/src/docs/codebase/language/overview.mtf`
2. Every relevant canonical unsuffixed Moth template file selected by that overview
3. `docs/src/docs/codebase/memory-management/overview.mtf` and its routed leaves when source behaviour touches access, copies, borrows, lifetimes, groups or ownership
4. Paired `-basic.mtf` files and `@page.moth` only when teaching, presentation or website structure is in scope

Before writing Moth code or changing tokenization, parsing, type checking,
semantics, diagnostics or lowering for a language feature, always read that
feature's relevant unsuffixed language reference. Do not infer the language
contract from examples, tests, compiler behaviour or a Basic teaching page.

Use:
- `docs/src/docs/progress/@page.moth` for current implementation status and coverage
- `docs/roadmap/roadmap.md` for sequencing, active plans, and genuinely deferred design
- `index.md` only as a file and module locator

The public unsuffixed Design Scope files under `docs/src/docs/design-scope/` own
accepted deferred implementation, open design questions and excluded language
boundaries. The progress matrix tracks implementation of accepted design only
and must not add open or outside-scope features as standalone rows.

## Instruction priority

1. The explicit user request for the current task
2. The most specific relevant design or standards document
3. This file
4. Existing implementation behavior

A narrow canonical design or standards document may refine a broader authority within its declared ownership area. Educational compiler-design pages explain concepts and implementation examples but cannot override the compiler overview, build-system design, language authorities, memory authorities or progress matrix.

`docs/compiler-design-overview.md` is the authority for compiler semantics and stage contracts. `docs/build-system-design.md` is the authority for project and build orchestration. The unsuffixed language references selected by `docs/src/docs/codebase/language/overview.mtf` own source syntax and observable language semantics. A plan that crosses compiler and build ownership must read both architecture documents. Roadmap plans cannot override these authorities.

Code may lag accepted design. When implementation conflicts with the relevant design document, call out the conflict rather than silently treating the code as authoritative.

The progress matrix answers what works today. It does not override accepted architecture or language semantics.

## Core working rules

- Prefer readability, modularity, correctness, and structured diagnostics over cleverness. Avoid complexity.
- Maintain strict boundaries between build-system, frontend, AST, HIR, analysis, project-builder, and backend responsibilities.
- Avoid user-input panics. User failures use structured diagnostics; panic paths are only for proven internal compiler invariants.
- Moth is pre-release. Do not preserve old APIs through compatibility wrappers, forwarding shims, parallel structs, or legacy entry points. No compatibility fallbacks.
- Prefer one current implementation path. Extend, consolidate, replace, or delete existing paths instead of adding parallel systems. 
- When an API shape changes, thread the new shape through the compiler and remove the old one. 
- Be strict about making root-cause fixes over patches. Never leave code that will need refactoring or cleaning up later.
- Write beautiful code that uses descriptive names, explicit control flow, narrow helpers, context structs, and concise WHAT/WHY comments.
- Remove dead code, obsolete helpers, stale comments, duplicate paths, and superseded fixtures as part of the owning change.
- Be strict about design drift, duplicated implementation paths, weak diagnostics, oversized modules, stale helpers, and stage-boundary leaks.
- Do not move shared logic into a broad utility module unless the behavior is genuinely shared and the owner remains clear.
- Do not claim work was validated by commands that were not run.
- Prefer data-oriented design over OOP patterns, especially when optimising code.

When creating temporary files for testing snippets of code or creating temporary artifacts that will be cleaned up before a commit, use the `/tmp` folder.

## Required workflow

Every non-trivial implementation plan must end with the Final audit below.

For multi-phase work, briefly re-check ownership, duplication, stale paths and
test gaps after each completed phase.

1. Identify and read the relevant documentation.
2. Inspect the current implementation and its existing owner.
3. Search for overlapping helpers, validators, lowering paths, diagnostics, tests, and legacy implementations.
4. Decide whether the task extends, consolidates, replaces, or removes an existing path.
5. Implement the smallest coherent slice without leaving transitional duplication.
6. Add or update tests according to `style-guide/testing.mtf` when behavior or internal invariants changed.
7. Review the progress matrix when support, rejection, backend coverage, or test coverage changed.
8. Apply the correct final gate from `style-guide/validation.mtf`.
9. Perform the final audit below.

If a user request changes accepted behavior, treat the request as authoritative for that task and update the relevant design/status documentation when documentation changes are in scope. Call out any implementation conflict explicitly.

## Duplication and abstraction policy

Be strict about avoiding duplicated logic. Prefer extending, consolidating, or replacing the existing owner of the behavior over adding a new module, system, or parallel path. Only add a new subsystem when the existing ownership is clearly wrong or the new behavior is genuinely separate.

Before adding a helper, pass, type, registry, validator, or module:
- check for an existing owner
- check adjacent stages and backend paths for near-duplicate logic
- prefer extending or restructuring the current owner
- extract shared code only when the behavior is genuinely identical and the abstraction has a clear home

When similar logic remains separate, state why the similarity is superficial or why sharing would blur ownership.

Actively look for duplicated:
- validation
- diagnostic construction
- type and coercion logic
- template handling
- control-flow lowering
- backend lowering
- test fixtures and assertions

## Testing

Follow `docs/src/docs/codebase/style-guide/testing.mtf`.

Key routing:
- prefer integration cases under `tests/cases/` for user-visible language behavior
- use focused unit tests only for subsystem-local invariants or side-table facts
  that integration output can't expose
- use backend-specific artifact assertions or contractual goldens for backend structure
- use one input with backend-specific expectations for cross-backend parity
- don't use benchmark fixtures as correctness coverage

## Validation

Always follow `docs/src/docs/codebase/style-guide/validation.mtf`.

If using the Moth compiler `check` command, prefer `--terse` for compact Moth error messages.

## Documentation policy

Do not modify documentation unless the user explicitly requests documentation
changes or explicitly approves them after they are identified. 

The progress matrix and `index.md` are exceptions. Update the matrix when implementation
status, rejection behavior, backend coverage, or test coverage changes. Do not
edit it for a pure refactor or prose-only correction. 

Update `index.md` whenever modules, files or folders are moved, renamed or have fundamentally changed behaviour.

If implementation work makes documentation inaccurate, report the affected files and required corrections as a separate follow-up. Do not edit generated files under `docs/release/**` directly, rebuild it through the compiler.

- Codebase design documents may describe accepted end-state architecture that has not fully landed.
- The progress matrix records current support, partial support, clean rejection, experimental paths, and coverage.
- The roadmap records sequencing, active plans, and genuinely deferred design.
- Update the progress matrix when current status changed. Do not make a meaningless matrix edit for a pure refactor or prose-only correction.
- Compiler semantic architecture belongs in `docs/compiler-design-overview.md`
- Build orchestration belongs in `docs/build-system-design.md`
- Keep memory, language-scope, testing and validation rules in their existing canonical references.

## Benchmarking

- Use `just bench-check` for non-recording performance evidence
- Use `just bench` only when intentionally recording benchmark history
- Keep raw profiling and benchmark data local
- Treat profiling as attribution evidence, not proof of correctness or improvement

## Compaction rules

When compacting: Completely forget project documentation from the reading list so it can be efficiently reloaded without duplication.

After context is compacted, reset or may be incomplete: Fully re-read this file and follow the `Reading list` at the top of this document.

Don't continue implementation from compressed memory alone.

## Final audit

Before reporting a non-trivial slice complete or reviewing changes, verify:
- the relevant style, compiler, memory, and language contracts are respected.
- stage and subsystem ownership remain clear.
- no duplicated, legacy or obsolete implementation path remains.
- there is no unnecessary indirection, weak diagnostics, poor comments, or missing test coverage.
- there are no abstractions that are too broad, too early, or placed in the wrong layer
- diagnostics use the correct lane and preserve useful source context.
- tests protect behavior or real internal invariants rather than implementation accidents.
- the progress matrix accurately reflects changed support.
- documentation and comments name the current owner and behavior.
- the correct validation path was run.
- the final report states exactly what was and was not validated.
