# Built-in testing and test modules

## Status

- Status: Queued. Test-module architecture and source-separation direction accepted. Remaining execution and assertion contracts are implementation gates, not assumed delivered features.
- Current slice: None.
- Blockers: Delivered compiler-owned `$export` syntax and public-interface validation, final module/support-package visibility machinery, and an executable host suitable for the selected test targets.
- Next action: At activation, settle the bounded decisions under "Required design gates", update canonical language and build references, then implement in ordered slices.

## Purpose

Provide built-in Moth tests with `$test` on test functions and `moth test` as the project-level command. Make small regression tests easy, retain explicit support for module-local tests and external integration tests, and preserve production/test separation by construction.

Test code lives only in reserved `tests/` directories. It may depend on permitted production code and test-only support packages. Production source discovery, binding, interfaces, link facts and artefacts never depend on test code. Do not add an alternate language for tests, a second compiler pipeline, a macro framework or implicit global test helpers.

This is a late sequenced roadmap item. It consumes the delivered module/export system rather than implementing transitional test export or support semantics ahead of it. This plan records accepted direction and explicit future gates, not current compiler support.

## Prerequisites

Before activation, require:

- One final compiler-owned `$export` directive syntax for ordinary module and support-package surfaces, including validated re-exports and exported type/effect/lifetime contracts.
- Stable module root, support-package, canonical identity, collision and private/public binding rules.
- Stable compiler module service, canonical/test tooling source lanes, callable validation roots and backend-neutral link facts.
- A supported execution host and target contract for the first `moth test` slice. Other builders/targets may report unsupported capability rather than silently changing the code under test.
- The current `assert` intrinsic and recoverable `Error!` contracts, retained source spans and structured runtime assertion identity.
- Separation from the compiler repository's existing plural `moth tests` integration-fixture command. That internal harness is not the new user-authored test runner.

## 1. Accepted test source and module model

A `tests/` directory immediately inside a normal production module directory is its reserved test module. A `tests/` directory alongside project `config.moth` is the project test module. Both are test-only and excluded from ordinary build, package and runtime source sets. Treat the project-level root as a test consumer, not the private-test child of every production module.

Only the specific directories above gain test-module meaning. Ordinary descendant folders under a `tests/` module may organise source and fixtures according to normal owned-file rules. A `tests/` directory elsewhere is not a new implicit module solely because of its basename. Reserve `tests` as a module-root/path identity so ordinary source roots, module directories and source kinds cannot claim conflicting `tests` identities in those scopes.

Example:

```text
project/
  config.moth
  +package.moth                   # Optional external package facade
  tests/
    +shared.moth                  # Optional test export root: @tests
    package_tests.moth
  src/
    parser/
      @parser.moth                # Normal production module: @parser
      lexer.moth
      tests/
        +helpers.moth             # Optional test export root: @parser/tests
        lexer_tests.moth
      expressions/
        @expressions.moth         # Child module: @parser/expressions
        tests/
          expression_tests.moth   # No + root required
```

Every recognised `tests/` is one test semantic/visibility unit, including when it has no `+` file. A `+*.moth` in that directory is its optional export facade/root, not an additional test module. Enforce at most one test-root `+` file and reuse normal file identity and duplicate-root diagnostics. Without a `+` file, local tests and private helpers still compile, but that test module has no public test-support interface.

Test files must be selected only by test-specific tooling. Their ordinary declarations, helper types, constants, imports and generated functions are test-only, including without a `$test` annotation. No top-level executable work, implicit `start`, pages or runtime fragments in a test root. Neither helper discovery nor compilation runs a test.

Test functions are explicitly annotated with `$test`; helper functions without the modifier run only when called. Do not require runnable test functions to be `$export`ed. `$test` is a declaration modifier on eligible functions only and is illegal in production files or unsupported declaration contexts.

The previously queued idea of `$test` as a purpose directive selecting normal-root top-level runtime work is superseded. Test entry activation belongs to selected annotated functions, not an implicit normal-root lifecycle.

## 2. Test support package identity and exports

Test-module export roots reuse the ordinary `+` support-package implementation and the delivered compiler-owned `$export` machinery. The `+` filename suffix remains cosmetic. The test package identity is the canonical owning production module path followed by `/tests`, with the project-root identity `@tests`.

- Project test root: `@tests`.
- Parser test root: `@parser/tests`.
- Parser expressions test root: `@parser/expressions/tests`.

