# Runtime title mutation

## Status

- Status: queued for capability-checked runtime title mutation.
- Prerequisites: delivered fixed roots, presence-based entry selection and direct const entry builder slots. This plan adds no entry, route, metadata or resource path.
- Next action: refresh the branch, authorities and baseline when this plan becomes active.

`docs/docs/project-structure/entry-config.mtf` and `docs/docs/project-structure/module-roots.mtf` own the delivered entry contract. The HTML project builder section of `docs/build-system-design.md` owns entry selection, builder slots and resource emission. This plan changes only the runtime title operation. It does not restore `page_*` scanning or add a second metadata path.

## Purpose

Add capability-checked runtime title mutation through `io.set_title`. Resolve the exact Core IO signature and target-capability contract at activation against the then-current directive, IO and project-builder authorities.

The delivered builder slots remain the sole source of initial document metadata. Runtime title calls never replace them.

## Activation reading

Read `AGENTS.md`, the current developer style, testing and validation guides, compiler and build-system architecture authorities, the directive and template references, project structure and resource references, Core IO documentation and the delivered builder slot contract. Reconcile current owners and later accepted plans before editing. Preserve the existing compiler data-layout, source ownership and diagnostic contracts.

## 1. Runtime title

Expose `io.set_title` through the existing Core IO binding and capability path. It accepts one ordinary String-content input, returns no source-visible value and has no recoverable result channel. Use the current native return-slot and canonical String conversion contracts.

Give the operation a stable external function identity and an explicit per-function title-capability requirement. HTML-JS advertises the capability and assigns the current document title for reachable calls. The runtime value does not reapply initial title prefix or postfix, alter routes, rewrite output files or mutate static builder slots.

Reject reachable calls before lowering on standalone or embedded JavaScript hosts without the capability and on whole-module HTML-Wasm while unsupported. A JavaScript backend alone does not imply the capability. Unreachable or statically removed calls create no target requirement, though their source still receives normal validation. A host that advertises the capability must implement it, not silently no-op.

`io.get_title`, reactive title binding, other runtime metadata mutation, a Wasm title bridge and mixed-backend partitioning remain outside this plan.

## 2. Implementation and acceptance

1. Refresh authorities, branch and baseline. Inventory the current Core IO binding, capability, scaffold and title owners.
2. Implement the typed IO operation and capability-gated HTML-JS realisation. Preserve canonical root and source ownership.
3. Reject unsupported reachable targets and omit helpers for unreachable calls.
4. Update the real starter project where appropriate, applicable references and progress matrices. Keep builder slots as the sole metadata path. Run focused checks, the current full code-bearing validation gate, the documentation release build and the required Slice review.

Coverage must prove title-capability boundaries, initial document parity and provider isolation. Include a metadata-only entry whose static title stays intact and exercise the generated starter through its real check and build paths. Retire this plan only after those gates pass.
