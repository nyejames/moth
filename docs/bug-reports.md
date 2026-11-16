# Bug reports

A casual log of bugs and limitations found during other work and left for later investigation. Each entry names where the problem lives, what was observed and a starting point for the fix.

## Open

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

### Windows `\\?\` prefix in damaged package set messages

- Found: Core/Builder package namespace and config filename work, 2026-11. Not reproduced on Windows.
- Where: `BUILTIN_PACKAGES_ROOT` in `src/builder_surface/definition.rs` is canonicalized once and displayed by `damaged_package_set_report`.
- Observed: `fs::canonicalize` on Windows returns verbatim `\\?\` paths, so the reinstall message is expected to show that prefix. Display only. Classification is unaffected.
- Investigate: render the root through the existing portable path display helper.

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
