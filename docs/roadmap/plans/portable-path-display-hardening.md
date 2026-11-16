# Portable Path Display Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make host-path presentation reliable across platforms, eliminate duplicate diagnostic conversions and prevent lossy display text from leaking into filesystem identity or URL handling.

**Architecture:** Give the existing `portable_path_text` conversion a specific `portable_path_display` module and add `DiagnosticStringBuilder::intern_path(&Path)`. Migrate display-only call sites and add exact diagnostic tests. Correct two adjacent native-path identity/safety risks and two URL boundaries in their existing owners, without creating a general path framework.

**Tech Stack:** Rust 2024, current stable toolchain, existing `moth` and `xtask`, no dependencies or public APIs added.

**Spec:** The design contract below, governed by `docs/compiler-data-layout-design.md`, `docs/compiler-design-overview.md` and `docs/build-system-design.md`.

## Global Constraints

- Read `AGENTS.md`, the complete code style guide, applicable testing and validation sections and routed compiler/build/data-layout authorities before editing. Follow their ownership and Slice review rules.
- Scope is compiler, build system, projects, builders and their tests. Inspect CLI, extracted crates, tooling and benchmarks for boundary leaks, but change them only for a proven path-presentation defect within their supported contract.
- `Path`/`PathBuf` and native `OsStr` remain authoritative for IO, canonicalization, symlinks, containment, cleanup and physical identity. Moth `PathId`, source logical paths, output-relative identities, manifest spellings and URLs keep their distinct owners.
- `portable_path_text` is **lossy human-readable text**, never a path key, security check, URL encoder, filesystem argument or portable on-disk identity. Source diagnostic paths keep their actual provenance and typed facts. Use no general `Path` wrapper or `fs::canonicalize` facade.
- Use one formatter and one diagnostic path interning operation. Remove obsolete functions, forwarding aliases and stale imports as their callers migrate. Preserve stable diagnostic codes, reasons, labels, source spans and capture lifetimes.
- Separate changes by owning subsystem. Each task ends with a focused test run, an independent review against the code style guide, a scan for adjacent violations and a coherent commit. Tests for changed behaviour should fail against the old implementation before the fix. A pure move may use passing before/after tests.
- Keep a worktree-local inventory under `./tmp/portable-path-display-audit.md`. It is review evidence, not a second permanent architecture document. Escalate a larger unrelated security or identity issue explicitly rather than hiding it behind display conversion.

## Review Focus

The following inputs must be covered by the named task's exact assertions:

1. **Windows verbatim drive and UNC paths (Tasks 1, 3, 8):** `\\?\C:\...` and `\\?\UNC\server\share\...` render as ordinary portable text, while containment and cleanup operate on correct native identities.
2. **Non-UTF-8 and literal backslash names (Tasks 1, 7):** display may be lossy, but distinct native paths must never silently collapse into one identity or bypass source-name rejection.
3. **Source-root diagnostic provenance (Tasks 2-4):** asserted `StringId` facts identify the actual compiled root and authored-versus-resolved folder correctly, not merely a matching code or a formatted path reconstructed in the test.
4. **Reserved URL characters (Task 6):** file links and browser/JS imports containing spaces, `%`, `#`, `?` or Unicode resolve to their intended target and do not use HTML escaping as URL encoding.
5. **Regression enforcement (Tasks 9-10):** obvious direct raw diagnostic path interning is caught without banning legitimate native operations, URL encoders or typed identity conversions.

---

## Ownership map

