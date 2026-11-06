# Explicit entries and runtime title

## Status

- Status: queued. The earlier `$page` metadata and entry-purpose proposal is superseded.
- Prerequisites: shared directive recognition and project configuration, plus direct const root builder-slot metadata.
- Next action: refresh the branch, authorities and baseline when this plan becomes active.

HTML metadata belongs to direct const root slot contributions, not `$page` arguments; `docs/docs/project-structure/entry-config.mtf` owns the current contract. The former `$page(head = ...)` design has no successor directive argument. This plan does not restore `page_*` scanning or introduce another metadata path.

## Purpose

Design and implement a general `$entry` directive for explicit root purpose, then add capability-checked runtime title mutation. Neither feature is part of the builder slots cutover. Resolve the exact `$entry` signature and purpose compatibility at activation against the then-current directive authority.

Preserve one directory-scoped semantic module, one module-owned implicit `start`, directory-derived routes and non-executing dependency access. Ordinary files, support roots and package facades do not become application entries.

## Activation reading

Read `AGENTS.md`, the current developer style, testing and validation guides, compiler and build-system architecture authorities, the directive and template references, project structure and resource references, Core IO documentation and the completed builder slot contract. Reconcile current owners and later accepted plans before editing. Preserve the existing compiler data-layout, source ownership and diagnostic contracts.

## 1. Explicit root purpose

Use the eventual compiler-owned `$entry` contract to mark a normal application module root as an entry. A root purpose applies to the whole root regardless of its position. Duplicate purposes and invalid placement need source-oriented diagnostics. A synthetic single-file root follows the same explicit-purpose rule. Do not promote ordinary files, external package roots, support roots or facades.

Validate authored activity, not optimised HIR liveness. Root runtime work and direct output without an applicable purpose must not disappear through entry filtering. An inactive branch containing authored runtime work and an empty direct fragment retain their authored roles. Declaration-only roots and semantically classified documentation or comment templates remain valid without a page. An assigned template value does not itself produce output.

Reuse the normal-root candidate inventory and canonical module compilation. Apply purpose validation to provider roots without activating their `start`, fragments or builder slots in consumers. Unknown or unimplemented purposes must not silently authorise execution. `$test` remains a separate future purpose with its own lifecycle and command design.

## 2. Routes and commands

Keep one semantic module per directory and at most one application page per normal root. Root filename suffixes remain cosmetic. `@site.moth`, `about/@page.moth` and `docs/basics/@module.moth` map to `index.html`, `about/index.html` and `docs/basics/index.html` respectively. Ordinary `about.moth` and `index.moth` gain no route meaning. Do not add route overrides, aliases or redirects.

An application may have descendant entries without a homepage. A valid declaration-only project may pass `check` with no entries. HTML `build` and `dev` need a structured no-entry diagnostic. Synthetic single-file HTML output needs an explicit entry. Preserve separate package-only policy.

Development tooling should launch the real homepage when present, otherwise the lexicographically first normalised route. It must not fabricate a homepage or change the response for an absent root route. Keep output conflicts, manifests and stale-output ownership intact.

## 3. Builder metadata and resources

The HTML builder slot contract owns title, description, language, favicon, body style, stylesheet and head content. It preserves absent versus explicitly empty values, title fallback and prefix/postfix, contextual escaping, structural resource identity and source-aware preflight. JS and supported Wasm document paths share that contract. Entry purpose must not replace, duplicate or bypass it.

A metadata-only root still follows the explicit-purpose policy once this plan lands. Entry activation must preserve slot resource liveness, page-relative URLs and provider isolation. Do not scan rendered HTML or arbitrary constant names for resources.

## 4. Runtime title

Expose `io.set_title` through the existing Core IO binding and capability path. It accepts one ordinary String-content input, returns no source-visible value and has no recoverable result channel. Use the current native return-slot and canonical String conversion contracts.

Give the operation a stable external function identity and an explicit per-function title-capability requirement. HTML-JS advertises the capability and assigns the current document title for reachable calls. The runtime value does not reapply initial title prefix/postfix, alter routes, rewrite output files or mutate static builder slots.

Reject reachable calls before lowering on standalone or embedded JavaScript hosts without the capability and on whole-module HTML-Wasm while unsupported. A JavaScript backend alone does not imply the capability. Unreachable or statically removed calls create no target requirement, though their source still receives normal validation. A host that advertises the capability must implement it, not silently no-op.

`io.get_title`, reactive title binding, other runtime metadata mutation, a Wasm title bridge and mixed-backend partitioning remain outside this plan.

## 5. Implementation and acceptance

1. Refresh authorities, branch and baseline. Inventory current root activity, entry selection, scaffold and title-capability owners.
2. Resolve and implement `$entry` parsing, placement, cardinality and compiler-owned purpose facts. Preserve canonical root and source ownership.
3. Switch entry selection and purpose validation together. Cover normal, synthetic, provider, helper and facade roots, no-entry commands, descendant routes and deterministic development launch.
4. Implement the typed IO operation and capability-gated HTML-JS realisation. Reject unsupported reachable targets and omit helpers for unreachable calls.
5. Update the real starter project, applicable references and progress matrices. Keep builder slots as the sole metadata path. Run focused checks, the current full code-bearing validation gate, the documentation release build and the required Slice review.

Coverage must prove source-oriented placement and duplicate errors, authored activity including false branches and empty output, declaration/documentation exemptions, provider isolation, metadata-only entries, route and output ownership, build/check/dev differences, initial document parity and runtime title capability boundaries. Test the generated starter through its real check and build paths. Retire this plan only after those gates pass.
