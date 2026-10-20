# Lappet native binding handoff plan

This plan delivers Lappet roadmap milestone 2, "Freeze the native binding and
artefact handoff". It is the entry point for the agent continuing the work on
the `lappet-and-packages-work` branch. Delete it in the commit that completes
the milestone and update `crates/lappet/roadmap.md` in the same commit.

## Status

- Status: active. Phases 0, 1, 2A, 2B.0 and 2B.1 are delivered.
- Current slice: Phase 2B.2, builder configuration sections and the build profile.
- Blockers: none for Phase 2. Executable native Wasm lowering is blocked on the
  compiler prerequisites listed in the native host ABI. Phase 2 does not need it.
- Next action: implement the `.config(name, |config| ...)` helper, the generic
  output roots, `CompiledProject::settings` and the public `BuildProfile`.

## Goal

Deliver a checked V1 compiler-to-Lappet host-binding and callable-export
contract with a complete public custom-builder SDK. A separate Rust crate must
be able to define a builder, compile a real Moth project and publish a
supported output through the canonical output writer. Do not invent a second
semantic type system and do not implement the Wasmtime engine. Execution
belongs to Lappet roadmap milestone 3.

## Read first

Follow the root `AGENTS.md` and `crates/lappet/AGENTS.md` routes. These are the
authorities this work consumes:

- `docs/src/developer-docs/memory-management/native-host-abi/native-host-abi.mtf`
  owns the accepted V1 physical boundary and its compiler prerequisites.
- `crates/lappet/docs/host-abi.md` owns Lappet's adapter, staging,
  resource-table and fault duties and its deferred tests.
- `docs/src/developer-docs/project-builders/creating-a-builder.mtf` and
  `capabilities-and-directives.mtf` describe the public SDK target. Their
  `#[moth::host_function]` and `moth::host_binding!` spellings are accepted
  future conveniences. Today's descriptors are the manual `moth::host` types.
- `docs/build-system-design.md` owns builders, configuration and outputs.
- `crates/lappet/roadmap.md` milestone 2 lists the evidence the milestone needs.

## Global constraints

- Keep the dependency direction `moth-cli -> moth` and `moth-cli -> lappet`.
  The `moth` crate never depends on Lappet, Wasmtime or a Rust host
  implementation.
- Reuse the external-package registry, `ExternalFunctionSpec`, ordered return
  slots, canonical identities and existing build orchestration. No `repr(C)`,
  guessed layouts, second type registry or general serialisation framework.
- The public SDK must not expose `ProjectCompilation`, HIR, `BuilderSurface`,
  `StringTable` or other compiler-private types.
- Macros come later and only after a separately approved slice.
- No Wasmtime integration, native Core implementations, package-manager
  commands or backend overhaul.
- Every executable change needs a red then green test. Never report a test
  that was not run.

## Process for every phase or sub-phase

1. Write the failing tests, implement the smallest complete slice and remove
   anything it makes obsolete.
2. Run an independent audit with a fresh reviewer. Cover correctness against
   the owning contract, ownership and duplication, code quality, coverage,
   validation evidence and the disposition of each finding.
3. Fix every blocker, rerun the focused tests and commit. Commits are authored
   by Nye only with no attribution lines. Push with
   `git push -u origin lappet-and-packages-work`. Do not open a pull request.
4. Stop for the user on any unresolved design or public SDK choice.

Update the packages-and-builders progress matrix row "Public Rust builder SDK"
and `index.md` when their rules require it.

## Accepted decisions

The user approved these. Do not reopen them without new evidence.

- **V1 ABI:** i64 resource tokens, builtin `Error!` only, a faulted instance is
  dropped, a process-wide token generator, host context supplied per entry
  call, staged-text allocation failure traps, exports are concrete public
  facade functions and `host.assertion_failed` moves to `moth-abi-v1`. The full
  list lives in the native host ABI.
- **V1 deferrals:** byte buffers, records, choices, collections including
  `{Byte}`, multi-slot host imports, resource results, optional resources and
  guest-side close.
- **Phase 2 needs no working native `compile_wasm`.** It returns a typed
  unsupported-capability error and never mints fake Wasm.
- **Naming:** the internal selection struct is `BuilderSelection`. The public
  trait is `moth::builder::ProjectBuilder`.
- **Builder identity:** a validated string `BuilderIdentity`, not a closed
  enum. First-party identities are reserved. Today only `html` is reserved,
  including its ASCII case-only variants. Add future first-party builders,
  such as a Lappet full-stack builder, to `FIRST_PARTY_BUILDER_IDENTITIES`.
- **Host descriptors:** the closed public `moth::host::{HostType,
  HostFunction}` vocabulary maps one to one onto `ExternalFunctionSpec`.
- **Outputs:** builders register their own configuration section with
  output-root fields. That needs dynamic section and field names, because
  `NamedConfigSectionSchema.name` and `ConfigSchemaField.name` are
  `&'static str` today. Every builder section gains `dev_output` and
  `release_output` with the HTML defaults, and HTML moves onto the same generic
  path. A builder without a section uses the defaults.