An export root uses exactly the same final `$export` form as normal Moth modules. Any authorised exportable declaration category is eligible under the ordinary public-surface checks. Do not add `$test_helper`, `$test_export` or special implicit exports.

Visibility follows the *downward scoped support-package direction* across the owning production module hierarchy:

- `@tests` exports are visible to all test modules inside the same project/package compilation boundary.
- `@parser/tests` exports are visible to tests belonging to `@parser` and its descendant normal modules.
- Tests belonging to unrelated sibling modules or ancestor production modules cannot bind `@parser/tests`.
- A test module may use its own private declarations under normal same-module source visibility, even if it has no export root.
- Test dependencies are explicit. No helpers are injected into every test source.
- Test packages cannot be consumed by ordinary production modules, the external project facade, or another separately compiled dependency's project graph.

The paths above are logical test-package identities resolved by the compiler. They do not authorise ordinary physical parent-path imports, private traversal across unrelated production modules, nearest-name fallback or global namespace shadowing. Apply normal canonical name/case collision diagnostics in the relevant test visibility scope.

Reuse existing export bindings, declaration origins, type projection, visibility validation, public-interface fingerprints, provider scheduling, borrow summaries and generated-function linking. Specialise only root classification, allowed consumers and test-lane identity. Do not implement a parallel test export registry or interface type.

## 3. Production/test isolation and private access

The dependency direction is one-way:

- Test modules can bind permitted production interfaces and permitted test-only support packages.
- Tests owned by a normal module can also inspect that same production module's private declarations through a compiler-controlled test view.
- Project-root tests use the external project package facade when present, other normally permitted public dependencies and builder-supported artefact/runtime testing facilities. Project-root placement does not grant blanket private access to all production modules.
- Production modules and production package facades cannot name or depend on test declarations or test-only packages, even during `moth test`.

The private test view is restricted to the immediately owning production module's declarations. It exists because a separate child test module would otherwise be unable to import its parent under the ordinary no-parent-dependency rule. Tests can bind that owner's normally nameable private functions, types and constants without making them public exports. This is the sole special private production access: descendant modules expose only their ordinary public interfaces, and ancestor, sibling and unrelated module privacy remains unchanged. Tests must still name and bind declarations through normal source language mechanisms. The test view is not a second public production interface and grants no private access transitively to imported test helpers or their consumers.

Production compilation and published interfaces remain identical with or without selected test sources. Adding or changing tests cannot resolve a production name, supply trait evidence, alter production declaration ordering, modify exported summaries, add a runtime import, change the production resource graph or change an ordinary production artefact. Test-only generated work and physical variants stay in the test compilation/output domain.

Support exports must not expose private production types or facts that the importing test consumer is not otherwise authorised to name. Preserve the normal export validation contract until an explicitly approved test-only surface rule exists. Do not grant transitive private access merely by re-exporting a test helper.

Any test source importing another test source or support root must remain within the admitted test visibility graph. Reject cycles using the normal provider graph validator rather than tolerating test-only cycles.

## 4. Discovery and compiler/build ownership

Stage 0 recognises reserved test roots, records their canonical ownership and builds test-only source/provider selections when test tooling requests them. Ordinary `build` and `dev` ignore test-only executable declarations and dependencies. `check` and editor tooling may validate test sources in a separately classified test context, but cannot publish their facts into ordinary production artefacts.

The builder selects the test environment and permitted host capabilities. The compiler retains sole ownership of directive placement, test declaration signatures, binding, typing, HIR, borrowing, lifetime legality, source locations and test entry metadata. The build system selects test roots, dependency order, test entries, physical targets and execution/reporting. The runner must not reconstruct the test inventory from generated JavaScript/Wasm or parse source itself.

Compile reusable code and interfaces once per applicable compilation boundary, then select annotated functions as callable test roots. Preserve compiler-generated stable test identity from project/module/source/function facts rather than runtime discovery order. Where targeted test views require additional compilation, share canonical production artefacts instead of mutating or recompiling their published state.

Test discovery must be deterministic, independent of filesystem enumeration and parallel scheduling. Changing an unannotated helper must still invalidate tests which depend on it. A change to a private production body may require re-execution even when its public-interface fingerprint is unchanged.

## 5. Runner V1

`moth test` is the user-facing command. It must distinguish compile diagnostics, test assertion failure, an unexpected returned `Error`, runtime/host fault, timeout and runner infrastructure failure. One failed test must not prevent independent tests from executing.

Initial V1 requirements:

