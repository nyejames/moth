# Lappet host contract crate plan

This plan delivers Lappet roadmap milestone 4, "Extract the shared host
contract crate". It moves the host descriptor vocabulary out of the compiler
into a small crate that both the compiler and Lappet's runtime depend on.
Delete this plan in the commit that completes it and update
`crates/lappet/roadmap.md` in the same commit.

## Status

- Status: accepted, not started. Nye confirmed arrangement B and the
  `moth-host` name, and the runtime design records the graph.
- Current slice: none.
- Blockers: const functions landing on `main`.
- Next action: Phase 0.

## Goal

Lappet's runtime builds against the host contract without the compiler in its
dependency tree. The compiler and Lappet share one definition of every host
descriptor type. The public `moth::host` path keeps working.

## The evaluated split

Lappet has two halves with different needs. The runtime executes Wasm and
needs Wasmtime plus the vocabulary that describes a host binding. The package
manager needs compiler-owned graphs and compatibility facts and must work
without Wasmtime. Three arrangements were compared.

**A. Lappet depends on the whole compiler.** One edge and no new crate. Every
embedder then compiles the frontend, build system and HTML builder to run a
Wasm module, and every compiler edit rebuilds the runtime and its tests. It
also rules out a later compiler-free deployment. The runtime needs about 150
lines of the compiler, the descriptors, so the cost is out of proportion.

**B. A small contract crate, with Lappet split by feature.** Accepted.

```text
moth-lexical  <- moth-mon
moth-host     (no workspace dependencies)
moth          -> moth-lexical, moth-mon, moth-host
lappet        -> moth-host and wasmtime   with the `runtime` feature
              -> moth                     with the future `packages` feature
moth-cli      -> moth, lappet
```

- `moth-host` owns the data both sides must agree on: the descriptor
  vocabulary today, then the host value types, the host manifest's value types
  and the ABI's reserved names and frame-layout rule as each gains a real
  consumer. It contains no Wasmtime type and no compiler-internal type.
- The compiler keeps everything semantic: converting a descriptor to an
  `ExternalFunctionSpec`, registry validation and producing the manifest.
- Lappet's `runtime` feature sees only `moth-host` and Wasmtime. Its future
  `packages` feature sees the compiler and no Wasmtime. Neither feature pulls
  in the other's dependencies.
- `moth::host` stays the public path through a deliberate re-export, the same
  arrangement as `moth::mon`.
- The repository already has the machinery: `moth-lexical` and `moth-mon` are
  extracted packages policed by `just first-party-deps`.

**C. Split Lappet into separate runtime and package crates now.** Cleaner on
paper, but the package manager doesn't exist yet. The runtime design already
says to prefer feature isolation and extract a crate only for a real boundary.
Revisit when the package half has code.

The cost of B is one more workspace crate and one re-export. Its main benefit
is that the facts both sides must agree on have one owner that neither side
can quietly reinterpret.

## Prerequisites

- Const functions are on `main` and `lappet-work` is synced, so the compiler
  crate changes here don't collide with that work.

Nothing else. This plan doesn't need the Lappet runtime to exist.

## Read first

Follow the root `AGENTS.md` routes for "Build system and project
orchestration" and "Tests", and the `crates/lappet/AGENTS.md` route for "CLI
split or compiler/build integration". These own the contracts:

- `crates/lappet/docs/design.md`: `Users and workspace dependencies` and `Rust
  bindings`.
- `docs/developer-docs/project-builders/host-packages.mtf`: the Rust signature
  profile.
- `docs/developer-docs/style-guide/validation.mtf`: the first-party dependency
  boundary section and `Feature lanes`.
- `src/host/mod.rs`, `src/builder/definition.rs` and
  `xtask/src/first_party_deps/rust_dependencies.rs`: the current owners.
- `tests/mon_reexport.rs`: the pattern for proving a re-export shares types.

## Global constraints

- Pure move. No descriptor gains or loses a variant, method or validation
  rule. No behaviour change is visible to a builder author.
- No forwarding wrapper, alias type or second descriptor type. One definition,
  one re-export.
