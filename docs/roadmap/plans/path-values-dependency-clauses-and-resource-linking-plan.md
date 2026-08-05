# Path values, dependency clauses and resource linking plan split

## Status

```text
PLAN_SPLIT_BASELINE: bfaacd54227811f9e2b279d5a24e3df84dc381c2
STATUS: superseded index - do not implement this file directly
CURRENT_ACTION: complete canonical module Phase 5, then execute the split plans in dependency order
```

The former combined plan was split because its grammar work and resource work have different prerequisites and ownership boundaries.

## Replacement plans

### 1. Dependency clauses and path syntax

[`dependency-clauses-and-path-syntax-plan.md`](./dependency-clauses-and-path-syntax-plan.md)

Runs immediately after canonical module Phase 5 Gate D.

Owns:

- one file-owned retained path syntax table
- one `DependencyShellId` per authored top-level clause
- coherent dependency terminology
- removal of the source `import` keyword
- extensionless source/package clauses
- registered explicit-extension provider clauses
- grouped and filtered namespace binding
- explicit re-export rules
- repository, docs and editor-grammar syntax migration

It does not implement builtin `Path`, stable resource identity, TIR resource anchors, asset emission or resource invalidation.

### 2. Path values and resource linking

[`path-values-and-resource-linking-plan.md`](./path-values-and-resource-linking-plan.md)

Runs after:

1. canonical module Phase 5 Gate D
2. dependency clauses and path syntax
3. TIR corrections and simplification

Owns:

- builtin compile-time `Path`
- stable module/provider resource origins
- module-local resource IDs and build-owned source records
- structural resource anchors through AST, TIR, HIR and link facts
- public Path values and resource-bearing templates
- exact entry and package resource unions
- builder placement and containing-artefact-relative URLs
- provider-managed and generated resources
- deletion of eager rendered-path asset reconstruction

The resource-only dev fast path is deliberately deferred until the core resource model and successful resource state are accepted.

## Locked boundary between the plans

A file-local dependency clause and a project dependency declaration are different facts.

```text
ProjectDependencyDeclaration
-> makes one external package available to the project resolver and Stage 0 namespace

FileDependencyClause
-> binds names from an already registered source/package/provider root into one source file
```

Source dependency clauses never add undeclared external packages implicitly.

The design-gated package dependency plan owns project-level declaration and resolver policy:

- [`package-dependency-declarations-and-manager-foundations-plan.md`](./package-dependency-declarations-and-manager-foundations-plan.md)

It must not restore the removed source `import` grammar or let parser implementation invent a config surface.

## Shared invariants

Both replacement plans preserve these rules:

- `@` starts a Moth project or package path
- `@./...`, parent components, `@/...` and `@@name` are invalid
- dependency and resource resolution starts from the owning module root, never the physical source-file directory
- child modules and support-package roots are ownership boundaries
- paths and output URLs are not semantic identity
- source preparation does not rescan arbitrary strings or rendered output
- builders never parse source to rediscover dependencies or resources
- old and new production paths cannot coexist after an accepted cutover

## Implementation order

```text
canonical module Phase 5 Gate D
-> dependency clauses and path syntax
-> TIR corrections and simplification
-> Path values and resource linking
-> later link/backend/reuse plans consume final resource facts
```

Do not mark either replacement plan active from this index. Use each plan's current-state block and prerequisite gates.