- `src/compiler_frontend/utilities/portable_path_display.rs` (new): the **only** host-path-to-readable-text formatter, still named `portable_path_text(path: impl AsRef<Path>) -> String`. This is distinct from `compiler_frontend/paths` and source path semantics.
- `src/compiler_frontend/utilities/basic.rs`: retain the existing Windows verbatim-prefix normalization needed by both rendering and output comparison. Give it a precise name/comment instead of treating it as general canonicalization.
- `src/compiler_frontend/compiler_messages/diagnostics/diagnostic_string_builder.rs`: `pub(crate) fn intern_path(&mut self, path: &Path) -> StringId`, implemented through `self.get_or_intern(portable_path_text(path))`. Existing text interning remains unchanged.
- `src/compiler_frontend/compiler_messages/render/paths.rs`: existing relative diagnostic display and, if needed, a narrow file-URL formatter. HTML link escaping belongs at the render boundary.
- `src/build_system/output/output_path.rs`, `output/policy.rs` and `output/manifest.rs`: validated portable output spellings, physical containment and deletion safety remain build-owned.
- `src/projects/html_project/moth_template/input.rs`: native file identity and user-supplied in-memory display names are different inputs. Do not normalize both with lossy component strings.
- `src/projects/html_project/external_js/runtime_glue/paths.rs`: generated browser import URLs reuse the build system's existing `percent_encode_url_segment` where legal. No display formatter is involved.

Initial known fixes: Stage 0 `project_structure_diagnostics::path_id()` currently interns `path.display().to_string()`. HTML diagnostics duplicate a `path_id()` wrapper. Configuration validation has four `intern(&...to_string_lossy())` path facts, output manifest has one and a project-entry diagnostic interns rendered path text directly. The failing build-orchestration assertion compares `Path::new(rendered_diagnostic_text)` to a canonical native `Path`, which is invalid on Windows. A few more sites use display conversion as lookup or logical-path text and need explicit domain classification.

## Task 0: Read-only whole-tree inventory and review

**Files:** `AGENTS.md`, routed authorities, Rust under `src/`, relevant `crates/` and `xtask/src/`.

- [ ] Confirm branch, clean working tree, current main comparison and existing tests. Inventory `.display()`, `.to_string_lossy()`, `.to_str()`, `portable_path_text()`, `normalize_path()`, manual slash replacement, `canonicalize`, string-interning of path values and `Path::new()` around diagnostic text. Use `rg -n` across the whole tree, then trace each suspicious value through its producer and consumer. Search `format!`, `write!`, `Debug(Path)`, URI links, artifact string keys and retained `PathBuf` fields. Avoid treating every generic `.to_string()` as a path.
- [ ] Classify each relevant occurrence as `DISPLAY`, `DIAGNOSTIC_PATH_FACT`, `NATIVE_IDENTITY`, `LOGICAL_IDENTITY`, `PORTABLE_RELATIVE`, `URL`, `INPUT_CONVERSION` or `TEST_ONLY`. Record exact files, path origin, representation contract, planned action and owner test. Group repeated instances without losing file coverage. Every production `.display()` and existing `portable_path_text()` use needs a reason to stay or migrate.
- [ ] Inspect typed diagnostic family constructors, `StringTable`/`DiagnosticStringBuilder` callers and renderers for indirect path facts and repeated conversion wrappers. Check direct `InfrastructureFailure::message()` consumers before proposing removal of path text from messages.
- [ ] Investigate specifically: `output/manifest.rs::is_dangerous_system_path`, `output/policy.rs::canonical_output_root_for_identity`, `moth_template/input.rs::normalize_path_for_identity`, `external_js/runtime_glue/paths.rs::relative_url_path`, `compiler_messages/render/dev_server.rs` source hyperlinks, `module_namespace.rs::extensionless_portable_path`, dev-server artifact keys, `source/database.rs` sorting, provider-import logical paths and CLI scaffold `cd` output. Identify actual defects rather than using display conversion to alter their identities.
- [ ] List affected exact-fact and rendered-error tests, current redundancies and missing Windows/Unix adversarial cases. Record baseline test commands/results.
- [ ] **Review gate:** independent reviewer verifies completeness, path domains, cross-stage ownership and any high-risk finding. Resolve unclear classifications and specify a bounded owner fix or explicit blocker before proceeding. No production edits until this gate closes.

## Task 1: Extract one portable path display implementation

**Files:** create `src/compiler_frontend/utilities/portable_path_display.rs` and `src/compiler_frontend/utilities/tests/portable_path_display_tests.rs`. Modify `utilities/{basic.rs,mod.rs,tests/mod.rs}`, all existing `basic::portable_path_text` imports and any normalization callers whose name changes. Remove `utilities/tests/basic_tests.rs` if empty.

