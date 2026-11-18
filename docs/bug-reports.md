# Bug reports

A casual log of bugs and limitations found during other work and left for later investigation. Each entry names where the problem lives, what was observed and a starting point for the fix. Remove an entry in the commit that resolves it.

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

### A symlink alias into a `@config.moth` directory bypasses its exclusion

- Found: reserved config directory fix, 2026-11. Traced, not reproduced.
- Where: `SourceTreeSkipPolicy::canonical_directory_is_skipped` in `src/build_system/create_project_modules/source_tree_index.rs` and `authored_directory_exclusion` in `file_reference_resolution.rs`.
- Observed: discovery skips a `@config.moth` directory only by its listed name, and the resolvers reject it only by authored components. A visible alias such as `src/alias -> src/@config.moth` is traversed and indexed under its canonical path, and `@alias/notes.mtf` can resolve into the excluded subtree. Hidden `_` directories have canonical-ancestry checks in both places for this case. Separately, dependency clauses that enter a `@config.moth` directory are rejected but the message says the import names `@config.moth` itself.
- Investigate: whether the config directory exclusion should match the hidden-subtree rule in canonical ancestry. If so, add it to the skip policy and both resolvers' canonical checks with a Unix symlink test.

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

### HTML `origin` has no URL encoding contract

- Found: portable path display hardening dev-server URL review, 2026-11.
- Where: `validate_origin`, `prefix_origin`, `strip_origin_prefix` and `origin_root_url` in `src/projects/routing.rs`. The dev server strips the origin from the raw request path in `src/projects/dev_server/http.rs` before percent-decoding the site-local path.
- Observed: validation accepts spaces, Unicode and `%` in the `origin` setting. Emitted URLs use the authored spelling verbatim, so a browser requests `/caf%C3%A9/...` for origin `/café` and the dev server's literal prefix match returns 404. It is undefined whether an authored `%20` is already encoded text or a literal percent.
- Investigate: choose one rule, either restrict origin segments to URL-safe characters or treat the authored origin as decoded text and encode it on emission. Apply the same rule to the dev server prefix match and add loopback coverage.

### Windows protected roots name a nonexistent `System32` directory

- Found: Windows protected output roots fix, 2026-11. Unverified lead.
- Where: `WindowsSystemLocations::protected_roots` in `src/build_system/output/manifest.rs` (`{drive}\System32` and the literal `C:\System32`).
- Observed: Windows keeps `System32` below the Windows directory, not at the drive root, so these entries protect nothing on a standard installation. The fix preserved them to avoid changing protection scope. `policy_tests.rs` asserts that `C:\Windows\System32` is an allowed output root because only exact protected directories are rejected.
- Investigate: decide whether the intended root is `<SystemRoot>\System32`. If so, replace the drive-root spelling and update the allowed-root assertion. Otherwise delete the dead entries.

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