- The contract crate has no dependency on any workspace package, on Wasmtime
  or on the compiler. Add an external dependency only with a current need.
- Move only what exists. Don't add the host manifest, `Error`, `Byte`, frame
  layouts or reserved names here. Each arrives in the plan that first consumes
  it.
- Every change keeps the existing builder tests green.

## Process for every phase

1. Make the change, keeping the build green at each commit.
2. Run the focused checks: `cargo test -p moth-host`,
   `cargo test -p moth builder`, `cargo test -p moth --test builder_sdk`,
   `cargo test -p lappet`, `cargo clippy --workspace --all-targets`,
   `cargo fmt --all -- --check` and `just first-party-deps`.
3. Run the phase review with a fresh reviewer that didn't write the code.
4. Fix every blocker, rerun the checks and commit. Push to `lappet-work`.
5. Stop for Nye on any public API choice.

## Phase 0: activation

- [x] Nye accepted arrangement B and the `moth-host` name.
- [x] `crates/lappet/docs/design.md` records the accepted graph and its
  ownership rule under `Users and workspace dependencies`.
- [ ] Sync with `main` and confirm the prerequisites in the current tree.
- [ ] Re-read the routed documents and report any drift before writing code.

Exit: the branch contains const functions and no drift is unreported.

## Phase 1: create the crate and move the vocabulary

- [ ] Write a failing test, modelled on `tests/mon_reexport.rs`, that builds a
  descriptor through `moth::host` and uses it as the standalone crate's type.
- [ ] Create `crates/moth-host` with the workspace lints, licence and a
  README, and add it to the workspace members.
- [ ] Move `HostType` and `HostFunction` with their builder methods and
  documentation. Expose read access to a function's name, parameters, success
  values and fallibility, because the compiler's conversion now reads them
  across a crate boundary.
- [ ] Move the conversion to `ExternalFunctionSpec`, including the nested
  optional and multi-return rejections, into the compiler beside its only
  caller in `src/builder/`. Delete `src/host/mod.rs` and re-export the crate as
  `moth::host` with a comment stating that the re-export is deliberate.
- [ ] Keep every existing rejection message and test expectation unchanged.

Review focus: the move changed no behaviour, the read accessors expose no more
than the conversion needs, no validation rule now has two owners and the
re-export is the only bridge.

Exit: the new test and every existing builder test pass.

## Phase 2: enforce the boundary

- [ ] Add `moth-host` to the extracted packages in
  `xtask/src/first_party_deps/rust_dependencies.rs` with no permitted
  workspace dependencies, and extend that module's tests.
- [ ] Add a feature-free lane for `moth-host`, following
  `validation.mtf > Feature lanes` and the lane table in `testing.mtf`.
- [ ] Add `moth-host` as an optional dependency of `lappet` under the
  `runtime` feature only if the runtime already has a consumer for it.
  Otherwise leave Lappet's manifest alone and name the edge in the roadmap.
- [ ] Prove the result: `cargo tree -p moth-host` shows no workspace package,
  and `just first-party-deps` passes.

Review focus: the boundary check fails closed for the new crate and no
dependency was added ahead of its consumer.

Exit: boundary checks and lanes pass.

## Phase 3: integration and closeout

- [ ] Update `index.md` for the new crate and the moved owner.
- [ ] Update the first-party dependency paragraph in
  `docs/developer-docs/style-guide/validation.mtf` to name the new extracted
  package. Nye accepted this plan, which approves this one documentation edit.
- [ ] Report, without editing, any other document that names `src/host` as the
  descriptor owner.
- [ ] Update `crates/lappet/roadmap.md` with delivered status, evidence and the
  next action.
- [ ] Run `just validate` and record the actual result.
- [ ] Run an independent review from a third-party builder author's view: the
  public paths, the documentation and the error messages are unchanged.
- [ ] Delete this plan in the completing commit.

## Out of scope

- The host manifest, host value types, frame layouts and reserved names.
- Any Lappet runtime or adapter code.
- Splitting Lappet itself into several crates.
- The `#[moth::host_function]` and `#[moth::host_type]` macros.