**Interface:** `pub(crate) fn portable_path_text(path: impl AsRef<Path>) -> String`.

- [ ] Keep explicit expected-text tests for `src/docs/main.moth`, synthetic backslash-separated input and Windows `\\?\C:\...` plus UNC. Test non-UTF-8 lossiness on Unix. Test actual normalizer output independently of the formatter to avoid two equally wrong helpers masking each other.
- [ ] Run existing focused utility tests. Move the implementation unchanged where possible; correct any proven UNC/verbatim rendering defect with a separate failing test and minimal patch. Preserve native `PathBuf` values for IO.
- [ ] Retain Windows comparison normalization as an internal helper in `basic.rs`, with a precise name such as `normalize_windows_verbatim_path` and comment limiting it to comparison/rendering, not general canonicalization. Update its few known users in `output/policy.rs` and diagnostic render paths without changing output identity policy.
- [ ] Remove `basic::portable_path_text`, stale tests/imports and any redundant pre-normalization (the dev-server renderer currently normalizes its root before calling a helper that normalizes it again).
- [ ] Verify with `cargo test -p moth --lib portable_path`, `cargo test -p moth --lib policy_tests`, `cargo check -p moth` and `just fmt-check`.
- [ ] **Review gate:** inspect platform prefix handling, allocation, visibility, module responsibility, duplicated transformations and exact-path identity consumers. Re-scan the old import and helper names. Commit.

## Task 2: Introduce diagnostic path interning

**Files:** `src/compiler_frontend/compiler_messages/diagnostics/diagnostic_string_builder.rs` and a focused test under `diagnostics/tests/`, registered by its `mod.rs`.

**Interface:** `pub(crate) fn intern_path(&mut self, path: &Path) -> StringId`.

- [ ] Write tests asserting literal expected resolved strings (`"src/docs/main.moth"`, `"C:/work/main.moth"` on Windows), repeated-input ID reuse and unchanged `intern(&str)` semantics. Do not derive every expected string using the same formatter under test.
- [ ] Run the filtered test to observe the missing method. Implement it with `get_or_intern(portable_path_text(path))`, consuming the new owned string once and using the command-owned table.
- [ ] Run `cargo test -p moth --lib diagnostic_string_builder` (adjust filter to the new test name), `cargo check -p moth` and formatting.
- [ ] **Review gate:** inspect compact string ownership, table handoff and absence of added caches, wrapper types or parallel interning domains. Search for other path interning API candidates. Commit.

## Task 3: Correct Stage 0 project-structure diagnostic paths

**Files:** `src/build_system/create_project_modules/{project_structure_diagnostics.rs,source_tree_index.rs,mod.rs}` and discovered direct users, `src/build_system/tests/{build_orchestration_tests.rs,create_project_modules_source_tree_index_tests.rs}`.

- [ ] Tighten the no-entry assertion to compare the resolved `StringId` **as text** to `portable_path_text(expected_root)`. Preserve `MOTH-CONFIG-0001`, the exact `NoRootModuleEntries` reason and the actual frontend-compiled root. Add a platform-neutral diagnostic fact test using synthetic backslashes so the old raw `.display()` path is detectably wrong on Unix too. Assert exact collision and root facts.
- [ ] Run `cargo test -p moth --lib build_orchestration_tests` and `cargo test -p moth --lib create_project_modules_source_tree_index_tests`. Show the new fact test failing before the correction.
- [ ] Replace all Stage 0 `path_id` calls with `diagnostic_strings.intern_path(path)` and delete `project_structure_diagnostics::path_id`. Convert the entry-extension diagnostic's direct rendered-string plus raw `StringTable::intern` to a short-lived `DiagnosticStringBuilder::intern_path`. Preserve batch capture and string-table handoff.
- [ ] Rerun filtered tests, `cargo check -p moth` and check exact codes/facts after freeze. Review all adjacent Stage 0 diagnostic constructors for new bypasses.
- [ ] **Review gate:** code/style and correctness review of the whole Stage 0 diagnostic path. Confirm that no local alias or unnecessary intermediate owned string remains. Commit.