- Discover all `$test` entries in selected test modules and show canonical names and source locations.
- Run the suite or a focused subset, with a list-only mode and an explicit no-match outcome for authored filters.
- Reuse the selected builder's normal compiler and target rules. An unsupported execution target is diagnosed rather than replaced by silently different semantics.
- Isolate each test invocation's mutable runtime state sufficiently that a fatal assertion, unexpected host fault or timeout cannot corrupt later tests or the runner.
- Bound test execution and captured output, clean up owned resources even after failed invocations, and report structured results useful to CI and editors.
- Preserve deterministic test identities/report ordering independent of execution parallelism. Save enough environment, target and input context for reproducible local reruns.
- Require no testing framework dependency for simple tests.

Do not make test functions into application `start` roots or manufacture page routes for them. Builder-specific tests, such as real HTML/browser tests, extend the execution capability while using the same test selection and result schema.

The default V1 host and exact command flags are design gates, not settled by this plan.

## 6. Assertion and recoverable-failure policy

Retain the existing `assert(condition, message?)` semantics: assertion failure is deliberate, unrecoverable and enabled in ordinary release builds. An assertion in a test produces a failed invocation that the outer runner observes. The runner identifies compiler-owned assertion failure facts, not message text, and continues independent tests after isolation.

Use ordinary `Error!` and `catch` for expected recoverable application failures. Do not redefine `assert` to sometimes return an error or silently become recoverable inside `moth test`. A possible convenience guard such as `ensure` is a separate language/API choice, not a prerequisite for this plan.

Optional diagnostic instrumentation in production source must remain strictly observational and never depend on test-only helpers. It cannot make production semantically valid only in a test build, silently change public failure contracts or bypass borrow/lifetime validation.

Before implementing any removable debug/test assertion, settle its exact syntax, build/profile activation, typing of inactive checks, evaluation/side-effect constraints and lowering. Keep test instrumentation separate from release optimisation so an optimised test may retain test checks while an ordinary release artefact does not.

Enhanced comparison reporting and a test-only nonfatal `check` operation are candidates, not accepted V1 syntax. Reported nonfatal failures, if introduced, must leave the test failed even when execution continues. Do not add a second panic mechanism.

## 7. Required design gates before implementation

Resolve the following only after their actual compiler/export/host prerequisites are delivered. Record the decisions in canonical documentation before implementing the dependent slice.

1. **`$export` attachment and test root syntax.** Consume the final regular export contract. Confirm whether an optional `+` file acts as the test module root directly or as a compiler-created root facade sharing the same semantic identity. It must never create two overlapping modules in one `tests/` directory.
2. **Owning-module private test view implementation.** Preserve the accepted one-owner exception without adding descendant, ancestor or sibling private access. Choose the minimal producer-owned view for that owner's private functions, types and summaries, and specify generic materialisation and diagnostics without publishing private declarations to ordinary consumers.
3. **Test call signatures.** Specify zero-argument/zero-success entry policy, whether built-in `Error!` is allowed, handling of custom typed errors, generic entries, methods and any future fixture/context parameter.
4. **Test imports and source selection.** Finalise exact resolution of `@module_path/tests`, how ordinary clauses bind local test files, treatment of nested organisational directories, duplicate names, symbolic links, project boundaries and optional root facades.
5. **Execution host and isolation.** Choose the initial supported builder/target, per-test execution boundary, timeout/cleanup policy, output capture, runner result protocol and deterministic reporting. Do not make host faults into recoverable Moth errors.
6. **Debug/test checks.** Decide whether extra production instrumentation is needed for V1, and define its source form and active/removed semantics without modifying ordinary `assert`.
7. **Integration contract.** Define what a project-root test can execute without an external facade, how it targets a built project and which builder capabilities are required. Avoid introducing a second test-only build system.

These gates do not reopen the accepted directory structure, one-way dependency direction, scoped test support visibility or `$export` reuse.

## 8. Delivery slices

### Slice 0: Final contracts and tests

- [ ] Confirm the delivered `$export` syntax and module root/name invariants.
- [ ] Settle the required design gates above and update canonical module, directives, project-structure and compiler/build references before coding.
- [ ] Write the V1 test runner contract and fixture-based acceptance scenarios.
- [ ] Identify existing test/discovery and assertion machinery available for reuse. Do not repurpose `moth tests`.

### Slice 1: Test source classification

- [ ] Reserve legal `tests/` positions and namespace identities with exact collision diagnostics.
- [ ] Build test-only root/source inventory without adding test source to production graphs or regular check-only orphan units.
- [ ] Ensure a `tests/` without `+` still owns runnable test sources.
- [ ] Validate no top-level test runtime work or page activation.

