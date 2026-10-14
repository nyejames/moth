# Lappet

Lappet is Moth's native runtime, Rust host-binding infrastructure and future
package manager. It lives in the Moth workspace and keeps all user-facing
commands under `moth`.

This initial library contains documentation only. It has no runtime dependency,
public execution API or package-installation command.

## Read the design

- [Runtime and embedding](docs/design.md) defines the accepted V1 boundaries.
- [Package management](docs/package-manager.md) defines package eligibility,
  dependency assessment and the future fail-closed admission workflow.
- [Roadmap](roadmap.md) tracks delivery and the design checkpoints that block it.
- [Wasm optimisation](../../docs/src/developer-docs/wasm-optimisation/overview.mtf)
  routes to the shared compiler/runtime optimisation authority.

Develop Lappet in its own isolated worktree. Its roadmap runs independently of
Moth's serial compiler roadmap. Integration consumes delivered compiler
capabilities and never resumes a paused compiler plan implicitly.

The first code-bearing milestone separates the CLI package from the compiler
library. Runtime and package-manager work starts only after its own design
gates have been reviewed.
