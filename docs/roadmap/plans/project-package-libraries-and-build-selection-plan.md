# Project package libraries and exact build selection implementation plan

> **For agentic workers:** Use the repository's current planning, testing and Slice review rules. Implement tasks in small checked slices. This is a compiler and build-system plan, independent of Lappet runtime implementation.

**Status:** Queued after the compiler supports namespaced public export routes. Must complete before Lappet package work or native readiness milestones resume.

**Goal:** Resolve explicitly declared, exactly versioned Moth-source packages from configured project-local dependency libraries. Use one compiler-owned configuration and dependency path model and compile each package in an isolated graph.

**Architecture:** The config compiler retains package declarations without binding or evaluating their targets. Bootstrap applies fixed project settings to determine library roots. The build system resolves exact package build identities from those roots before Stage 0 constructs dependency graphs. The frontend owns path/qualifier syntax and canonical semantic identities. File-local source clauses only bind already selected packages.

**Specification:** The current compiler and build-system design authorities, `docs/docs/packages/dependency-paths.mtf`, `docs/docs/project-structure/project-config.mtf` and the retained ObVer reference at `docs/docs/packages/obver.mtf`. The accepted V1 rules below explicitly supersede current prohibitions on config dependency declarations and `package_folders` and must be transferred into the permanent authorities before the plan is retired.

## Status and prerequisites

- Current slice: baseline and precise producer/consumer contract.
- Blockers: completion of public-export route support and review of the exact project-setting spelling before production syntax is frozen.
- Next action: inspect the current config compiler, retained dependency path syntax, source index, package registration and package graph owners. Write focused baseline and negative cases.

Prerequisites are existing single-source config compilation, retained path/header syntax, canonical package graphs, immutable public interfaces, separately compiled source package boundaries, valid `@package.moth` facades and stable package identity/fingerprints. Do not add a second parser or package graph representation to satisfy this feature.

## Accepted V1 contract

### Syntax and identity

A top-level dependency clause in `@config.moth` declares a direct external package and its exact ObVer build:

```moth
@cool_package::2026.32.5
@renderer::2027.2-debug as graphics

project #= (
    name = "game",
)
```

- Every config dependency clause is an external Moth-source package declaration, never a file-local import or dependency of config evaluation. Only one whole-package name and optional replacement root alias are permitted. No selected symbols or module subpaths are accepted in config V1.
- `::` conceptually qualifies a dependency *provider* with a selected build identity. V1 supports only the complete exact ObVer identifier for external packages in config. It is not a general expression operator, a public route segment or an arbitrary compile-time settings override.
- The V1 package's canonical external name is one valid project/package identifier, matching one physical package-directory component. The source path `@cool_package/forms` may name a public namespace route within the selected facade, not a different installed package. Namespaced registered third-party package identities require separate directory/identity design.
- A version qualifier is required. Unqualified external package declarations are diagnosed, rather than selecting the latest installed build or a default. Relative and absolute filesystem paths are not source-level dependency declarations.
- A project-local root alias replaces the source-visible canonical import root for that project. It does not create a second name, change the canonical package or build identity or change output resource prefixes. Repeated declarations of one canonical name, even with different versions or aliases, fail within one compilation boundary.
- Ordinary project source imports `@cool_package` or `@cool_package/forms` without an ObVer qualifier. For the alias above, source imports `@graphics`. No `::` qualifier is accepted in ordinary source dependency clauses, qualified expressions or file values in V1.
- Canonical package/build identity includes the exact ObVer build, including its optional label. File-local aliases and physical paths remain provenance, not semantic identity. Distinct build identities may coexist privately across separately compiled dependency graphs when their owners select them.

The existing `Choice::Variant` expression grammar stays unchanged. Header preparation recognises the version-qualified config-dependency form in its own syntactic position. The path tokenizer currently terminates at `:`; change that one owned path/header grammar deliberately and retain a typed build qualifier rather than reparsing a string later. Do not introduce a generic selector registry, a new token language for arbitrary settings or speculative source-module variants.

A future extension may permit a module with an explicitly defined variant set to be selected by `@module::selector`. It requires an independent contract for producing variants, build/config inputs, interface identity, graph and cache compatibility and consumer visibility. This plan neither implements such a producer nor allows imports to recompile existing modules.

### Dependency library layout

A **package library** is a directory containing package directories. A **package directory** is named exactly after the canonical package name and contains version directories. A **package version directory** is named after one canonical ObVer build identifier and is the compilation root of that build.

```text
game/
    @config.moth
    @entry.moth
    packages/                   # default package library
        cool_package/           # package directory
            2026.32.5/         # exact installed build
                @config.moth
                @package.moth
            2026.32.5-debug/
                @config.moth
                @package.moth
        renderer/
            2027.2/
                @config.moth
                @package.moth
```