### Slice 2: Exports, imports and private test view

- [ ] Reuse ordinary `+` and `$export` machinery for optional test support roots and interfaces.
- [ ] Bind `@tests` and scoped `@module_path/tests` through the normal import syntax and restricted test visibility domain.
- [ ] Supply owning-module private access only in the test compilation context.
- [ ] Reject production-to-test imports, unrelated sibling test imports, illegal re-exports, collisions and provider cycles.
- [ ] Prove no production interface, reachability or artefact changes when tests are added.

### Slice 3: `$test` discovery and compilation

- [ ] Validate the accepted `$test` function forms and placement through the directive registry.
- [ ] Discover stable callable test entries without implicit `start` or runtime registration.
- [ ] Compile and validate test functions/helpers through ordinary HIR, generated specialisation, borrow/lifetime and target checks.
- [ ] Preserve source spans and canonical test identities across failures, caching and parallel schedules.

### Slice 4: Runner and diagnostics

- [ ] Implement `moth test` selection, focused execution, list mode, explicit empty-filter behavior and structured reports.
- [ ] Integrate the supported host with per-test failure isolation, timeouts, bounded output and owned cleanup.
- [ ] Distinguish assert failure, returned `Error`, unexpected runtime faults, compiler diagnostics and infrastructure failures.
- [ ] Cover project-root integration entry selection and backend capability errors.
- [ ] Verify source/artefact freshness and no production output changes.

### Slice 5: Optional developer experience and later extensions

- [ ] Add watch/failure rerun and richer assertion-value reporting only after the runner contract works.
- [ ] Design parameterised tests and reusable fixtures without adding implicit global fixture state.
- [ ] Evaluate debug/test-only checks as a separate accepted source contract.
- [ ] Defer property-based testing and shrinking, coverage-guided fuzzing, snapshots, documentation tests, browser automation, coverage and mutation testing to subsequent scoped plans or package features. Do not prebuild extensibility layers for them.

## 9. Required acceptance coverage

Exercise real directory projects and complete compiler/build services, with focused stage-local tests only for hidden invariants:

- A valid project with only local `tests/` and no `+` root; all `$test` entries discovered, helpers not independently executed.
- A project-root `@tests` support facade visible to multiple child test modules; a scoped `@parser/tests` facade visible to parser descendants but not parser siblings or ancestors.
- Non-exported helper use within one test module, explicit export binding across permitted scopes, normal `$export` type/re-export validation and cosmetic `+` filename changes preserving package identity.
- An owning-module test binds its owner's private functions, types and constants without `$export` and without changing the production interface. The same test may bind a direct child module's public interface but not the child's private declarations. Parent, sibling and unrelated private imports fail. Project-root tests cannot access arbitrary private internals.
- A regular build before/after introducing tests is semantically and artefact-identical; tests cannot repair production name errors or add production dependencies, traits, resources or runtime requirements.
- Reserved `tests` name conflicts, duplicate `+` roots, case-only path collisions, invalid nested placement, symlink escape, ambiguous module aliases and deterministic diagnostics.
- Test-only provider cycles rejected; dependency package tests invisible to the importing project's test namespace.
- Valid/invalid `$test` signatures and placement, compile errors in helpers, test source location preservation and source-order-independent discovery.
- Fatal assertion failure stops only that test execution; typed returned errors, runtime traps and host/infrastructure failures remain distinguishable; timeouts always bound the test.
- Debug/release test and ordinary release behaviour match the eventually accepted assertion/instrumentation policy.
- Test filters, zero-match reporting, reruns and incremental invalidation after private production or test-only helper edits.
- At least one project-level integration case against a builder-produced artefact, with an explicit capability error for unsupported test hosts.

Acceptance requires real public `moth test` invocations, target-appropriate runtime coverage and the active validation guide's final gates. No test-only shortcut may bypass the ordinary module compiler, public-interface validation, mandatory borrow/lifetime validation or target validation.

## 10. Documentation and retirement

At delivery, update canonical test source rules, module and support-package visibility, general directives, project structure, package exports, assertion semantics and the progress matrices. Keep the user-facing cheatsheet focused on the final supported language surface.

Retire the old normal-root `$test` purpose proposal as superseded by the function-entry design. Avoid duplicating this plan's long-term semantic contracts in roadmap notes. Once delivered, move permanent rules into their canonical authorities and delete this plan and its roadmap entry in the completion commit, according to roadmap policy.