## Task 4: Consolidate HTML, config and manifest diagnostic path facts

**Files:** `src/projects/html_project/diagnostics.rs`, `src/build_system/project_config/validation.rs`, `src/build_system/output/manifest.rs`, `src/build_system/tests/{build_orchestration_tests.rs,build_cleanup_tests.rs}`, `src/build_system/output/tests/manifest_tests.rs` and relevant HTML diagnostic tests.

- [ ] Add exact typed-fact tests for HTML duplicate/reserved output destinations, invalid output folder, dev/release folder conflict and manifest ownership conflict. Include Windows canonical versus authored-relative spellings where applicable, and assert error codes/labels as well as the path text.
- [ ] Run the affected tests before migrating, including `cargo test -p moth --lib build_orchestration_tests` and the focused HTML/output tests identified in Task 0.
- [ ] Replace HTML's existing portable `path_id` wrapper with direct `intern_path`, deleting the helper. Replace the four configuration `to_string_lossy` interns and the manifest-owner-conflict intern. Migrate all other confirmed path-bearing diagnostic facts from the inventory, keeping ordinary names on `intern`.
- [ ] Check that typed facts retain authored versus resolved source spelling, original spans and frozen string-table IDs. Leave manifest serialization and output path security predicates untouched in this task.
- [ ] **Review gate:** re-scan diagnostic producers for direct `.display()`, `to_string_lossy()`, `StringTable::intern` and duplicate helpers. Run focused tests and `cargo check -p moth`, then commit.

## Task 5: Consolidate human-facing filesystem failure messages

**Files:** display-only sites in `compiler_frontend/{compiler_messages/compiler_errors.rs,source/database.rs,paths/path_resolution.rs,analysis/borrow_checker/boracle/service.rs}`, `build_system/create_project_modules/`, `build_system/output/{writer.rs,manifest.rs,orchestrator.rs}`, `projects/{build_command.rs,dev_server/,html_project/}`, `builder/`, `builder_surface/`. Primary test owners: `compiler_frontend/compiler_messages/render/tests/infrastructure_tests.rs`, `build_system/tests/build_infrastructure_tests.rs`, `build_system/output/tests/{writer_tests.rs,manifest_tests.rs}`, `projects/tests/build_command_tests.rs` and `projects/dev_server/tests/error_page_tests.rs`.

- [ ] Divide this task into independently committed owner groups: frontend/rendering, Stage 0/discovery, output writer/manifest and projects/builders. For **each** group, choose exact human-facing failure-text assertions and run them before changes.
- [ ] Convert only native host paths appearing in user/operational prose to `portable_path_text(path)` at presentation boundaries. Keep existing validated portable-relative output spellings and structured `InfrastructureFailure::host_path` as their native/typed representations. Preserve `Debug` formatting when deliberately exposing an unrepresentable filename.
- [ ] Audit repeated text such as a `filesystem(stage, path, format!("... '{path}' ..."))` plus a separately rendered `host_path`. Remove repetition only where all direct `.message()` consumers and terminal/dev-server views retain the essential context; don't silently remove a second, distinct path from an explanation. Prefer simplifying a caller over introducing another formatting wrapper.
- [ ] Migrate ad hoc test-only slash replacement and lossy conversions **only** when asserting displayed host text (`projects/tests/check_tests.rs`, package-materialisation tests and Stage 0 fixtures). Keep `to_str()` at genuine UTF-8 input gates.
- [ ] After **each** owner group run focused tests, `cargo check -p moth`, a fresh search for direct path display in that owner and an independent style/correctness review. Remove stale imports and unnecessary `format!` temporaries. Commit each owner group separately.

## Task 6: Keep URL paths separate from displayed paths

