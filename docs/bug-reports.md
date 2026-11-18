# Bug reports

A lightweight queue for unresolved bugs and issues discovered during other work or a Correctness audit. Any agent may record an unrelated, non-blocking concern here and return to its task without starting a formal audit. This includes concrete documentation, test, redundancy and efficiency leads found outside the current scope.

Check for an existing entry before adding one. Name where the problem lives, what was observed or looks suspicious and a useful next investigation. Distinguish confirmed defects from unverified hypotheses. Keep the format flexible and the entry short when evidence is limited. Follow [the audit guide](./roadmap/audit-guide.md) for formal audit coverage and findings. Remove an entry in the commit that resolves it.

## Open

### Unreachable generic call after return crashes summary convergence

- Found: Constraints branch implementation and independent merge review, 2026-11.
- Where: `call_escapes_builtin_failure` / `retained_call_summary` in `src/compiler_frontend/module_compilation/generated/convergence.rs` (around line 1080).
- Observed: a generic call after an unconditional return is unreachable, but summary convergence still looks for its exact callable summary and raises a compiler bug: `implicit builtin failure contributor has no exact call summary for ...`. The implementation agent reproduced the crash on original base `d80f56343` without any constraint directives, so it predates `$safe_math` and `$infallible`.
- Investigate: preserve the unreachable-call repro as a compiler test, trace how its builtin failure contributor survives without an exact summary and either exclude dead contributors at their owner or ensure the mandatory summary is available before convergence. Keep selected-source fast-use accounting distinct from liveness-conditioned failure propagation. User-authored unreachable code must not trigger an infrastructure failure.

### Mutable local passed by `~` gets an invalid parameter effect summary

- Found: Constraints branch implementation and independent merge review, 2026-11.
- Where: `validate_parameter_summary` in `src/compiler_frontend/public_call_summary.rs` (around line 263), with the upstream mutation/transfer summary producer still to identify.
- Observed: copy an immutable function parameter to a mutable local, then pass that local to another function through `~`. Compilation fails with `MOTH-INFRA-0001` and an `invalid effect combination` message instead of accepting the legal access or producing a source diagnostic. The implementation agent reproduced it on original base `d80f56343` and referenced a temporary probe at `tmp/probes-t3/p90`, which is not a durable test fixture.
- Investigate: first commit a minimal repro under `tests/cases/`, then follow the parameter's declared access, mutation effect and transfer effect to find the first inconsistent producer. Preserve the `Shared` versus `Mutable` summary invariants and fix the producer rather than weakening the validator to accept invalid combinations.

### Spanless release-folded calls may lose debug/release constraint parity

- Found: Constraints follow-up `abc903852` source review, 2026-11. This is a potential limitation, not a reproduced source-level defect.
- Where: `project_selected_hir_evidence` in `src/compiler_frontend/hir/hir_builder/metadata.rs`, specifically the `None => true` liveness fallback for `ast.release_erased_call_sites`, and the release-fold ledger in `ast/const_values/const_execution/rewrite.rs`.
- Observed: a folded call with no source span cannot be matched against skipped statement spans, so the release path assumes it was live. If such a call originates in a skipped handler or after a terminator and its callee may trap or fail, release could reject `$infallible` where debug accepts. No test reached this case. The numeric backend contract calls for identical debug and release verdicts.
- Investigate: determine whether any legal source or materialised callable can produce a spanless erased call in skipped code. Add a focused debug/release parity test if reachable. If impossible, document and enforce that invariant at the producer, then remove the fallback ambiguity. Otherwise carry explicit liveness or enclosing statement identity into the ledger rather than guessing from absent span data.

### `ReservedConfigFile` path reason is unreachable for file values

- Found: Core/Builder package namespace and config filename work, 2026-11.
- Where: `src/build_system/create_project_modules/file_reference_resolution.rs` (directory resolver around line 115, single-file resolver around line 922) and `InvalidCompileTimePathReason::ReservedConfigFile`.
- Observed: a file value such as `@"@config.moth"` is rejected by the AST first with `MOTH-SYNTAX-0023` (`invalid_expression.moth_file_has_no_value`). The resolver check that would report `invalid_compile_time_path.reserved_config_file` never runs for it, although both resolver comments say the reserved filename is reported instead of the no-value reason. The case `file_value_reserved_config_reference_rejected` pins the AST diagnostic.
- Investigate: either let the reserved filename win before the AST no-value check or delete the reason, its code, key and renderer branch along with the misleading comments.