- `<project root>/packages` is the sole default library. The CLI neither creates nor consults a global directory. An absent default folder is allowed when no selected package requires it.
- A fixed-only project setting may replace the default library list with an explicit list of project-root-relative directories. Initial proposed field: `project.package_libraries`, containing an ordinary folded collection of Strings. Omission gives one `packages` entry. An explicitly supplied list replaces rather than appends to the default. The field is build-control metadata, not a published `@project` value or a dynamic `$config` input. Confirm the final field spelling and exact value restrictions during the Phase 0 design checkpoint.
- V1 library paths must remain within the selected project root after lexical and canonical containment checks. Reject `..`, absolute paths, symlink escapes, overlapping library roots and overlap with owned output locations using typed diagnostics. A valid library remains a subdirectory of the project but is excluded from its ordinary source index. A future managed global library can be an explicitly configured source only after its separate trust, ownership and installation policy is approved.
- Every validated library subtree is excluded from the consuming project's ordinary source index regardless of its name, including check-only units, helper/import lookup and watcher membership. Retain the existing independent underscore-prefixed exclusion rule. A package is indexed only when its version directory is selected as the root of a separate package compilation.
- The selected build requires `@package.moth` at its root and matching `project.name` and canonical `project.version` from its own `@config.moth`. Project applications may remain unversioned, but a selected external package may not. Do not treat the folder name alone as authority for package identity.
- `@core/*`, `@builder/*` and the synthetic `@project` root retain their reserved provenance. An authored package cannot claim those roots.

### ObVer exact selection

Use the supplied ObVer specification, not SemVer precedence or an ad hoc split on dots.

- Format: `YYYY[-MM[-DD]].MINOR[.MICRO][-LABEL]`. Validate Gregorian dates, zero-free integer spelling except `0`, lower-case ASCII snake_case labels and valid component ranges.
- `MICRO=0` is omitted canonically. `2026.32.0` may be accepted in a request and normalised to `2026.32` for lookup; installed directory names and package metadata use canonical spelling.
- The full canonical identifier, including any label, is the exact build identity. `2027.2` does not select `2027.2-debug` or vice versa. Labels have no ordering relative to the unlabelled build or to each other.
- Numeric precedence compares parsed year, month, day, minor and micro, with absent numeric components treated as zero. The V1 resolver uses exact identity, not precedence, ranges, latest-version selection or compatibility guessing.
- Preserve ObVer's release and immutability rules in its independent canonical reference. Version solving, automatic updates, lockfiles and managed integrity checks are later work. Do not infer build compatibility from matching numeric precedence.

### Resolution and compilation

- Bootstrap retains authored config dependency declarations from the existing frontend preparation path, folds only permitted config constants and applies fixed library roots. Config never binds imported symbols, evaluates dependent package code or gains a config dependency graph.
- Resolve each *declared exact build* by constructing `library/package_name/canonical_version` under each configured library root. Inspect only these candidates. Do not scan unrelated installed packages or walk the whole filesystem.
- If two libraries contain the same requested canonical name and exact build, report an ambiguity with both locations. Different installed versions in different libraries do not conflict unless the declaration could select both. No implicit local-first precedence or fallback exists.
- If a requested build is missing, report the missing dependency. If the local project index already knows a same-named module, add a hint that config imports external packages only. This check is diagnostic-only.
- Use the same fixed set of library roots for the entire dependency closure. Each dependency compiles separately with its own config, private project globals, source index and public facade, and may declare its own exact dependencies. A nested dependency config cannot change the invoking project's search-root environment.
- Source clauses can bind only the project's declared direct roots and the explicitly allowed Core/Builder/project-local providers. Transitive external package dependencies are private to their declaring package. Registered public virtual export routes remain views of their owning facade, not new packages.
- Resolver output is one immutable canonical package/build catalogue consumed by Stage 0 and the existing public-interface, link and compatibility owners. Programmatic compiler/builder clients and the CLI supply the same resolved facts. No manager download, lockfile, duplicate registry or automatic package discovery path is introduced.
- Distinguish user configuration/name/version errors, missing package filesystem/IO failures, invalid package structure and incompatible cached artefacts through the existing diagnostic/operational failure lanes. Include the authored config span and candidate paths where applicable.
- Deterministic graph order does not depend on directory enumeration or search-root order. Preserve authored order for diagnostics and canonical identities for package outputs and resource prefixes.

## Explicit exclusions

No global library creation, out-of-project library roots, arbitrary absolute or relative source imports, download/update commands, version ranges, dependency solving, lockfile format, native build scripts, WIT imports or automatic runtime package admission. No general configurable build variants for local modules and no import-time configuration overrides. Do not merge this work with Lappet's native ABI, Core conformance or security assessment.

## Implementation phases

### Phase 0: Baseline and exact contract checkpoint