**Files:** `src/compiler_frontend/compiler_messages/render/{dev_server.rs,paths.rs}`, renderer tests, `src/projects/dev_server/tests/error_page_tests.rs`, `src/projects/html_project/external_js/runtime_glue/paths.rs`, `external_js/tests/runtime_glue_tests.rs`.

- [ ] File-link subphase: write an exact HTML `href` test for source names with spaces, `%`, `#`, `?` and Unicode, plus Windows drive/UNC spellings. The present `href="file://{portable_path_text(path)}"` and HTML escaping are insufficient URL serialization. Run tests and confirm they detect the defect.
- [ ] Implement a small render-owned host-file URL operation (for example `file_href_from_path(&Path) -> Option<String>` in `render/paths.rs`). Percent-encode URL path bytes and produce correct file URL authority/drive structure, then HTML-escape the completed attribute. For non-UTF-8 paths that cannot be faithfully linked, show portable visible text without a misleading hyperlink. Keep `portable_path_text` for the link label only. Do not make the frontend depend on build-system URL helpers.
- [ ] Run `cargo test -p moth --lib error_page_tests` and renderer tests. **Review gate:** audit HTML injection, URI encoding, UNC/drive handling and rendering fallbacks. Commit.
- [ ] Runtime-import subphase: add tests that `relative_url_path` produces encoded browser import specifiers for validated output components containing spaces, `#`, `%`, Unicode and `?`, preserving structural `./` and `../`. Replace component `to_string_lossy()` with validated UTF-8 conversion and reuse `build_system::output::percent_encode_url_segment` (already used by structural URL rendering). Reject or propagate nonportable input through the existing output-path validation contract rather than silently replacing bytes.
- [ ] Run `cargo test -p moth --lib runtime_glue_tests` and relevant HTML builder tests. **Review gate:** confirm module import paths match emitted artifact names and no display helper is used as an encoder. Commit.

## Task 7: Preserve native and in-memory MTF input identities

**Files:** `src/projects/html_project/moth_template/input.rs`, `src/projects/html_project/moth_template/tests/mod.rs`.