### Damaged package set classification is path-only

- Found: Core/Builder package namespace and config filename work, 2026-11.
- Where: `damaged_package_set_report` in `src/builder_surface/definition.rs`, used by Stage 0 package discovery, source tree indexing, resource inputs and package reserved-root validation.
- Observed: the infrastructure lane is chosen by whether a path sits below the canonical `BUILTIN_PACKAGES_ROOT`. A ProjectLocal or Dependency package registered inside the compiler's `packages/` directory gets a damaged-installation failure instead of an authored diagnostic. An official package root that is a symlink to a directory outside `packages/` gets the authored diagnostic instead. Both need a modified installation.
- Investigate: whether lane selection should also require Core or compiler-shipped Builder registration identity, kept in one owner for all callers.

### Repeated work in single-file namespace validation

- Found: Core/Builder package namespace and config filename final review, 2026-11.
- Where: `ModuleNamespaceSet::for_single_file` in `src/build_system/create_project_modules/module_namespace.rs` and `single_file_namespace_inventory` / `eligible_child_directories` in `src/build_system/create_project_modules/source_tree_index.rs`.
- Observed: a selected `@entry.moth` or `@mod.moth` builds a `ProjectModuleGraph` and project namespaces only to validate reserved-root claims, then discards them. The registered-prefix collision probe for an ordinary selected file can list the same parent directories again for each registered source package prefix. Both are bounded and small today.
- Investigate: reuse the namespaces later stages build for bounded selections, and group prefixes by shared parent components in the probe. Measure before changing.

### User-facing package backing docs omit `HostBinding`

- Found: Core/Builder package namespace and config filename work, 2026-11.
- Where: `docs/docs/packages/package-origins-and-backing.mtf` (backing list and example table).
- Observed: code has three backings (`MothSource`, `ExternalBinding`, `HostBinding`) and `docs/build-system-design.md` now lists all three. The user-facing package reference still lists only `MothSource` and `ExternalBinding`, so builder SDK host packages have no documented backing.
- Investigate: add `HostBinding` with a builder SDK host package example, matching `Package classification` in `docs/build-system-design.md`.

### Scaffold `cd` instruction is not shell-quoted

- Found: portable path display hardening inventory, 2026-11.
- Where: `render_summary` in `crates/moth-cli/src/scaffold/mod.rs` (`format!("  cd {}", report.project_path.display())`) and `crates/moth-cli/src/scaffold/tests/summary_tests.rs`.
- Observed: the printed next-step command interpolates the native display spelling with no quoting. Spaces, quotes and shell metacharacters break or alter the pasted command. Control characters pass through to the terminal and a non-UTF-8 directory prints a lossy path that cannot address the directory.
- Investigate: a CLI-owned shell quoting operation with a stated POSIX and Windows contract, control-character handling and no runnable command for an unrepresentable path. Portable path display is the wrong operation here.

### Benchmark invocations pass display text as child arguments

- Found: portable path display hardening inventory, 2026-11.
- Where: `xtask/src/benchmark_workspace.rs` (`absolute_entry.display().to_string()`, `workload.entry.display().to_string()`) feeding `CliBenchmarkInvocation.args: Vec<String>` and `process_runner.rs`. Related display-only quoting in `xtask/src/profile/runner.rs` (`format_command_line`, `shell_display_arg`).
- Observed: a non-UTF-8 repository or entry path becomes U+FFFD text, so the compiler is launched against a different path. The recorded profile command quotes only empty or whitespace-bearing arguments.
- Investigate: keep the entry argument as `PathBuf`/`OsString` through `Command::arg`. Quote recorded command lines completely or label them as non-runnable.

### Import case check skips non-UTF-8 canonical components