- **Phase 2B SDK shape:** `ProjectBuilder` has `definition` and `build`.
  `build_project(&dyn ProjectBuilder, BuildRequest)` returns `BuiltProject`,
  which exposes `warnings`. `BuildRequest::new` takes the project root.
  `CompiledProject::require_package` returns `ProjectPackage<'_>`, an
  owner-bound handle and the future `compile_wasm` input.
  `CompiledProject::settings` returns typed `string` and `bool` reads of the
  builder's section. `BuildContext` and `BuildArtifacts::new` start empty.
- **SDK errors:** the kinds are `InvalidDefinition`, `Compilation`, `Build`,
  `Infrastructure` and `CompilerBug`. `Error::build` creates builder failures
  and `Error::messages` returns rendered terse compiler lines. A structured
  diagnostic view comes later. A missing facade is a `Build` error.
- **Definition rules:** `finish` rejects relative `from_dir` paths and
  `available_in_templates` without `moth_templates`.
- **Build profile:** the SDK re-exports the existing `BuildProfile` enum.
  `BuildRequest::profile` selects it and builders read it through
  `BuildContext::profile`.
- **Configuration helper:** `.config(name, |config| ...)` passes a
  `&mut ConfigSection`. `config.string` returns `&mut StringSetting` and
  `config.bool` returns `&mut BoolSetting`. A definition has at most one
  section. Its name is any Moth identifier except `project` and the
  first-party builder identities. The section must be authored only when it
  declares a required field, and an omitted section yields its declared
  defaults. A field cannot be both required and defaulted. `dev_output` and
  `release_output` are reserved for the automatic output roots.

## Delivered so far

- **Phase 1:** the accepted V1 host ABI documents listed above.
- **Phase 2A:** `moth::builder::{BuilderDefinition, BuilderDefinitionDraft,
  SourcePackage, HostPackage, Error, ErrorKind}` and `moth::host`. `finish` is
  the only path into the registries. It validates the identity, package paths
  (no `#`, reserved `@project` and `@core` including case variants), single
  component source roots, case-only duplicates, source and host path overlap
  and the V1 signature profile. Tests live in `src/builder/tests/` and
  `tests/builder_sdk.rs`.
- **Phase 2B.0:** `finish` rejections use builder-facing terms and name the
  offending package, path or function. `BuilderDefinition` has a compact
  `Debug`. The definition rules above are enforced and documented.
- **Phase 2B.1:** `moth::builder::{ProjectBuilder, BuildRequest, BuiltProject,
  build_project}`, `CompiledProject`, `ProjectPackage`, `BuildContext` and
  `BuildArtifacts` run a project through the shared build orchestration. A
  private adapter presents the builder to that orchestration as a
  `BackendBuilder`, so the SDK path cannot drift from the CLI. `ErrorKind` covers
  `InvalidDefinition`, `Compilation`, `Build`, `Infrastructure` and
  `CompilerBug`, and `Error::messages` carries the rendered terse compiler lines.
  A missing package facade is a `Build` error. Tests live in
  `src/builder/tests/` and `tests/builder_sdk.rs`.

## Code map

| Fact | Owner |
|---|---|
| Builder identity and output ownership | `src/build_system/output/policy.rs` (`BuilderIdentity`, `OutputOwner`) |
| Manifest ownership and recovery | `src/build_system/output/manifest.rs` |
| Public definition SDK | `src/builder/definition.rs`, `src/builder/error.rs` |
| Public SDK build driver | `src/builder/project.rs` (`build_project`, `ProjectBuilderAdapter`) |
| Public compiled project handles | `src/builder/compiled_project.rs` (`CompiledProject`, `ProjectPackage`, `BuildContext`, `BuildArtifacts`) |
| Host descriptors | `src/host/mod.rs` |
| Package path rule | `src/builder_surface/package_metadata.rs` (`validate_package_path`) |
| Surface validation | `src/builder_surface/definition.rs` (`BuilderSurface::validate`) |
| Internal backend trait | `src/build_system/build.rs` (`BackendBuilder`) |
| Internal build driver | `src/build_system/build.rs` (`build_project`) |
| Package facade assembly | `ProjectCompilation::package_assembly` in `build.rs`, reached through `CompiledProject::require_package` |
| Output writing | `src/build_system/output/orchestrator.rs` (`write_project_outputs`) |
| Output root settings | `effective_output_settings` in `src/build_system/project_config/validation.rs`, HTML-coupled |
| Config schema | `src/builder_surface/config_schema.rs` |
| First-party builder | `src/projects/html_project/html_project_builder.rs` |

## Phase 2B: immutable compiled project and build context

- [x] Write a failing external-crate test that implements
  `moth::builder::ProjectBuilder` with `definition` and `build`. A valid
  project reaches `build` with an immutable compiled project. Invalid source
  never calls `build`.
- [x] Implement `ProjectBuilder`, `CompiledProject`, `BuildContext`,
  `BuildRequest` and `build_project` through one internal `BackendBuilder`
  adapter over the existing orchestration. Do not add a forwarding framework.
  `CompiledProject::require_package` uses `package_assembly` and reports a
  missing facade as a builder error.