- [ ] Read `AGENTS.md`, the current style guide, its testing and validation guides, the compiler/build authorities and the canonical package, config and path syntax references.
- [ ] Inspect the config compilation service, source dependency clause/parser, path tokenizer, Stage 0 index, registered source packages, builder SDK and present module collision/diagnostic owners. Map current producers and consumers and remove proposed duplicate owners.
- [ ] Confirm the config field name and exact fixed-only path-list syntax. Record whether an explicitly empty list disables package libraries and how that is diagnosed when dependencies exist.
- [ ] Ratify the build qualifier spelling `@name::ObVer`, V1 one-component external names and the deferral of source-level and local-module selectors.
- [ ] Validate the retained `docs/docs/packages/obver.mtf` against the supplied specification before parser work. Preserve exact-build labels and the canonical `.0` rules.
- [ ] Establish baseline focused tests and record actual command evidence. Do not treat hand-authored fixtures as proof of full downstream native package support.

### Phase 1: One config syntax and typed ObVer parser

- [ ] Add failing positive and negative config cases for qualified declarations, labels, aliases, invalid dates/counters/labels, duplicate canonical dependencies and forbidden local/selected-symbol imports.
- [ ] Extend the existing retained dependency-path syntax in the compiler. Parse a typed exact build qualifier only for top-level config declarations, preserving original source spans and Choice expression grammar.
- [ ] Implement one ObVer parser/canonicaliser/comparator suitable for both project version validation and exact package selection. Avoid SemVer libraries and duplicated parsing between frontend and resolver.
- [ ] Keep config constant folding free of provider interfaces. Return retained dependency declarations as separate build-owned inputs. Run focused frontend/config tests and the codebase Slice review.

### Phase 2: Library configuration, containment and source exclusion

- [ ] Add the approved fixed-only project record field and default local `packages` library. Validate a supplied list once at bootstrap, before Stage 0 indexes project source.
- [ ] Reject path traversal, external symlinks, duplicate/overlapping roots, overlaps with source/output ownership and source-dependent settings with precise typed diagnostics.
- [ ] Exclude validated package-library subtrees from the consuming project's canonical/check-only/source-provider discovery. Compile a selected package version directory in its own boundary, not as an application child module.
- [ ] Add tests for default and custom names, no global auto-creation, an absent unused default, underscored folders and no accidental package source/watch leakage. Run focused indexing/build tests and Slice review.

### Phase 3: Exact catalogue lookup and independent graphs

- [ ] Resolve only authored exact package/build identities from every configured local library, with no precedence. Validate a selected facade plus matching declared name/version. Reject missing, malformed, duplicate and escaping candidates.
- [ ] Produce one immutable resolved catalogue from the existing package registration path and compile each selected Moth-source dependency with the canonical package graph service. Preserve private dependency configuration and canonical IDs across boundaries.
- [ ] Test root alias replacement, reserved roots, local-name hints, duplicate aliases, direct-only visibility, transitive packages, differing transitive versions and package resource/output identity.
- [ ] Test selected source public routes once virtual namespaced exports are available. No duplicate registration for public nested paths.
- [ ] Verify deterministic ordering when the configured library order changes. Verify byte-identical contents in two matching locations still cause ambiguity. Run graph and integration tests and Slice review.

### Phase 4: Integration, documentation and final gates

- [ ] Reconcile `docs/build-system-design.md` bootstrap, source indexing, package declaration/resolver and fixed bootstrap order with this accepted contract. Remove the outdated `package_folders` exclusion and the unconditional ban on config dependency clauses.
- [ ] Update `docs/compiler-design-overview.md` path preparation/config handoff, the canonical package path and config references and the language cheatsheet. Keep config non-semantic and avoid a second compiler parser. Align the package manager's consumption contract without implementing it.
- [ ] Add focused CLI, builder SDK, library collision, exact ObVer identity/label, invalid config and graph-determinism integration cases, including source outputs and package-only builds.
- [ ] Check documentation links, stable capability/ABI fingerprints, invariant and output-resource policy, regression tests and relevant feature lanes. Run the repository documentation gate and the final `just validate-full` gate with the proper toolchain.
- [ ] Perform the repository Slice review, reconcile outstanding incidental findings and retire this plan in its completion commit while removing the roadmap entry.

## Review focus

1. **One parser and graph:** Do config clauses remain retained frontend syntax, with no config binding and no parallel Stage 0 path parser?
2. **Isolation:** Can an installed package leak into ordinary project discovery, be imported transitively without declaration or inspect the consumer's `@project` data?
3. **Exact identities:** Can two ObVer labels, distinct package versions or source aliases accidentally share an artefact identity or resource-output prefix?
4. **Containment:** Can lexical paths, symlinks, malformed metadata or duplicate libraries redirect resolution outside the project or silently choose the wrong package?
5. **Future-proof syntax:** Is `::` limited to a real V1 package-build selector without creating a generic module configuration engine or perturbing Choice construction?

After this plan is implemented and the permanent compiler/build authorities are updated, Lappet can resume its separately gated native readiness and package milestones. The Lappet runtime and host ABI remain unchanged by this plan.