- Found: portable path display hardening inventory, 2026-11.
- Where: `src/compiler_frontend/paths/dependency_resolution.rs` (canonical component collection with `segment.to_str()` and `first_case_mismatch`).
- Observed: a non-UTF-8 canonical component is dropped from the compared list, the lengths differ and `first_case_mismatch` returns `None`, so `InvalidImportPathReason::CaseMismatch` is skipped. Containment still applies and later source loading may reject the file.
- Investigate: treat a non-UTF-8 canonical component as a typed rejection or a mismatch instead of omitting it. Confirm which lane rejects the file today.

### Boracle projected field source tests fail on `main`

- Found: portable path display hardening validation, 2026-11.
- Where: `src/projects/tests/boracle_tests.rs` (`run_source_dump` and `successful_value`), run by `just boracle`.
- Observed: `boracle_source_base_alias_protects_fresh_projected_storage`, `boracle_source_distinct_projected_fields_remain_disjoint`, `boracle_source_projected_rebind_separates_old_projection_origin` and `boracle_source_projected_write_is_not_a_binding_definition` fail with a user diagnostic (`DiagnosticCode(3044)`) on the projected field write. The same failure reproduces at base `7b50e17e9`.
- Investigate: whether the frontend now rejects these projected writes by design, in which case the fixtures need updating, or whether the Boracle source path lost a supported form.

### HTML `origin` has no URL encoding contract

- Found: portable path display hardening dev-server URL review, 2026-11.
- Where: `validate_origin`, `prefix_origin`, `strip_origin_prefix` and `origin_root_url` in `src/projects/routing.rs`. The dev server strips the origin from the raw request path in `src/projects/dev_server/http.rs` before percent-decoding the site-local path.
- Observed: validation accepts spaces, Unicode and `%` in the `origin` setting. Emitted URLs use the authored spelling verbatim, so a browser requests `/caf%C3%A9/...` for origin `/café` and the dev server's literal prefix match returns 404. It is undefined whether an authored `%20` is already encoded text or a literal percent.
- Investigate: choose one rule, either restrict origin segments to URL-safe characters or treat the authored origin as decoded text and encode it on emission. Apply the same rule to the dev server prefix match and add loopback coverage.

### Absolute `PathId` facts render with a doubled root separator

- Found: portable path display hardening MTF input review, 2026-11.
- Where: `DiagnosticPathContext::render` in `src/compiler_frontend/compiler_messages/render/context.rs` and `render_portable_with` in `src/compiler_frontend/symbols/path_interner/fork.rs`.
- Observed: both join resolved components with `/`. An absolute native path interned with its root component `/` renders as `//project/src/page.mtf`. A Windows prefix and root component would render with mixed separators. Logical relative paths are unaffected.
- Investigate: whether absolute `PathId` facts should reach diagnostics at all. If they should, render root and prefix components without an extra separator and add exact Unix and Windows assertions.

### Windows root-relative in-memory template names fail as internal errors

- Found: portable path display hardening MTF input review, 2026-11.
- Where: `validate_in_memory_source_name` and in-memory `Sources` handling in `src/projects/html_project/moth_template/input.rs`.
- Observed: on Windows `Path::is_relative` reports `/docs/x.mtf` as relative, so the name passes input validation and later fails as an internal logical-path failure instead of a typed input failure.
- Investigate: classify a leading `/` without a drive prefix explicitly and reject or accept it with the typed input lane on every host.

### Direct Moth template sources reject a non-UTF-8 ancestor above the source root

- Found: portable path display hardening final integration review, 2026-11.
- Where: `validate_source_path_identities` and `intern_filesystem_path_identity` in `src/projects/html_project/moth_template/input.rs`.
- Observed: the direct filesystem lane interns each whole native source path, so a non-UTF-8 directory above the selected source root fails the MTF-owned UTF-8 identity check even when every module-relative name is UTF-8. The lane has no production caller outside its tests, and no test covers the case.
- Investigate: whether identity should be checked only for the root-relative portion. Add a Unix test with a non-UTF-8 ancestor either way.

### Windows protected output roots are only checked on `C:`

- Found: portable path display hardening final review, 2026-11.
- Where: `DANGEROUS_WINDOWS_ROOTS` in `is_dangerous_system_path` (`src/build_system/output/manifest.rs`).
- Observed: named Windows protected directories are compared only against their `C:` spelling, so a Windows installation on another drive leaves `D:\Windows`, `D:\Program Files` and `D:\Users` unprotected. Drive and share roots are still rejected by the structural rule.
- Investigate: derive the protected roots from `SystemDrive` or `SystemRoot` rather than hard-coding `C:`, and keep the component comparison. This limitation predates the native-component safety work.

