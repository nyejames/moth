# Runtime title mutation

## Status

- Status: queued for capability-checked runtime title mutation. Explicit entry selection is delivered: a directory with `@entry.moth` is entry-eligible, and directories without it are not.
- Prerequisites: fixed root roles, presence-based entry selection and direct const entry builder-slot metadata.
- Next action: refresh the branch, authorities and baseline when this plan becomes active.

The earlier `$page` metadata proposal and general `$entry` root-purpose directive are withdrawn. No root-purpose directive exists. `docs/docs/project-structure/entry-config.mtf` and `docs/docs/project-structure/module-roots.mtf` own the delivered entry contract. This plan does not restore `page_*` scanning, another metadata path or a root-purpose marker.

## Purpose

Add capability-checked runtime title mutation through `io.set_title`. Resolve the exact Core IO signature and target-capability contract at activation against the then-current directive, IO and project-builder authorities.

Preserve one directory-scoped semantic module, one module-owned implicit `start` in `@entry.moth`, directory-derived routes and non-executing dependency access. Ordinary files, `@package.moth` support roots and API facades do not become application entries.

## Activation reading

Read `AGENTS.md`, the current developer style, testing and validation guides, compiler and build-system architecture authorities, the directive and template references, project structure and resource references, Core IO documentation and the completed builder slot contract. Reconcile current owners and later accepted plans before editing. Preserve the existing compiler data-layout, source ownership and diagnostic contracts.

## 1. Structural entry eligibility

Entry eligibility is a source-structural fact. A directory containing `@entry.moth` is an entry module whether the file is empty, static-only, metadata-only or full of runtime work. `@mod.moth` and `@package.moth` never become entries. A synthetic single-file root follows the command's existing selected-root policy.

An imported module exposes only its public API. Binding a dependency never activates its entry, `start`, fragments or builder slots. This plan does not filter entries by authored activity, a direct `$in`, rendered content or non-empty HIR.

`$test` remains a separate future test capability with its own lifecycle and command design. It is not a root-purpose directive.

## 2. Routes and commands

Keep one semantic module per directory and at most one application page per `@entry.moth` directory. Route paths come from directory position. A root entry maps to `index.html`, and a nested entry maps to its directory-relative `index.html`. Ordinary filenames such as `about.moth` and `index.moth` gain no route meaning. Do not add route overrides, aliases or redirects.

An application may have descendant entries without a homepage. A valid declaration-only project passes `check` with no entries. HTML `build` and `dev` need at least one `@entry.moth` anywhere below the selected project root and report a structured no-entry diagnostic otherwise. A selected `@mod.moth` or `@package.moth` keeps its facade role, so an application build reports the no-entry outcome. Preserve separate package-only policy.

Development tooling launches the real homepage when present, otherwise the lexicographically first normalised route. It must not fabricate a homepage or change the response for an absent root route. Keep output conflicts, manifests and stale-output ownership intact.

## 3. Builder metadata and resources

The HTML builder slot contract owns title, description, language, favicon, body style, stylesheet and head content. It preserves absent versus explicitly empty values, title fallback and prefix/postfix, contextual escaping, structural resource identity and source-aware preflight. JS and supported Wasm document paths share that contract. Entry selection must not replace, duplicate or bypass it.

A metadata-only `@entry.moth` already produces a page. Entry activation must preserve slot resource liveness, page-relative URLs and provider isolation. Do not scan rendered HTML or arbitrary constant names for resources.

## 4. Runtime title

Expose `io.set_title` through the existing Core IO binding and capability path. It accepts one ordinary String-content input, returns no source-visible value and has no recoverable result channel. Use the current native return-slot and canonical String conversion contracts.

Give the operation a stable external function identity and an explicit per-function title-capability requirement. HTML-JS advertises the capability and assigns the current document title for reachable calls. The runtime value does not reapply initial title prefix/postfix, alter routes, rewrite output files or mutate static builder slots.

Reject reachable calls before lowering on standalone or embedded JavaScript hosts without the capability and on whole-module HTML-Wasm while unsupported. A JavaScript backend alone does not imply the capability. Unreachable or statically removed calls create no target requirement, though their source still receives normal validation. A host that advertises the capability must implement it, not silently no-op.

`io.get_title`, reactive title binding, other runtime metadata mutation, a Wasm title bridge and mixed-backend partitioning remain outside this plan.

## 5. Implementation and acceptance

1. Refresh authorities, branch and baseline. Inventory the current Core IO binding, capability, scaffold and title owners.
2. Implement the typed IO operation and capability-gated HTML-JS realisation. Preserve canonical root and source ownership.
3. Reject unsupported reachable targets and omit helpers for unreachable calls.
4. Update the real starter project where appropriate, applicable references and progress matrices. Keep builder slots as the sole metadata path. Run focused checks, the current full code-bearing validation gate, the documentation release build and the required Slice review.

Coverage must prove title-capability boundaries, initial document parity, metadata-only entries, route and output ownership, build/check/dev differences and provider isolation. Test the generated starter through its real check and build paths. Retire this plan only after those gates pass.