- [ ] Add Unix-only non-UTF-8 file-name tests and distinct filenames containing literal `\`. Confirm directory and `Files` inputs cannot collapse identities or convert non-UTF-8 bytes into replacement characters before `intern_filesystem_path_identity` rejects them. Test duplicate canonical files, sorting and in-memory `Sources` portable names separately.
- [ ] Run `cargo test -p moth --lib moth_template::tests` and observe the relevant failures.
- [ ] Remove or restrict `normalize_path_for_identity`, which currently builds `PathBuf` from `component.as_os_str().to_string_lossy().replace('\\', "/")` and is used in sorting and duplicate detection. Preserve canonical native `PathBuf` for filesystem identity and sorting. Handle user-supplied in-memory `display_path` using its explicit portable name contract after UTF-8 validation, without changing filesystem names. Keep the existing typed non-UTF-8 and duplicate-input failure lanes.
- [ ] Rerun filtered tests, assess deterministic output ordering and cross-platform logical source identity, then inspect for remaining display-derived native path keys.
- [ ] **Review gate:** inspect input variants, changed identity keys, source interning, potential duplicate tests and redundant normalizers. Commit.

## Task 8: Correct native output-root safety independently

**Files:** `src/build_system/output/{manifest.rs,policy.rs}`, tests under `src/build_system/output/tests/`.

- [ ] Add a focused Windows test for `is_dangerous_system_path` through its owner safety boundary with canonical `\\?\C:\Windows`, `\\?\C:\Users` and ordinary aliases, including case/trailing-separator variants. Retain Unix dangerous-root tests and an allowed project-local hidden output path. Assert the typed output rejection reason, not just string text.
- [ ] Run `cargo test -p moth --lib policy_tests`. Show the Windows regression failing under the old guard where possible.
- [ ] Make safety comparisons operate on native/canonical path components with Windows prefix normalization appropriate to identity, not `to_string_lossy().to_lowercase()` or `portable_path_text`. Preserve the existing project/adjacency, hidden-directory and symlink checks. Audit whether the shared Windows normalizer is being used for comparison only.
- [ ] Rerun output policy/manifest tests, `cargo check -p moth` and the relevant writer tests. **Review gate:** check bypasses, false positives, root/UNC cases and no string-based containment. Commit this owner-specific safety correction separately from diagnostic formatting.

## Task 9: Add a narrow source tripwire and concise rules

**Files:** `xtask/src/source_audit.rs`, `xtask/src/source_audit/tests.rs`, `AGENTS.md`, `docs/developer-docs/style-guide/{style-guide.mtf,testing.mtf}`.

- [ ] Write source-audit fixtures that flag the known direct diagnostic-path forms (`intern(&path.to_string_lossy())`, `get_or_intern(path.display().to_string())`) in production diagnostic producers, including relevant multiline variants. Prove that IO, identity, `PathId`, validated output paths, URL encoding and unrelated `.display()` are allowed. Record what source matching cannot prove.
- [ ] Add a single scoped typed rule such as `UnportableDiagnosticPathFact` to the existing `just source-audit`. Match only reliable producer patterns, with a clear message pointing to `intern_path`. Avoid blanket bans and a new parser/regex framework. If a sub-pattern is unreliable, drop only that detection and retain its exact behavioral regression test.
- [ ] Run `cargo test -p xtask source_audit` (`xtask` is binary-only), `just source-audit`, plus formatter and style checks.
- [ ] Add one terse AGENTS rule: use `portable_path_display::portable_path_text` for host-path presentation and `DiagnosticStringBuilder::intern_path` for diagnostic path facts, while retaining native/semantic/output path identities. Add one concise style-guide Diagnostics rule. Replace the existing testing-guide portable-path bullet with direct string-assertion guidance; avoid duplicating text.
- [ ] Run the documentation check prescribed by `validation.mtf`. **Review gate:** source rule scope, false positives, dead helper imports, updated docs and test honesty. Commit.

## Task 10: Full-tree re-audit and final verification

- [ ] Re-run the Task 0 search and review remaining `.display()`, `.to_string_lossy()`, slash replacement, `portable_path_text()` and path-related `intern` calls in **production and tests**. Ensure every relevant site is migrated or explicitly justified in the inventory. Include `crates/moth-cli/src/scaffold/mod.rs` as a distinct command-quoting case: `cd` output needs shell-safe quoting, not portable display conversion. Record any separate CLI fix required.
- [ ] Check cross-owner duplication, obsolete `path_id`, redundant `normalize_path` use, pointless `String` allocations, moved tests, stale comments and changed diagnostic codes/labels. Confirm source semantics, `PathId`, source ordering, output identity, URI references and physical containment have not been silently altered.
- [ ] Conduct an independent **post-migration audit** of the full diff with the code review guide and style guide. Correct findings, rerun the source scan and review the corrected diff. Do not accept an unexamined residual violation or incomplete safety test.
- [ ] Run focused tests for all changed owners, `cargo test -p xtask source_audit`, `just source-audit`, relevant Moth integration cases and `just validate`. For merge readiness, run `just validate-full` on the final tree and obtain Windows CI evidence. Report actual commands/results rather than assuming unrun gates.
- [ ] Finish with a fresh whole-branch Slice review and a clean handoff listing commits, significant deleted/reused helpers, migrated path sites, justified identity/URL exceptions and any explicit follow-up/blocker.

## Acceptance

- [ ] The Windows no-entry failure is fixed at the diagnostic fact and assertion boundaries, with exact portable string expectations and correct compiled-root provenance.
- [ ] The sole display formatter and `intern_path` handle all applicable diagnostic facts and user-facing native path text without extra wrappers or repeated allocations.
- [ ] Existing filesystem, logical, output and URL representations retain their intended identities. MTF native paths cannot pass through lossy identity conversion, and Windows output-root protection recognizes canonical verbatim paths.
- [ ] Source links and browser import references use correctly encoded URLs, not display strings.
- [ ] Every phase has passing focused evidence and an independent review. The final search, full validation and Windows CI give accurate merge-readiness evidence.