### Wasm rejects a runtime fragment that renders an open const loop template

- Found: `$supplied` directive work, 2026-11. Reproduced without `$supplied` on base `1c79842cf`.
- Where: `wasm_numeric_rejection` in `src/backends/backend_feature_validation.rs` rejects the range loop's numeric operation because it is delivered by `Invoke`. The upstream lowering that turns an open const loop template into runtime loop code still needs identifying.
- Observed: in this entry, `html` builds and `html_wasm` fails with `MOTH-RULE-0064` (`Backend 'Wasm' does not support recoverable numeric failure yet`) at the fragment. Applying the template as `[looped:[$in x:"*"]]` fails the same way. A loop body without `$slot` builds on both backends.

  ```moth
  looped #= [:<ul>[loop 0 to 2 |i|:[$slot x]]</ul>]
  name = "Priya"
  ~[:[name]|[looped]]
  ```

- Workaround: `template_supplied_nested_composition` renders its loop checks in a separate fragment, `~[:[loops]]`, whose const content folds before lowering.
- Investigate: whether a const range loop with constant bounds should fold or lower without checked `Invoke` delivery. Otherwise, implement recoverable numeric failure on Wasm. Pin the entry above as an `html_wasm` success case once fixed.

### Whitespace around a consumed `$in` helper is kept on both sides

- Found: `$supplied` directive work, 2026-11. Reproduced without `$supplied` on base `1c79842cf`. Possibly intended, since authored body whitespace is otherwise preserved.
- Where: loose-content grouping in `route_tir_fill_nodes_against_schema` (`src/compiler_frontend/ast/templates/tir/slot_composition/contributions.rs`). Its contract says body formatting whitespace around explicit inserts is not routed.
- Observed: given `plain #= [$html:<button>[$slot][$slot note]</button>]`, the following fills render as shown.

  | Fill | Output |
  |---|---|
  | `[plain: Go]` | `<button> Go</button>` |
  | `[plain:[$in note:] Go]` | `<button> Go</button>` |
  | `[plain: [$in note:] Go]` | `<button>  Go</button>` |

  The space before the helper and the space after it both reach the default slot, so a common idiom yields a double space.
- Investigate: decide whether inline spaces next to an `$in` helper count as formatting whitespace. If they do, drop them at routing. If not, document that the helper does not absorb adjacent spaces.

### JS emitter rebuilds a block lookup map that dense HIR already provides

- Found: AUD-0022 numeric proof redundancy audit, 2026-11. Unmeasured efficiency and redundancy lead outside that audit's area.
- Where: `JsEmitter::new` in `src/backends/js/emitter.rs` builds `blocks_by_id: HashMap<BlockId, &HirBlock>` over every module block, and `block_by_id` in `src/backends/js/lookups.rs` reads it. `HirModule::block` in `src/compiler_frontend/hir/module.rs` already offers an identity-checked dense lookup, relying on the `compact_block_ids` invariant that analyses such as `analysis/numeric_proofs` depend on.
- Observed: every JS lowering, including each linked module, hashes all blocks into a second lookup structure. Every reachability, cycle, branch-inspection, demand-scan and emission visit then goes through that map.
- Investigate: confirm that every HIR reaching `lower_hir_to_js` has passed `compact_block_ids`, including hand-built backend test fixtures. If it has, replace the map with `HirModule::block` and keep the typed infrastructure failure for a missing block.

### Frontend analysis module header names only frontend consumers

- Found: AUD-0024 numeric proof comments audit, 2026-11. Comments lead in the parent module, outside that audit's area.
- Where: the header of `src/compiler_frontend/analysis/mod.rs` says "later frontend stages need one place to access borrow-checking reports and analysis data".
- Observed: the module also declares `numeric_proofs`, whose only production consumers are the JS and Wasm backends through `ModuleExecutable`. The WHY names neither those consumers nor the pass's optional status.
- Investigate: confirm the consumer set of both child modules, then state what the analysis module owns and who reads its results.