- [ ] Add the builder configuration section helper,
  `.config("game", |config| ...)`, with dynamic section and field names and
  output-root fields. Decouple `effective_output_settings` from the HTML
  section so a custom builder's roots come from its own section.
- [ ] Re-export the existing `BuildProfile` enum as `moth::builder::BuildProfile`.
  `BuildRequest::profile` selects it and `BuildContext::profile` reads it.
- [x] Add error kinds for compilation failure that carry the compiler messages
  without exposing internal types.
- [x] Test independent definitions in one process, deterministic package
  identities, no access to private module arrays or mutable HIR, and a Byte
  versus U8 type-check case through a real build.
- [x] Remove the `dead_code` allow on `BuilderDefinition::surface`.

## Phase 2C: artefact publication

- [ ] Test an external builder that builds a real minimal output, adds it
  through `BuildArtifacts` and publishes it with `write_outputs`.
- [ ] Implement `BuildArtifacts::add` with validated relative paths and
  collision detection. Reuse `parse_relative_path`, `output_path_identity` and
  the existing writer and manifest lifecycle.
- [ ] Test failed compilation, failed build, invalid paths, write failure,
  repeated builds and that a failed replacement never cleans stale output.

## Phase 2D: native Wasm availability and SDK acceptance

- [ ] Add `BuildContext::compile_wasm`, which returns a typed
  unsupported-capability error until the compiler supplies the accepted ABI.
- [ ] Add a separate Cargo fixture crate that uses only `moth::builder` and
  `moth::host` to register packages, compile, write one real artefact and see
  the honest `compile_wasm` failure. Either add it as a workspace member or
  give it its own `[workspace]` and exclude it at the root.
- [ ] Prove the compiler has no Lappet or Wasmtime dependency, for example
  with `cargo tree -p moth`.
- [ ] Audit from a third-party builder author's perspective, covering every
  public type in the trait closure.

Phase 2 is complete only when a real external crate runs the whole workflow.

## Phase 3: import and export requirements and preflight

- [ ] Write failing tests for an accepted host import, a missing
  implementation, a wrong type, arity or return channel, a duplicate import
  identity, an explicit export and an unselected declaration that is not
  exported. Pin canonical identity and deterministic ordering.
- [ ] Extend a compiler-owned immutable handoff only as far as needed to
  carry import and export requirements and provenance. If physical lowering
  facts are missing, publish a bounded semantic descriptor and record the
  missing physical step. Never publish placeholder offsets.
- [ ] Add a pure-Rust preflight that checks the requirements against declared
  Rust implementations before any guest entry. It is not another type checker
  or plugin registry. Linker installation stays in milestone 3.
- [ ] Test capability isolation between two builder definitions and that a
  selected export never exposes every function or `moth_start`.

## Phase 4: narrow adapters

- [ ] Implement only the pure-Rust validations that Phase 2 or 3 actually
  calls, each with failing-first tests from `host-abi.md`.
- [ ] Classify every remaining adapter test as blocked on Wasmtime or on
  compiler lowering. No adapter library at all is an acceptable result.

## Phase 5: integration and closeout

- [ ] Run the validation the repository policy requires, including
  `just validate` after these public API changes. Record actual results.
- [ ] Update `crates/lappet/roadmap.md` with delivered status, evidence and the
  exact compiler prerequisites still missing. Update `index.md` for new owners.
- [ ] Write the milestone 3 handoff. Name the descriptor API, the
  import/export check, the existing conversions, what is blocked and the first
  real core Wasm fixtures to run with Cranelift at `OptLevel::None` and
  `OptLevel::Speed`.
- [ ] Run an independent end-to-end architecture and test review, then delete
  this plan.

## Open follow-ups and questions

- The source and host overlap check does not cover JS packages registered
  during compilation.
- A project with a `+<name>.moth` facade but no normal `@*.moth` root fails
  with `NoRootModuleEntries`, because the project boundary always requires a
  normal entry module. The SDK inherits that rule from the shared
  orchestration, and so does the CLI, so a facade-only package cannot build
  through either path. Deciding whether normal-entry admission belongs to the
  HTML builder or to every project is a build-system behaviour change that
  needs its own authority reading, so it is not decided in Phase 2B. The
  artefact-publication and native-Wasm fixtures need a project with both a
  facade and at least one normal module.
- Documentation follow-ups to report rather than edit: the host manifest is not
  yet described in `docs/build-system-design.md` or
  `docs/compiler-design-overview.md`, the reserved first-party identities are
  not yet described in `creating-a-builder.mtf`, and the milestone 2 evidence in
  the Lappet roadmap asks for tests that need emitted Wasm.

## Test environment notes

- The integration suite needs Node 24. The container's default Node 22 fails
  the generated JavaScript tests. Install Node 24 under the ignored `tmp/`
  directory, check its checksum and prepend its `bin` to `PATH` for test runs.
- Three `cargo test --workspace` tests fail when running as root because they
  expect permission errors: `unreadable_project_root_surfaces_file_error_not_missing_facade`,
  `unreadable_normal_module_root_returns_file_error` and
  `node_harness_reports_a_workspace_that_cannot_be_removed`. They fail on the
  unmodified baseline too.
