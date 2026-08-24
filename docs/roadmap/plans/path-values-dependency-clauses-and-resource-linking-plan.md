# Path values, dependency clauses and resource linking - split record

## Status

```text
STATUS: historical index - half complete, do not implement this file directly
REMAINING WORK: docs/roadmap/plans/path-values-and-resource-linking-plan.md
```

A single combined plan once owned both dependency-clause grammar and the resource model. It was split because the two halves had different prerequisites and ownership boundaries. This file records the split and where each half went. It is not an active plan and carries no implementation contract.

## 1. Dependency clauses and path syntax - complete

Delivered. The plan file was removed on completion, per the repository convention that Git history is the implementation record. It owned the file-owned retained path syntax table, one `DependencyShellId` per authored top-level clause, removal of the source `import` keyword, extensionless source and package clauses, registered explicit-extension provider clauses, flat direct-selection and namespace binding, explicit re-export rules, and the repository-wide syntax migration.

The accepted contract now lives in the canonical package documentation under `docs/src/docs/packages/`, principally `dependency-clauses.mtf` and `dependency-paths.mtf`. Read those rather than this file.

## 2. Path values and resource linking - remaining

[`path-values-and-resource-linking-plan.md`](./path-values-and-resource-linking-plan.md)

Owns builtin compile-time `Path`, stable module and provider resource origins, module-local resource IDs and build-owned source records, structural resource anchors through AST, TIR, HIR and link facts, public Path values and resource-bearing templates, exact entry and package resource unions, builder placement and containing-artefact-relative URLs, provider-managed and generated resources, and deletion of the eager rendered-path asset reconstruction lane.

Its three prerequisites - canonical module compilation Gate D, dependency clauses and path syntax, and the TIR corrections - are all complete. The resource-only dev fast path stays deferred until the core resource model is accepted.

## Boundary this split established

A file-local dependency clause and a project dependency declaration are different facts:

```text
ProjectDependencyDeclaration
-> makes one external package available to the project resolver and Stage 0 namespace

FileDependencyClause
-> binds names from an already registered source/package/provider root into one source file
```

Source dependency clauses never add undeclared external packages implicitly. Project-level declaration and resolver policy belong to the design-gated [`package-dependency-declarations-and-manager-foundations-plan.md`](./package-dependency-declarations-and-manager-foundations-plan.md), which owns this boundary now. It must not restore the removed source `import` grammar or let parser implementation invent a config surface.

The `@` path rules this split locked - `@` starting a project or package path, the invalidity of `@./...`, parent components, `@/...` and `@@name`, and resolution starting from the owning module root rather than the physical source-file directory - are now canonical in `docs/src/docs/packages/dependency-paths.mtf`. That page is the authority; this file is not.
