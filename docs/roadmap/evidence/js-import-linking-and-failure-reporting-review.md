# JavaScript import linking and failure reporting evidence

## Activation

Implementation started on `link-fixes-and-test-cleanup` at `aa072a27a32b7233b1e3dcfdd9e4f42b0578bafc` with a clean staged, unstaged and untracked worktree. The user selected this branch, overriding the plan's proposed main landing. Other paused programmes remain paused.

Rust is the repository pin `1.99.0` (`b940084d7`, aarch64-apple-darwin), Cargo `1.99.0`, Clippy `0.1.99` and rustfmt `1.10.0-stable`. Checks use the pinned Node `24.21.0` from the installed nvm version. The baseline uses Cargo's dev profile and HTML development output.

## Baseline regression

`html_js_reexport_linking` owns `build.external_js.reexport_runtime_linking`. Existing direct-provider, source-facade and unused-asset cases do not protect execution through a pure Moth facade with no directly owned provider clause in the caller.

| Case | Command | Revision/profile | Fixture | Observed result | Expected contract | First failing boundary | Disposition |
|---|---|---|---|---|---|---|---|
| Minimal re-export | `cargo run --quiet -- tests --case html_js_reexport_linking --backend html --terse` | `aa072a27`, dev | `tests/cases/html_js_reexport_linking` | 0/1 correct: successful-build expectation failed | Execute `canvas:1` | HTML glue runtime-asset lookup | Confirmed compiler defect; retain exact minimal fixture |
| Failure detail | `cargo run --quiet -- build tests/cases/html_js_reexport_linking/input` | `aa072a27`, dev | Same fixture | `HTML JS glue could not find a runtime asset for external package ExternalPackageId(8).` | Matching provider metadata for retained external call | `runtime_glue/source.rs` | Raw baseline logs retained under ignored `tmp/js-import-linking-red*.log` |
| Supplied website | No build possible | Activation | Supplied archive unavailable | No matching archive found in workspace, `tmp`, project parents or inspected home archive matches | Unchanged website dev/release rebuild | Input availability | Environment/input limitation; no website build claimed |

The attempted `build ... --terse` command was rejected because only `check` and `tests` support that flag. The valid build command above supplied the actual failure detail.
