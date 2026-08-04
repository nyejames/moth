# Path values, dependency clauses and resource linking implementation plan

## Purpose

Replace Moth's current `import` grammar and eager rendered-path asset tracking with one coherent path model:

- extensionless top-level `@...` clauses bind source or provider dependencies
- explicit-extension `@...ext` expressions create compile-time `Path` values
- opaque resources are values, never imports
- resources obey module and package visibility
- resource identity remains structural through templates, public interfaces, HIR and link planning
- builders choose final output locations and relative URLs
- opaque resource byte changes can re-emit output without semantic recompilation
- no compiler or builder scans arbitrary strings to rediscover dependencies

This is the immediate semantic follow-up to:

- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md`

Implementation must not begin until that plan reaches its mandatory Phase 5 handoff. TIR resource work must also begin from the accepted result of:

- `docs/roadmap/plans/tir-corrections-and-simplification-plan.md`

The plan is intentionally breaking. Moth is pre-release. There is no compatibility parser, feature flag, dual resource pipeline or migration period.

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/path-values-dependency-clauses-and-resource-linking-plan.md`

CURRENT_SLICE:
- Phase: Phase 0A plan adoption
- Checklist item: add this plan and queue it after TIR cleanup, then pause
- Goal: preserve the accepted design and current repository baseline while prerequisites complete
- Non-goals: code implementation, partial terminology migration, resource IDs or new source syntax

LAST_GOOD_COMMIT:
- `909d41660b198db5f39e7d822282a107e69be118`
- Remote `main` at the final repository refresh on 2026-08-04
- This commit reports R5C5B complete with `just validate` green and Gate A next

CURRENT_WORKTREE_STATE:
- Clean / known changes: unknown from the read-only GitHub refresh. Record the real local status before activation.
- Branch: remote default branch `main`
- Dedicated worker worktrees: none recorded for this plan. Create one only after prerequisites pass.

RELEVANT_DOCS_THIS_SLICE:
- `AGENTS.md`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/language-overview.md`
- `docs/src/docs/codebase/language/overview.mtf`
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md`
- `docs/roadmap/plans/tir-corrections-and-simplification-plan.md`
- `docs/src/docs/progress/@page.moth`

RELEVANT_CODE:
- `src/compiler_frontend/tokenizer/tokens.rs`: current `TokenKind::Path`, `PathTokenItem` and last-writer body modes
- `src/compiler_frontend/paths/const_paths/`: current path grammar, grouped expansion and `import` clause parsing
- `src/compiler_frontend/paths/compile_time_paths.rs`: current absolute-path and eager public-path value
- `src/compiler_frontend/paths/rendered_path_usage.rs`: current module-wide rendered-path side channel
- `src/compiler_frontend/headers/`: import shells, public re-exports, namespace binding and prepared syntax
- `src/build_system/create_project_modules/`: Stage 0 discovery, graph edges, provider scheduling and boundary handoff
- `src/compiler_frontend/ast/expressions/expression_kind.rs`: test-only `ExpressionKind::Path`
- `src/compiler_frontend/folded_value.rs`: public folded values and const-template pieces
- `src/compiler_frontend/ast/templates/tir/`: structural template authority
- `src/compiler_frontend/ast/templates/runtime_handoff.rs`: AST-to-HIR runtime-template vocabulary
- `src/compiler_frontend/hir/hir_expression/templates/`: runtime template string lowering
- `src/compiler_frontend/hir/reachability.rs`: per-function link and reachability facts
- `src/builder_surface/external_import_providers/`: current provider contract and `PathBuf` runtime-asset identity
- `src/projects/html_project/tracked_assets.rs`: current HTML-only tracked asset planning
- `src/projects/html_project/html_project_builder.rs`: current per-entry asset planning and emission
- `src/projects/html_project/style_directives.rs`: current balanced `$css` and `$code` registration
- `src/projects/dev_server/watch.rs`: revision-only watch reporting
- `src/projects/dev_server/build_loop.rs`: current full compiler rebuild path
- `src/projects/dev_server/static_files.rs`: current content-type table
- `tests/cases/`: user-visible syntax, diagnostics and emitted-output contracts
- `nyejames/moth-vscode-highlighting`: external TextMate grammar and import-group tests

ACCEPTANCE_CRITERIA:
- one source path grammar and one retained path-syntax table
- no `import` keyword in current source syntax
- no opaque resource import form
- production `Path` is compile-time-only and enforced transitively
- every authored resource path is validated once during preparation
- only reachable resource uses are emitted
- stable resource identity crosses modules without absolute paths
- template resource anchors survive every compile-time and runtime path
- exact per-entry and package resource unions drive builder output
- final URLs are relative to their containing output artefact
- provider source files and provider runtime assets share explicit stable resource identities
- opaque byte-only changes can use the dev resource fast path
- old rendered-string asset reconstruction is deleted
- docs, tests, examples and syntax highlighting use the new grammar
- all phase audits and final validation pass

DECISIONS_ALREADY_MADE:
- decision: remove `import` in one breaking cutover
  - reason: one path syntax and one dependency mental model
  - source/user/date: user interview, 2026-08-04
- decision: extensionless top-level paths are source or package dependencies
  - reason: source-bearing targets use strict module visibility
  - source/user/date: user interview, 2026-08-04
- decision: explicit-extension paths are compile-time resource values
  - reason: resource use must remain explicit, typed and trackable
  - source/user/date: user interview, 2026-08-04
- decision: opaque resources cannot be imported
  - reason: resources are values while dependency clauses bind semantic interfaces
  - source/user/date: user interview, 2026-08-04
- decision: `Path` is compile-time-only, including aggregates that contain it
  - reason: final URL depends on builder output placement
  - source/user/date: user interview, 2026-08-04
- decision: all authored resource paths validate early, but emission is reachability-driven
  - reason: prepare once without copying unused resources
  - source/user/date: user interview, 2026-08-04
- decision: cross-module resources travel through exported `Path` values
  - reason: reuse ordinary module visibility rather than inventing asset visibility
  - source/user/date: user interview, 2026-08-04
- decision: builder URLs are relative to final containing artefacts
  - reason: routes, standalone CSS and deployment subpaths must remain correct
  - source/user/date: user interview, 2026-08-04
- decision: provider source files are not automatically emitted
  - reason: provider semantics, implementation source and runtime assets are distinct
  - source/user/date: user interview, 2026-08-04
- decision: `$css` becomes normally composable and `$literal` owns literal square brackets
  - reason: resource anchors and nested templates must work in CSS by default
  - source/user/date: user interview, 2026-08-04
- decision: plain Markdown links remain literal and untracked
  - reason: `.md` has no Moth scope and must not gain heuristic string scanning
  - source/user/date: user interview, 2026-08-04

BLOCKERS / RISKS:
- canonical module Phase 5 is still active at the planning baseline
- TIR cleanup is queued and owns structural walker consolidation needed before a new resource node
- resource-bearing strings touch AST, TIR, public interfaces, HIR and builder planning
- direct grammar cutover affects every Moth source file plus the external highlighting repository
- current dev watching reports only revisions, not changed paths
- current provider runtime-asset identity embeds canonical filesystem paths
- package output naming must remain stable when package-manager identities arrive later

VALIDATION_STATE:
- last command: reported `just validate` at `909d41660b198db5f39e7d822282a107e69be118`
- result: reported green by the commit, including workspace tests, 1817 integration cases, Clippy, docs and bench-ci
- known unrelated failures: none reported
- this plan creation did not rerun compiler validation

DOCS_IMPACT:
- progress matrix needed: yes, first as accepted deferred design then as implementation status changes
- other docs stale: compiler design, build-system design, language overview, package/import docs, template docs, `.mtf` docs and dev/build docs
- authorized docs updates: all affected canonical and teaching docs, roadmap, progress matrix and generated release docs through the build

NEXT_ACTION:
- copy this plan into the repository and update the roadmap as a queued docs-only slice
- run the documentation-only gate and pause
- after both prerequisite plans are accepted, refresh this capsule from current `main` and execute Phase 0B

Refresh this capsule after every accepted slice and immediately before context compaction. Never continue from a stale capsule after another branch has changed the same owners.

## Current repository baseline

At `909d41660b198db5f39e7d822282a107e69be118`:

- the canonical module plan has completed R5C5B and is waiting for Gate A
- `TokenKind::Import` is still a keyword and header classification input
- paths are tokenized as expanded `Vec<PathTokenItem>` payloads
- grouped path syntax is flattened during tokenization
- the path parser still accepts the special public-root form `@/`
- Stage 0 still has import-oriented scanners and retained import terminology
- `CompileTimePath` carries absolute `PathBuf`, eager public paths, directories and `RelativeToFile`
- `ExpressionKind::Path` is test-only
- `RenderedPathUsage` is retained as module compiler metadata
- the HTML builder plans tracked assets per module and route from that metadata
- public folded values have no stable resource value or resource template piece
- TIR and runtime handoff have no resource node
- `$css` and `$code` use `TemplateBodyMode::Balanced`
- template body mode is mutable last-writer state
- providers identify runtime assets with canonical source paths
- the dev server watches broad scopes and reports a revision, then always invokes a full build
- the dev server MIME table lacks modern font and image mappings
- the external VS Code grammar still has an import-group grammar and `import` keyword rules

This baseline is navigation, not authority. Reinspect all named owners after the canonical and TIR prerequisite plans finish.

## Activation and sequencing

The implementation chain is:

```text
canonical module Phase 5 Gate D
-> TIR corrections and simplification acceptance
-> this plan
-> later link/backend/reuse plans consume final resource facts
```

Activation rules:

- [ ] Do not mark this plan active before canonical module Phase 5 reaches its mandatory handoff.
- [ ] Do not add `TemplateIrNodeKind::Resource` before the TIR cleanup's accepted preparation, folding and walker owners exist.
- [ ] Refresh `main`, this capsule, the implementation map and test counts on activation.
- [ ] Record any renamed or moved owner from the prerequisite plans.
- [ ] If the prerequisites already implemented part of this design, consolidate into that owner rather than adding another path.
- [ ] If either prerequisite changed the agreed language surface, stop for user review. Do not silently reinterpret the interview decisions.

## Final language contract

### One `@` mental model

`@` always starts a Moth project or package path.

The syntactic context plus the final extension determines the operation:

| Form | Context | Meaning |
|---|---|---|
| `@core/math` | top level | bind a source/package namespace named `math` |
| `@core/math as maths` | top level | bind the whole namespace as `maths` |
| `@core/math { sin, cos }` | top level | bind selected names directly |
| `@core/math as maths { sin, cos }` | top level | bind a filtered namespace as `maths` |
| `@drawing.js { draw }` | top level | bind selected names through a registered provider |
| `@drawing.js as drawing` | top level | bind the whole provider interface |
| `@drawing.js as drawing { draw }` | top level | bind a filtered provider namespace |
| `@images/logo.svg` | expression | produce one compile-time `Path` value |
| `[@images/logo.svg]` | template head | insert one structural resource anchor |
| `logo #= @images/logo.svg` | declaration | define a compile-time resource value |
| `"https://example.com/logo.svg"` | expression | ordinary untracked `String` |
| `"/favicon.svg"` | expression | ordinary untracked site-root string |

`@` does not imply a module-root filename marker in source. `@@name`, `@./...`, parent components and `@/...` are invalid.

### Dependency clause grammar

Conceptual grammar:

```text
source_dependency_clause
    = extensionless_path
    | extensionless_path "as" identifier
    | extensionless_path selection_group
    | extensionless_path "as" identifier selection_group

provider_dependency_clause
    = explicit_extension_path selection_group
    | explicit_extension_path "as" identifier
    | explicit_extension_path "as" identifier selection_group

selection_group
    = "{" selection_entry ("," selection_entry)* [","] "}"

selection_entry
    = path_component ["as" identifier] [selection_group]
```

Rules:

- [ ] Dependency clauses are valid only at `.moth` file top level and inside a module root's `export:` block.
- [ ] `config.moth` rejects every dependency clause before path resolution.
- [ ] `.mtf` files cannot declare dependencies.
- [ ] A source dependency path has no explicit source extension.
- [ ] `.moth`, `.mtf` and `.md` are always selected extensionlessly.
- [ ] A bare extensionless clause creates a namespace whose inferred name is the final valid identifier component.
- [ ] Invalid namespace stems require `as`.
- [ ] A selection group without clause-level `as` binds its selected leaves or child namespaces directly.
- [ ] Clause-level `as` plus a group creates one filtered namespace.
- [ ] A namespace alias always has one provider origin.
- [ ] Nested groups select real child namespaces from that same provider interface.
- [ ] A leading `@` is invalid inside a selection group.
- [ ] Empty groups are invalid.
- [ ] Any selected entry may use `as` to rename a leaf or filtered child namespace.
- [ ] A selected child namespace still belongs to the clause's one provider origin.
- [ ] Duplicate selected paths and duplicate local names are diagnosed.
- [ ] Dependency bindings are file-local, prepared before body semantics and visible throughout their owning file independent of clause source position.
- [ ] Direct symbol paths such as `@core/math/sin` remain invalid. Select `sin` through a group.
- [ ] A whole provider namespace always requires `as`.
- [ ] A provider clause with a group may bind selected names directly.
- [ ] A bare explicit-extension clause is invalid when it has neither `as` nor a group.
- [ ] An explicit-extension top-level clause is valid only when the active builder registered a semantic provider for that extension.
- [ ] An explicit-extension clause with no provider gets a targeted diagnostic instructing the author to use the path as a value.
- [ ] Recognized source extensions get a targeted extensionless-source diagnostic, not the opaque-resource diagnostic.
- [ ] `=` and `#=` remain value-binding syntax and never bind dependency namespaces.
- [ ] Namespace records are static visibility facts, not first-class values. They cannot be passed, returned, stored or constructed by merging providers.
- [ ] Consumer-side mixed-origin namespace aggregation is outside this implementation.

Inside `export:`:

- [ ] only explicit non-empty selection groups are valid dependency re-exports
- [ ] whole namespaces cannot be re-exported implicitly
- [ ] clause-level `as` is invalid
- [ ] leaf aliases define the public API names
- [ ] nested groups may qualify leaf declarations but do not export namespace-record values
- [ ] aliases on non-leaf selection groups are invalid inside `export:` because namespace exports remain deferred
- [ ] receiver methods remain attached to exported receiver types and are not selected separately
- [ ] re-exports preserve declaration and resource origins

### Removed `import` keyword

The migration is one-way:

```moth
-- Removed
import @core/math { sin }

-- Current
@core/math { sin }
```

Rules:

- [ ] Remove `TokenKind::Import` and the keyword entry.
- [ ] Lowercase `import` becomes an ordinary identifier.
- [ ] `import = 1` is parsed as an ordinary binding when otherwise valid.
- [ ] A symbol spelled `import` followed by an old path-clause shape gets a dedicated removed-syntax diagnostic.
- [ ] The diagnostic spans `import` and the clause, includes the replacement form and uses a new stable diagnostic code.
- [ ] Do not retain an old parser, warning-only acceptance or hidden compatibility mode.
- [ ] `#Import` build-input syntax is unrelated and remains unchanged.
- [ ] Backend JavaScript imports, WIT imports and `RequiredRuntimeImport` keep their domain-appropriate terminology.

### `Path` value grammar

A resource path expression:

- is one path
- has an explicit non-source extension on its final component
- resolves to an existing regular file
- is module-root-relative
- is not grouped
- is not a directory
- is not `@/`
- is not external URL syntax

Examples:

```moth
logo #= @images/logo.svg
font #Path = @fonts/cmunss.woff2

icons #{Path} = {
    @icons/add.svg,
    @icons/remove.svg,
}
```

The following are invalid:

```moth
thing #= @CNAME
thing #= @docs/intro.mtf
thing #= @images
thing #= @images {logo.svg, hero.webp}
thing #= @/favicon.svg
thing #= @./logo.svg
thing #= @../logo.svg
```

Extensionless generated files such as `CNAME` remain builder-owned output features. A future explicit resource escape syntax is deferred.

### `Path` type semantics

`Path` is a reserved builtin compile-time type.

Allowed:

- `#Path` constants
- inferred Path constants
- transparent aliases to `Path`
- const-record or nominal record fields containing `Path`
- compile-time collections of `Path` or records that contain `Path`
- compile-time field access and collection iteration
- exported folded constants
- exported public nominal record types whose fields are public and compile-time-only
- direct template insertion
- compile-time template control flow

Disallowed in V1:

- mutable Path bindings
- ordinary runtime bindings
- runtime struct or choice instances containing Path
- receiver methods on compile-time-only Path-containing records
- function parameters or returns containing Path
- body locals that would require runtime Path storage
- options, choices, maps or generic applications containing Path
- map keys
- equality or ordering
- operators
- casts
- string construction from a runtime value
- implicit `Path -> String` assignment
- runtime filesystem access
- config values
- provider namespace values

Type checking must use one recursive capability classifier, conceptually:

```rust
pub enum TypeAvailability {
    RuntimeStorable,
    CompileTimeOnly,
    NonValue,
}
```

Requirements:

- [ ] `Path` is `CompileTimeOnly`.
- [ ] A record or collection is `CompileTimeOnly` when any contained type is compile-time-only.
- [ ] Aliases preserve the target's classification.
- [ ] Runtime receiving boundaries reject compile-time-only types through one diagnostic owner.
- [ ] A non-constant binding whose inferred value contains Path gets a targeted "Path values require a compile-time binding" diagnostic.
- [ ] The diagnostic recommends `#=` or `#Path` where that correction is valid.
- [ ] A mutable declaration gets a separate reason explaining that Path values cannot be mutable.
- [ ] Generic type arguments reject compile-time-only types in V1.
- [ ] Do not encode compile-time availability into nominal identity or create parallel Path-containing type enums.

### Resource ownership and visibility

Every resource has one owning package and module.

A direct Path literal may address only a regular file inside the current module or package's private filesystem ownership.

Rules:

- [ ] Resolve from the source file's owning module root, never its physical directory.
- [ ] Ordinary unrooted directories owned by that module may be traversed.
- [ ] Reaching a child normal module or support package stops filesystem traversal.
- [ ] Another module's private resource path cannot be addressed directly.
- [ ] Parent, ancestor, sibling and unrelated-branch access remains invalid.
- [ ] Support-package visibility follows the existing scoped-package rules.
- [ ] The project package facade does not become an internal global resource escape.
- [ ] Cross-module resources are shared only through ordinary exported Path constants or resource-bearing exported const templates.
- [ ] Resource files themselves do not acquire a public visibility table.
- [ ] Re-exporting a Path constant changes public binding, not resource origin.
- [ ] A physical resource root outside `entry_root` is not supported in V1.
- [ ] Future configured resource-package roots are deferred.
- [ ] Canonical containment rejects a target that escapes the owning project/package root through symlinks.
- [ ] Case validation reuses the existing strict source-path policy.
- [ ] Logical aliases to one canonical file remain distinct resource origins. Normal output conflict and duplicate rules apply.

Example:

```moth
-- src/assets/+assets.moth
export:
    logo #= @logo.svg
    cat #= @animals/cat.png
;

-- src/@home.moth
@assets {
    logo,
    cat as cat_picture,
}

#[img, logo]
#[img, cat_picture]
```

### Resource dependency and emission liveness

A resource has two independent liveness states.

**Dependency liveness**

Every authored resource literal is resolved and validated during the single preparation pipeline, including literals in:

- unused constants
- compile-time branches that later fold away
- unmaterialised generic templates
- resource-bearing helper templates that are never rendered

This establishes:

- stable resource origin
- source ownership
- existence and regular-file kind
- canonical source path for build IO
- watch registration
- source location for diagnostics

**Emission liveness**

A resource is emitted only when a structural resource anchor reaches a selected entry or package output through:

- a compile-time page fragment
- dormant root runtime work
- a reachable source function
- a reachable generated function
- a reachable provider runtime requirement
- a selected package artefact

An unused Path constant creates no output file.

Runtime branch reachability remains the compiler's current syntactic CFG reachability. Do not add constant-condition tree shaking to asset planning.

### Resource-bearing strings

A template that inserts a Path still has language type `String`, but its internal value is not a flat string until output placement is known.

Rules:

- [ ] A Path inserted into a template becomes a structural resource piece.
- [ ] Const folding may produce a builder-deferred resource string, not an ordinary `StringLiteral`.
- [ ] Resource-bearing strings may be composed, stored in constants, exported, imported, passed as String values and returned as String values.
- [ ] Their resource pieces survive until builder-specific URL rendering.
- [ ] Compile-time operations that require final string content cannot inspect a resource-bearing string.
- [ ] Such operations produce a targeted diagnostic in a const-required context.
- [ ] Runtime operations may operate after the resource pieces lower to entry-specific string literals.
- [ ] Do not define Path equality as resource identity or output URL equality.
- [ ] Do not make builder output paths observable through a compile-time cast.

### Moth template resource paths

`.mtf` remains declarationless and importless. Resource use happens only through ordinary nested Moth template syntax inside its implicit Markdown body.

```moth
[$html:
    <img src="[@images/ownership.webp]" alt="Ownership graph">
]
```

Rules:

- a plain `@images/ownership.webp` spelling in Markdown body text is ordinary text, not a Path expression
- a Path in a nested template head resolves from the `.mtf` file's owning module root
- the restricted same-directory module-root constant scope may expose exported Path constants and resource-bearing const templates
- `@html` compile-time template helpers may receive Path values through ordinary head and slot composition without making Path a function parameter type
- `.mtf` gains no dependency declarations, frontmatter or general Moth declarations
- all authored nested resource literals still participate in preparation-time dependency validation

### External and site-root URLs

These remain ordinary strings:

```moth
external #= "https://example.com/logo.svg"
cdn #= "//cdn.example.com/app.js"
site_root #= "/favicon.svg"
```

They are:

- not existence checked
- not copied
- not rewritten
- not watched as resources
- not included in resource unions

No compiler or builder scans String, HTML, CSS or Markdown text for URL-shaped dependencies.

### Final output paths and URLs

The resource source identity and emitted output path are separate.

Default output placement:

- project-local resource: preserve its path relative to `entry_root`
- source/dependency package resource: prefix the stable package output identity, then preserve the package-relative path
- provider-managed resource: use the provider's declared stable output path
- generated provider resource: use the provider's declared path and generated bytes

The package prefix encoder must be a single build-system owner derived from `StablePackageIdentity`. Current source packages use their stable import/package name. Future versioned dependency identities may extend that encoder without changing Moth source syntax.

URL rendering:

1. select the final containing output artefact
2. select the canonical resource output path
3. compute the lexical relative path from the containing artefact's parent
4. use `/` separators
5. percent-encode each UTF-8 path segment
6. prefix same-or-descendant paths with `./`
7. leave parent-relative paths as `../...`
8. never prepend HTML `origin`

Examples:

```text
resource output: assets/logo.svg
containing HTML: index.html
URL: ./assets/logo.svg

resource output: assets/logo.svg
containing HTML: docs/getting-started/index.html
URL: ../../assets/logo.svg

resource output: styles/fonts/site.woff2
containing CSS: styles/site.css
URL: ./fonts/site.woff2
```

Inline CSS is contained by the HTML artefact. Emitted standalone CSS is its own containing artefact. Runtime template strings selected for an HTML entry also use that entry's HTML document as their containing artefact, even when JavaScript constructs the string. A builder with no containing-artefact resource policy must reject reachable resource anchors during target validation.

Conflicts:

- one stable resource used by many entries emits once
- identical output path plus identical resource origin deduplicates
- identical output path plus different origin fails with both source locations
- resource output conflicting with HTML, JS, Wasm, manifest or provider output fails
- one source file used as both an ordinary Path and an unchanged provider runtime asset deduplicates only when provider identity and output path agree
- transformed or generated provider output is a separate resource identity

### `$literal` and template body syntax

Replace mutable last-writer body modes with an order-independent requirement merge:

```rust
pub enum TemplateBodySyntax {
    Template,
    Literal,
    Discard,
}
```

Rules:

- normal directives request no override from `Template`
- `$literal` requests `Literal`
- `$code` requests `Literal`
- `$note` and `$todo` request `Discard`
- repeated equal requirements are idempotent
- incompatible non-default requirements are diagnosed with both directive locations
- directive order cannot change body tokenization
- `$raw` remains whitespace policy, not literal-bracket policy
- `$css` and `$css("inline")` use normal `Template` body syntax
- `$css("raw")` is not added
- `$literal` treats square brackets as literal text and disables nested templates, expressions and resource anchors in that body

Examples:

```moth
font_css #= [$css:
    @font-face {
        font-family: "Computer Modern Sans";
        src: url("[@fonts/cmunss.woff2]") format("woff2");
    }
]

attribute_css #= [$css, $literal:
    input[type="email"] {
        border-color: green;
    }
]
```

### Plain Markdown

`.md` remains raw Markdown source:

- links and image targets stay literal
- no Moth path expressions
- no resource tracking
- no rewriting
- no implicit file copying

Resource-aware Markdown is deferred as an explicit source-kind feature. It must not be implemented by scanning rendered HTML.

## Compiler and build architecture

### Path syntax ownership

Replace expanded path payloads with one file-owned table.

Conceptual data:

```rust
pub struct PathSyntaxId(u32);

pub struct PathSyntaxTable {
    clauses: Vec<PathSyntax>,
    selections: Vec<PathSelectionNode>,
}

pub struct PathSyntax {
    root: InternedPath,
    alias: Option<StringId>,
    selection_range: Option<PathSelectionRange>,
    location: SourceLocation,
}

pub struct PathSelectionNode {
    component: StringId,
    alias: Option<StringId>,
    child_range: Option<PathSelectionRange>,
    location: SourceLocation,
}
```

Requirements:

- `TokenKind::Path(PathSyntaxId)` carries only a dense file-local handle.
- The table preserves authored tree shape and every alias location.
- Group syntax is not expanded into one full path allocation per leaf.
- Header preparation classifies top-level source/provider clauses.
- AST consumes expression Path syntax.
- The tokenizer does not decide semantic provider, module or resource ownership.
- `FileTokens` or its Phase 5 successor owns the table with the token vector.
- String-table remapping walks the table once.
- Source-identity rebinding updates table locations once.
- Frozen generic syntax copies/remaps referenced table rows through the canonical token vocabulary.
- No second stable path-token enum is introduced.
- No later phase reparses raw source or reconstructs group shape from flattened paths.

### Dependency facts

Use distinct typed phases and coherent terminology:

```text
ScannedDependencyClause
-> RetainedDependencyClause { DependencyShellId }
-> BoundDependencyClause
```

One authored top-level clause receives one `DependencyShellId`, not one ID per selected leaf.

Selected bindings use clause-local selection identities or indexes.

Coherent migration includes, where each name still reflects its role:

- `ImportShellId` -> `DependencyShellId`
- `FileImport` -> `FileDependencyClause`
- import-clause parser -> dependency-clause parser
- import visibility environment -> binding/visibility environment
- structural provider import -> structural provider dependency
- source import access -> source dependency access
- diagnostics and comments

Do not rename backend runtime-import concepts or source `#Import`.

### Resource identities and tables

Stable identity:

```rust
pub struct StableResourceOriginId {
    pub package: StablePackageIdentity,
    pub owner_module: StableModuleOriginIdentity,
    pub logical_path: PortableResourcePath,
}
```

It does not include:

- absolute filesystem path
- content hash
- output path
- alias
- export binding
- source location
- current route
- builder origin prefix

Module-local identity:

```rust
pub struct ResourceId(u32);
pub struct ResourceUseId(u32);
```

Module-local tables:

```rust
pub struct ModuleResourceTable {
    records: Vec<ModuleResourceRecord>,
    by_origin: FxHashMap<StableResourceOriginId, ResourceId>,
}

pub struct ModuleResourceRecord {
    origin: StableResourceOriginId,
    source_location: SourceLocation,
}
```

Build-owned source registry:

```rust
pub struct ResolvedResourceSource {
    origin: StableResourceOriginId,
    canonical_source_path: PathBuf,
    logical_source_path: PortableResourcePath,
    owner_root: PathBuf,
    content_fingerprint: ResourceContentFingerprint,
}
```

Rules:

- local compiler IR uses dense IDs
- public interfaces use stable resource origins
- build IO owns canonical paths
- equal stable origins must agree on source facts
- source bytes are hashed once per build state
- content fingerprint is based on bytes, not mtime
- resource content is not part of a public-interface fingerprint
- adding, removing or changing resource identity is a semantic/link fact change
- byte-only opaque changes are resource-output invalidation only

### Preparation and early validation

The single preparation pipeline must:

- classify every explicit-extension expression path
- reject grouped resource expressions
- resolve module/package ownership
- validate extension and source-kind exclusion
- validate file existence and regular-file kind
- canonicalize for containment and IO
- register stable origin and source location
- retain dependency/watch facts even when later folding removes the use
- freeze resource references in generic templates without donor-local IDs

User mistakes use structured `CompilerDiagnostic`. Filesystem failures and table disagreement use `CompilerError`.

### AST and type environment

Production AST adds a real Path expression:

```rust
ExpressionKind::Path(ResourceId)
```

The builtin `Path` TypeId participates in:

- type parsing
- diagnostic rendering
- canonical type identity
- alias projection
- const-value classification
- public interface type validation

AST owns the recursive compile-time-only type classifier and all receiving-boundary diagnostics.

### TIR and formatter boundary

After the TIR cleanup, add:

```rust
TemplateIrNodeKind::Resource {
    resource: ResourceId,
}
```

Update exhaustively:

- construction
- summaries
- preparation
- classification
- folding
- subtree copy
- slot composition
- wrapper handling
- formatter views
- cycle-safe walkers
- runtime slot plans
- const-template projection
- tests

Formatter input adds an opaque resource anchor kind. Formatters may preserve or reorder the anchor only according to their existing opaque-anchor contract. They cannot inspect filesystem identity or render the URL.

A resource node is structurally output-producing and non-reactive.

### Public folded values

Add an owned stable public resource reference:

```rust
pub struct PublicResourceRef {
    pub origin: StableResourceOriginId,
}
```

Extend the one folded-value vocabulary:

```rust
PublicFoldedValue::Path(PublicResourceRef)

PublicConstTemplatePiece::Resource(PublicResourceRef)
```

Rules:

- no absolute path crosses the interface
- nested records and collections visit resource refs recursively
- imported values project stable refs into consumer-local ResourceId values
- public-interface agreement compares stable refs structurally
- re-export closure follows existing declaration origins
- public-interface fingerprints include stable resource refs
- content fingerprints remain outside the semantic interface

### Generic materialisation

Generic bodies may contain resource literals even though `Path` is not a legal generic type argument.

Requirements:

- every authored generic-body resource resolves during declaring-module preparation
- frozen syntax retains stable resource refs or a compact frozen resource table
- materialisation projects them into the generated sidecar's local resource table
- generated link facts use generated-local ResourceId values
- an unmaterialised generic contributes no emission use
- a materialised but unreachable generated function contributes no entry use
- no declaring-package absolute path enters persistent template metadata

### Runtime handoff and HIR

Add:

```rust
OwnedRuntimeTemplateNode::Resource {
    resource: ResourceId,
    location: SourceLocation,
}
```

Update all immutable and mutable handoff walkers.

HIR must retain a resource append operation or equivalent structured expression. It must not receive an eagerly formatted URL.

Per-function link facts record resource uses in deterministic order. Dormant root and compile-time fragment metadata retain their resource pieces through entry assembly.

The builder supplies a validated resource URL map when lowering a physical entry/package variant.

Physical variant identity must include the resource URL mapping or an equivalent containing-artefact fingerprint. Two routes at different depths must not reuse code containing the wrong relative resource URL.

### Entry and package resource unions

For each selected entry or package assembly:

```text
entry roots
-> source/generated function reachability
-> dormant root and compile-time fragment facts
-> provider runtime requirements
-> exact stable resource-origin union
-> canonical output placement
-> containing-artefact URL map
```

Module-wide resource inventories are not the reachability authority.

### Provider resource contract

Rename the semantic extension point coherently, for example:

```text
ExternalImportProvider -> ExternalSourceProvider
ExternalImportRequest -> ExternalSourceDependencyRequest
ResolvedExternalImport -> ResolvedExternalSourceDependency
```

Exact names may differ, but the API must stop treating a runtime asset's canonical `PathBuf` as semantic identity.

Provider request includes:

- stable provider-source resource origin
- canonical source path for IO
- authored dependency clause location
- provider kind
- content fingerprint

Provider selection remains on the retained Moth dependency clause and is bound against the provider's completed interface. The provider parses one source interface independently of how many clauses select from it.

Provider result includes:

- typed semantic interface/package facts
- provider diagnostics
- stable runtime resource declarations
- generated resource declarations
- required runtime module imports

A provider runtime resource declaration includes stable identity, stable output path and either source bytes/file identity or generated bytes.

The provider source is emitted only when explicitly declared or separately used as a reachable ordinary Path.

Provider cache keys include provider kind, stable source origin and content fingerprint.

### Builder resource plan

Create one builder-facing resource plan owner rather than extending HTML-specific heuristics.

Conceptual outputs:

```rust
pub struct ResourceLinkPlan {
    resources: Vec<PlannedResource>,
    urls_by_use: Vec<RenderedResourceUrl>,
}

pub struct PlannedResource {
    origin: StableResourceOriginId,
    output_path: PathBuf,
    content_source: ResourceContentSource,
    content_fingerprint: ResourceContentFingerprint,
}
```

The HTML builder selects the containing artefact and policy. The central output writer still owns validation, unchanged-write skipping, manifests and stale cleanup.

Large-resource warnings are emitted once per stable resource per build, anchored at the first reachable use.

### Incremental and dev state

Extend build results with a successful resource state:

```rust
pub struct ResourceBuildState {
    sources_by_path: HashMap<PathBuf, ResourceSourceState>,
    outputs_by_origin: HashMap<StableResourceOriginId, ResourceOutputState>,
    emitted_uses: Vec<ResourceUseOutput>,
}
```

Change watcher reporting from revision-only to stable changed-path batches.

Resource-only fast path is allowed only when every changed path:

- belongs to a known opaque resource
- is not provider-backed
- still exists as the same regular-file target
- resolves to the same stable origin
- has unchanged logical path, ownership and output placement

Fast path:

```text
read changed bytes
-> recompute content fingerprint
-> rewrite affected OutputFile::Bytes through output ownership
-> update ResourceBuildState
-> broadcast reload only when emitted output changed
```

No tokenizer, header, AST, HIR, borrow or backend compile path runs.

Full rebuild is required for:

- source or config changes
- provider source changes
- new, removed or renamed resource files
- kind or containment changes
- path ownership changes
- output config or route changes
- a failed build without a trusted successful resource state
- unknown changed paths that may affect source discovery

Missing resource diagnostics must add a parent watch interest so creating the file retriggers a full build.

### Dev server serving

This plan adds only resource-serving support required by the resource implementation:

- `font/woff2`
- `font/woff`
- `font/ttf`
- `font/otf`
- `image/webp`
- `image/avif`
- `application/pdf`
- `application/javascript` for `.mjs`
- `application/json` for source maps
- existing fallback `application/octet-stream`

Add origin-mounted binary response tests and exact byte assertions.

Broader response metadata, caching policy, range requests, compression and a general dev-server hardening review are deferred to a separate plan.

## Diagnostics contract

Add typed reasons and stable codes for at least:

- removed `import` keyword
- dependency clause not allowed in this context
- source dependency must omit recognized extension
- explicit-extension top-level clause requires a registered provider
- opaque resource clause must be used as a value
- provider namespace requires `as`
- empty or malformed selection group
- invalid nested selection alias
- resource path requires an explicit extension
- source-kind path cannot be a resource
- resource path cannot be grouped
- removed public-root path syntax
- resource target missing
- resource target is not a regular file
- resource escapes owner boundary
- resource crosses a private module/package boundary
- Path requires a compile-time binding
- Path cannot be mutable
- Path-containing type cannot cross a runtime boundary
- unsupported Path aggregate shape
- resource-bearing string requires final output placement
- incompatible template body syntax requirements
- resource output collision
- provider resource identity disagreement

Requirements:

- allocate new codes through the diagnostic registry
- never repurpose old import codes for changed semantic families
- retain exact authored path and source location
- collision diagnostics show both owners
- migration diagnostics include a rendered replacement
- infrastructure disagreement uses `CompilerError`
- no user path failure panics

## Data-oriented and performance requirements

- dense local IDs instead of repeated path payloads
- one path syntax tree per authored path
- one resource source record per stable origin
- one content hash per source per build state
- one emitted byte read per deduplicated resource
- sorted contiguous final resource vectors
- narrow construction maps dropped or retained only where repeated queries justify them
- no `PathBuf` per resource use
- no URL String stored in semantic identity
- no full resource-table clone per entry
- no resource scan over every compiled module when exact reachability exists
- no formatter access to resource tables
- no arbitrary HTML/CSS/Markdown scanning
- no extra source tokenization or parsing

Add counters for:

- path syntax rows
- dependency clauses
- authored resource dependencies
- resolved resource sources
- resource anchors
- entry resource uses
- emitted unique resources
- deduplicated uses
- bytes hashed
- bytes read for output
- resource-only dev updates
- compiler invocations avoided
- fallback full rebuild reasons

## Work protocol

### Branch and worktree

- [ ] Create a dedicated worktree and branch after activation.
- [ ] Use a branch such as `agent/path-resource-linking`.
- [ ] Keep one coordinator as the only writer in that worktree.
- [ ] Auditors are read-only.
- [ ] Do not mix unrelated roadmap work into the branch.
- [ ] Rebase or merge current `main` only at phase boundaries.
- [ ] Refresh the capsule after integration changes.

### Checkpoints

Each phase ends with:

- [ ] focused tests
- [ ] style-guide review
- [ ] ownership and duplication audit
- [ ] progress-matrix review
- [ ] required validation
- [ ] a terse checkpoint commit
- [ ] capsule refresh
- [ ] parent review before the next phase

Phases 3A and 3B are internal vertical-cutover checkpoints. They must not merge to `main` until Phase 3C deletes the old rendered-path lane.

### Stop conditions

Stop and request review when:

- a second durable resource or path representation appears necessary
- a public identity would need an absolute path, output path or content hash
- a formatter would need resource internals
- a builder would need to parse source or inspect arbitrary text
- Path becomes runtime-storable to solve an implementation problem
- a route-specific URL would enter a module public interface
- a compatibility parser or fallback appears necessary
- a phase requires more than two unlisted stage-boundary changes
- an accepted TIR owner would need to be bypassed
- focused tests cannot isolate the intended owner
- a user failure would need `CompilerError`
- the same resource bytes are read or hashed repeatedly without a lifecycle reason

## Phase 0 - plan adoption, authority and activation baseline

### Context

Phase 0A may run immediately as a documentation-only adoption slice. It queues the accepted design without claiming implementation support. Phase 0B runs only after the canonical module and TIR prerequisites are accepted, then refreshes the implementation baseline and marks this plan active.

### Phase 0A - adopt and queue now

- [ ] Add this plan to `docs/roadmap/plans/`.
- [ ] Add this plan immediately after TIR cleanup in the queued implementation chain.
- [ ] Remove completed plans from `Active implementation work`.
- [ ] Correct the canonical module plan's roadmap status.
- [ ] Update `docs/language-overview.md` with the accepted end-state grammar and Path rules.
- [ ] Update `docs/compiler-design-overview.md` with stable resource identity, public folded resources, TIR/HIR anchors and per-function resource facts.
- [ ] Update `docs/build-system-design.md` with resource ownership, output placement, provider rules, exact unions and invalidation.
- [ ] Add a Deferred progress-matrix row for direct dependency clauses, Path values and managed resources.
- [ ] Keep current import rows truthful until the compiler cutover.
- [ ] Record managed `.md` resource links as deliberately deferred in the existing Markdown row.
- [ ] Record all deferred follow-ups from this plan in the roadmap section defined below.
- [ ] Do not change executable `.moth` or `.mtf` examples to unsupported syntax yet.
- [ ] Run the docs-only gate, checkpoint the plan adoption and pause.

### Phase 0B - activate after prerequisites

- [ ] Verify canonical module Phase 5 Gate D is accepted.
- [ ] Verify the TIR corrections plan is accepted.
- [ ] Refresh remote and local `main`.
- [ ] Record the actual `LAST_GOOD_COMMIT`, branch and worktree status.
- [ ] Reinspect every `RELEVANT_CODE` owner and update moved paths in the capsule.
- [ ] Resolve any documentation drift introduced by the prerequisite plans.
- [ ] Mark this plan active only after the refreshed architecture review is clean.

### Audit and style review

- [ ] Confirm roadmap plans do not override architecture authorities.
- [ ] Confirm the progress matrix reports current behavior, not accepted future behavior.
- [ ] Confirm no generated `docs/release/**` file was edited manually.
- [ ] Confirm all examples labeled current still use current syntax.

### Validation

Documentation-only gate:

```bash
moth build docs --release
```

or the equivalent Cargo invocation.

Inspect changed routes and generated diffs.

### Exit gate

For Phase 0A:

- [ ] Authorities contain the complete agreed design.
- [ ] Roadmap order is correct.
- [ ] Matrix truthfully says Deferred.
- [ ] The plan remains queued and inactive.
- [ ] Stop until prerequisites complete.

For Phase 0B:

- [ ] Capsule names the real implementation baseline.
- [ ] Moved owners and test counts are current.
- [ ] The plan is marked active.
- [ ] Stop for activation review.

## Phase 1 - one retained path syntax and dependency owner

### Context

Before changing source grammar, remove the current expanded path payload and token-rescan architecture. Preserve current source behavior for this phase only. This is an internal ownership cutover, not a compatibility layer.

### Checklist

- [ ] Introduce file-local `PathSyntaxId` and `PathSyntaxTable` or equivalent dense owners.
- [ ] Preserve path root, alias, selection tree and exact locations without leaf expansion.
- [ ] Change `TokenKind::Path` to carry the dense handle.
- [ ] Thread the table through `FileTokens` or its accepted successor.
- [ ] Remap path syntax rows once during string-table merge.
- [ ] Rebind source scopes once during source identity rebinding.
- [ ] Update frozen generic-body capture and materialisation to carry referenced path rows.
- [ ] Introduce `DependencyShellId` with real `FileId` plus clause ordinal.
- [ ] Give one shell ID to one authored top-level clause.
- [ ] Replace `FileImport` with one retained dependency-clause record.
- [ ] Replace scanned/retained provider import names with dependency terminology.
- [ ] Make Stage 0 consume retained prepared dependency clauses.
- [ ] Delete token rescanning through `collect_provider_references_from_tokens`.
- [ ] Preserve current `import @...` user grammar until Phase 2.
- [ ] Keep external runtime-import and `#Import` names unchanged.
- [ ] Update counters to count clauses and selection nodes separately.
- [ ] Update `index.md` if module/file ownership moves.

### Tests

- [ ] Path table remapping and source rebinding.
- [ ] Deep nested selection tree preservation.
- [ ] Alias location preservation.
- [ ] One shell per clause with many selected leaves.
- [ ] Stage 0 uses prepared facts and performs no token rescan.
- [ ] Frozen generic paths round-trip through one canonical token vocabulary.
- [ ] Existing import integration suite remains unchanged.

### Audit and style review

- [ ] No second path syntax vocabulary.
- [ ] No broad generic visitor hides stage ownership.
- [ ] `mod.rs` files remain structural maps.
- [ ] Test support does not leak into production.
- [ ] Current behavior remains one path, not old plus new.

### Validation

```bash
cargo fmt --all
just validate
```

### Exit gate

- [ ] One path syntax table owns every authored Moth path.
- [ ] One retained dependency-clause owner feeds Stage 0 and binding.
- [ ] No raw token rescan remains.
- [ ] Stop for review.

## Phase 2 - breaking dependency grammar cutover

### Context

Remove `import`, implement direct top-level path clauses and migrate the whole repository in one accepted slice. There is never a tree that accepts both grammars.

### Checklist

- [ ] Remove `TokenKind::Import`.
- [ ] Remove `import` from keyword classification, token stats and compiler-owned highlighter roles.
- [ ] Parse extensionless path clauses directly at top level.
- [ ] Parse provider explicit-extension clauses through the agreed forms.
- [ ] Implement whole namespace, namespace alias, direct selection and filtered namespace semantics.
- [ ] Enforce non-empty groups, one provider origin and no `@` inside groups.
- [ ] Enforce explicit grouped re-exports inside `export:`.
- [ ] Preserve leaf aliases as public names.
- [ ] Reject clause-level namespace aliases inside `export:`.
- [ ] Implement the targeted removed-`import` diagnostic.
- [ ] Prove `import` can be used as an ordinary identifier.
- [ ] Keep `#Import` unchanged.
- [ ] Reject dependency clauses in `config.moth`.
- [ ] Keep `.mtf` declarationless.
- [ ] Update namespace collision and casing diagnostics.
- [ ] Migrate every `.moth`, `.mtf`, fixture, package, benchmark and generated-test source in the repository.
- [ ] Migrate canonical language docs and teaching examples required for docs compilation.
- [ ] Rebuild generated docs from source.
- [ ] Update the built-in `$code("moth")` keyword and path-context tests.
- [ ] Update `nyejames/moth-vscode-highlighting` on its own branch:
  - [ ] remove import-keyword grammar
  - [ ] replace import-group grammar with dependency-clause grammar
  - [ ] update `.moth` and `.mtf` fixtures
  - [ ] update grammar regression tests
  - [ ] run `npm test`
  - [ ] run `npm pack --dry-run`
- [ ] Search for obsolete current-syntax `import @` examples and classify intentional migration tests separately.
- [ ] Update the progress matrix dependency syntax status.

### Integration contracts

- [ ] bare extensionless namespace
- [ ] aliased namespace
- [ ] grouped direct bindings
- [ ] filtered namespace
- [ ] aliased filtered child namespace
- [ ] nested real provider namespace
- [ ] grouped public re-export
- [ ] provider grouped clause
- [ ] provider namespace clause
- [ ] invalid bare provider clause
- [ ] invalid providerless explicit-extension clause
- [ ] invalid explicit source extension
- [ ] invalid empty group
- [ ] invalid mixed-origin nested `@`
- [ ] old syntax migration diagnostic
- [ ] ordinary identifier named `import`
- [ ] config and `.mtf` rejection
- [ ] receiver method visibility parity

### Audit and style review

- [ ] No old parser or fallback.
- [ ] No user-visible `Import` terminology remains where dependency is the concept.
- [ ] External runtime-import terminology remains intact.
- [ ] Namespace records remain static visibility facts, not values.
- [ ] No consumer-created mixed-origin namespace exists.
- [ ] Compiler and external highlighter agree.

### Validation

Main repository:

```bash
cargo fmt --all
just validate
```

Highlighting repository:

```bash
npm test
npm pack --dry-run
```

### Exit gate

- [ ] The repository has one current dependency grammar.
- [ ] All executable docs and fixtures use it.
- [ ] Old syntax fails only through the migration diagnostic.
- [ ] Stop for review.

## Phase 3A - stable resource identity and early resolution

### Context

Start the vertical resource cutover in a dedicated worktree. This internal checkpoint may coexist with the old rendered-path lane only inside that unmerged worktree. It is not parent-accepted until Phase 3C deletes the old lane.

### Checklist

- [ ] Add `StableResourceOriginId` and portable logical resource path.
- [ ] Add module-local `ResourceId` and resource table.
- [ ] Add build-owned resolved resource source registry.
- [ ] Separate resource identity from content fingerprint.
- [ ] Classify explicit-extension expression paths during preparation.
- [ ] Reject grouped resource expressions.
- [ ] Reject missing extension, recognized source extension and directory target.
- [ ] Remove `@/` public-root semantics from resource parsing.
- [ ] Resolve resources from the owning module root.
- [ ] Enforce package/module boundary visibility.
- [ ] Canonicalize and validate containment.
- [ ] Register every authored resource dependency, including dead const branches and unmaterialised generic bodies.
- [ ] Retain parent watch interests for missing targets.
- [ ] Reuse one source location and one source record per stable origin.
- [ ] Add content hashing through the existing fingerprint substrate.
- [ ] Keep opaque bytes out of public-interface fingerprints.
- [ ] Add counters for syntax rows, sources and bytes hashed.
- [ ] Add focused resource-source agreement checks.

### Tests

- [ ] stable identity ignores source location and output path
- [ ] moving the declaration between files in one module preserves identity
- [ ] moving or renaming the resource changes identity
- [ ] missing file diagnostic
- [ ] directory target diagnostic
- [ ] source-extension diagnostic
- [ ] extensionless resource diagnostic
- [ ] cross-module private resource diagnostic
- [ ] symlink escape diagnostic
- [ ] strict case diagnostic
- [ ] dead const branch still validates
- [ ] unmaterialised generic body still validates
- [ ] equal origin source disagreement is `CompilerError`

### Audit and validation

- [ ] Resource ID tables are dense and module-local.
- [ ] Absolute paths stay build-owned.
- [ ] No output URL is computed.
- [ ] No public semantic identity contains content hash.
- [ ] Run focused tests, `cargo fmt --all` and `just validate`.
- [ ] Commit an internal worktree checkpoint.
- [ ] Do not merge or mark Phase 3 accepted.

## Phase 3B - resource anchors through templates and HIR

### Context

Replace eager string coercion with structural resource anchors from AST through link facts.

### Checklist

- [ ] Promote production `ExpressionKind::Path(ResourceId)`.
- [ ] Make direct template-head resource syntax emit `TemplateIrNodeKind::Resource`.
- [ ] Add opaque formatter resource anchors.
- [ ] Update every TIR walker and reducer exhaustively.
- [ ] Update slot, wrapper, control-flow and preparation semantics.
- [ ] Preserve resource pieces in const-template folding.
- [ ] Add resource pieces to compile-time page fragments.
- [ ] Add `OwnedRuntimeTemplateNode::Resource`.
- [ ] Update immutable and mutable runtime-handoff walkers.
- [ ] Add resource append/string operation to HIR.
- [ ] Record resource uses per source and generated function.
- [ ] Record dormant-root resource uses.
- [ ] Make resource nodes non-reactive and output-producing.
- [ ] Preserve lazy runtime branch behavior.
- [ ] Ensure resource-bearing strings cannot be flattened into ordinary literals.
- [ ] Add targeted const-required content diagnostics.
- [ ] Update HIR validation and reachability.
- [ ] Add containing-artefact mapping input to backend lowering contracts.
- [ ] Include the mapping fingerprint in physical variant identity.

### Tests

- [ ] const direct template resource anchor
- [ ] runtime direct template resource anchor
- [ ] resource inside slot and child wrapper
- [ ] resource inside runtime branch and loop
- [ ] resource inside reactive template remains non-reactive
- [ ] resource-bearing const fragment order
- [ ] resource-bearing string passed and returned as String
- [ ] compile-time content operation rejects unresolved placement
- [ ] TIR subtree copy preserves resource ID
- [ ] runtime handoff walkers visit resources exactly once
- [ ] HIR link facts retain deterministic resource-use order
- [ ] routes at different depths do not share the wrong variant

### Audit and validation

- [ ] TIR cleanup owners remain the sole walkers.
- [ ] Formatter code cannot inspect resource internals.
- [ ] No URL string exists before builder planning.
- [ ] No extra resource representation parallels TIR.
- [ ] Run focused tests, `cargo fmt --all` and `just validate`.
- [ ] Commit an internal worktree checkpoint.
- [ ] Do not merge or mark Phase 3 accepted.

## Phase 3C - builder placement and old-lane deletion

### Context

Complete the vertical cutover. Build final paths and URLs from structural facts, then delete the old rendered-path system before merging.

### Checklist

- [ ] Add one `ResourceLinkPlan` owner.
- [ ] Plan project-local resource outputs relative to `entry_root`.
- [ ] Build containing-artefact-relative URLs.
- [ ] Percent-encode portable path segments.
- [ ] Lower resource HIR operations with the entry URL map.
- [ ] Return resource bytes as ordinary `OutputFile::Bytes`.
- [ ] Integrate output validation, skip-unchanged writes, manifests and stale cleanup.
- [ ] Deduplicate one origin across many entry uses.
- [ ] Diagnose resource/resource and resource/builder conflicts.
- [ ] Emit large-resource warnings once per reachable origin.
- [ ] Add exact entry union tests for compile-time and runtime uses.
- [ ] Delete `RenderedPathUsage`.
- [ ] Delete module-wide rendered-path metadata.
- [ ] Delete eager path formatting from template heads.
- [ ] Delete `CompileTimePathBase::RelativeToFile`.
- [ ] Delete directory resource values.
- [ ] Delete public-origin prefix application from Path rendering.
- [ ] Delete HTML per-module tracked-asset reconstruction.
- [ ] Remove or repurpose `tracked_assets.rs` under the new plan owner.
- [ ] Update `index.md` for moved/deleted owners.
- [ ] Run repository searches for every old owner.

### End-to-end tests

- [ ] root page uses `./assets/logo.svg`
- [ ] nested page uses `../../assets/logo.svg`
- [ ] binary output bytes are exact
- [ ] one asset used by two pages emits once
- [ ] unused validated resource emits nothing
- [ ] different origins claiming one path fail with both locations
- [ ] resource conflicts with HTML output
- [ ] stale resource output is cleaned after use removal
- [ ] external URL string emits no resource
- [ ] site-root String remains unchanged
- [ ] dev and release builds produce equivalent resource sets

### Audit and validation

- [ ] One resource authority remains.
- [ ] No arbitrary string scanning.
- [ ] No source file is read more than once for one emitted resource.
- [ ] No route-specific URL enters semantic identity.
- [ ] Run `cargo fmt --all`.
- [ ] Run `cargo run --quiet -- tests --audit`.
- [ ] Run `just validate`.
- [ ] Run a read-only architecture audit over AST, TIR, HIR, link and output boundaries.
- [ ] Merge the complete Phase 3 vertical cutover only after the audit is clean.
- [ ] Refresh the capsule and stop for review.

## Phase 4 - builtin Path and compile-time aggregate rules

### Context

Direct template resource paths now work structurally. Expose the full Path value surface while keeping it compile-time-only.

### Checklist

- [ ] Add reserved builtin `Path` type syntax and TypeId.
- [ ] Add canonical builtin Path identity.
- [ ] Implement direct Path expression evaluation.
- [ ] Add one recursive `TypeAvailability` classifier.
- [ ] Propagate compile-time-only classification through aliases, records and collections.
- [ ] Allow `#Path`, inferred Path constants and const collections.
- [ ] Allow nominal const records with Path fields.
- [ ] Allow compile-time field access and loops over Path collections.
- [ ] Reject ordinary and mutable Path bindings with precise diagnostics.
- [ ] Reject function parameters, returns, receiver methods and runtime fields containing Path.
- [ ] Reject option, choice, map and generic Path shapes.
- [ ] Reject operators, equality, ordering and casts.
- [ ] Reject Path in `config.moth`.
- [ ] Ensure defining a Path is a dependency but not a resource use.
- [ ] Ensure rendering a Path creates the use.
- [ ] Update diagnostic rendering and type-name tests.
- [ ] Update compiler-owned code highlighting.
- [ ] Update external VS Code grammar and tests for Path type and explicit-extension expressions.

### Integration tests

- [ ] direct Path constant
- [ ] explicit Path annotation
- [ ] transparent alias
- [ ] const record fields
- [ ] collection of Paths
- [ ] compile-time loop chooses resource anchors
- [ ] ordinary binding diagnostic
- [ ] mutable binding diagnostic
- [ ] parameter and return diagnostic
- [ ] runtime record diagnostic
- [ ] option, choice, map and generic diagnostic
- [ ] config diagnostic
- [ ] Path has no equality, operator or cast
- [ ] unused Path emits no file

### Audit and validation

- [ ] One recursive availability classifier owns all runtime-boundary rejection.
- [ ] Path does not create a second type system.
- [ ] Compile-time-only state is not encoded in nominal identity.
- [ ] Run `cargo fmt --all` and `just validate`.
- [ ] Run external highlighter tests.
- [ ] Stop for review.

## Phase 5 - public Path values, resource templates and generics

### Context

Make resources cross module and package boundaries through ordinary exported values. This phase fixes the original font failure at the semantic boundary.

### Checklist

- [ ] Add `PublicResourceRef`.
- [ ] Add `PublicFoldedValue::Path`.
- [ ] Add `PublicConstTemplatePiece::Resource`.
- [ ] Extend recursive folded-value visitors.
- [ ] Extend public-interface agreement and validation.
- [ ] Project imported stable refs into consumer-local ResourceId values.
- [ ] Preserve resource-bearing const templates without flattening.
- [ ] Support exported Path constants.
- [ ] Support exported public nominal record types containing Path while keeping every instance compile-time-only.
- [ ] Support exported const records and collections containing Path.
- [ ] Support re-exported Path declarations through ordinary dependency clauses.
- [ ] Retain package/project resource source registries through `ProjectCompilation`.
- [ ] Preserve provider package ownership and source availability.
- [ ] Add resource origins to public-interface fingerprints.
- [ ] Keep content fingerprints out of public interfaces.
- [ ] Freeze generic-body resource refs.
- [ ] Project them into generated sidecars.
- [ ] Keep Path illegal as a generic type argument.
- [ ] Ensure unmaterialised/unreachable generated resources are not emitted.
- [ ] Add support-package asset surface fixtures.

### Integration tests

- [ ] exported font Path used inside exported CSS template
- [ ] support package exports Path and consumer renders it
- [ ] alias/re-export changes public name but preserves origin
- [ ] consumer cannot address provider's private resource path
- [ ] public const record of resources
- [ ] public collection of resources
- [ ] two consumers share one resource output
- [ ] dependency package resource output gets package prefix
- [ ] generic body with resource validates before materialisation
- [ ] materialised generic emits only when reachable
- [ ] interface agreement catches differing resource refs
- [ ] no absolute path appears in interface debug/test payloads

### Audit and validation

- [ ] Resource visibility is ordinary declaration visibility.
- [ ] No public resource-file visibility table exists.
- [ ] Public interfaces remain backend-neutral.
- [ ] Generated sidecars use boundary-local resource IDs.
- [ ] Run `cargo fmt --all` and `just validate`.
- [ ] Stop for review.

## Phase 6 - `$literal` and composable CSS

### Context

Make normal CSS templates capable of nested templates and Path anchors. Replace balanced-mode overwrite behavior with strict syntax requirements.

### Checklist

- [ ] Add frontend-owned `$literal`.
- [ ] Replace `TemplateBodyMode` overwrite with requirement merging.
- [ ] Make directive requirement merge order-independent.
- [ ] Diagnose incompatible requirements with both locations.
- [ ] Move `$css` and `$css("inline")` to normal Template syntax.
- [ ] Keep CSS validation around opaque anchors.
- [ ] Make `$code` request Literal.
- [ ] Make `$note` and `$todo` request Discard.
- [ ] Keep `$raw` as whitespace policy.
- [ ] Remove `$css` balanced-body dependence.
- [ ] Do not add `$css("raw")`.
- [ ] Preserve CSS attribute selectors and grid line names through `$literal`.
- [ ] Add Path-in-CSS font, image and nested-template examples.
- [ ] Update directive registry docs and diagnostics.
- [ ] Update compiler and VS Code highlighting for `$literal`.

### Tests

- [ ] resource Path inside ordinary `$css`
- [ ] nested template inside `$css`
- [ ] `$css("inline")` resource anchor
- [ ] literal attribute selector
- [ ] literal grid line names
- [ ] `$literal, $css` equals `$css, $literal`
- [ ] duplicate equal requirements accepted
- [ ] incompatible Literal/Discard reports both directives
- [ ] `$raw` does not suppress child templates
- [ ] `$code` remains literal
- [ ] `$note` and `$todo` remain discarded

### Audit and validation

- [ ] Body syntax and formatting remain separate owners.
- [ ] No directive-order behavior remains.
- [ ] CSS validator does not inspect resource URLs.
- [ ] Run `cargo fmt --all` and `just validate`.
- [ ] Run external highlighter tests.
- [ ] Stop for review.

## Phase 7 - provider-backed resources

### Context

Unify provider source identity, semantic interfaces and runtime resources without automatically publishing provider implementation files.

### Checklist

- [ ] Rename provider APIs from import-oriented to source-dependency terminology.
- [ ] Pass stable provider-source resource origin into provider requests.
- [ ] Pass content fingerprint into provider cache keys.
- [ ] Keep canonical source path as IO only.
- [ ] Replace `RuntimeAssetIdentity { canonical_source_path, ... }`.
- [ ] Add stable provider runtime resource declarations.
- [ ] Support generated provider resource bytes.
- [ ] Require stable provider output paths.
- [ ] Keep provider source un-emitted by default.
- [ ] Emit it only through an explicit provider declaration or ordinary reachable Path.
- [ ] Attach provider runtime requirements to exact exported symbol reachability.
- [ ] Update JavaScript provider and runtime emission plan.
- [ ] Deduplicate shared runtime resources by stable identity.
- [ ] Diagnose provider identity/output disagreement transactionally.
- [ ] Ensure a `.js` Path expression alone does not invoke the provider.
- [ ] Ensure a `.js` dependency clause does invoke it.
- [ ] Update builder-surface docs and tests.
- [ ] Delete old provider runtime-asset PathBuf identity.

### Tests

- [ ] grouped provider dependency
- [ ] whole provider namespace requires `as`
- [ ] filtered provider namespace
- [ ] providerless explicit-extension clause diagnostic
- [ ] provider source not copied by default
- [ ] provider source copied when separately rendered as Path
- [ ] provider declares itself as runtime resource
- [ ] provider declares generated runtime resource
- [ ] unreachable provider symbol emits no runtime resource
- [ ] provider content edit invalidates provider semantics
- [ ] stable provider resource conflicts are deterministic
- [ ] same source Path/provider resource deduplication

### Audit and validation

- [ ] Provider semantics decorate one resource identity.
- [ ] Provider source, runtime asset and generated output are distinct explicit states.
- [ ] No backend reconstructs provider identity from extension or PathBuf.
- [ ] Publication is transactional.
- [ ] Run `cargo fmt --all` and `just validate`.
- [ ] Stop for review.

## Phase 8 - exact project/package resource planning hardening

### Context

All resource-producing lanes now exist. Consolidate global planning, package naming, conflicts and physical variant keys into their final form.

### Checklist

- [ ] Build one project-level resource source registry over project and package boundaries.
- [ ] Build one exact resource union per entry/package assembly.
- [ ] Add stable package output-prefix encoding.
- [ ] Cover project-local, dependency, Core and Builder source package identities.
- [ ] Plan provider-managed resources through the same conflict authority.
- [ ] Deduplicate source reads and emitted bytes globally.
- [ ] Sort plans by output path then stable origin.
- [ ] Validate every output path before reading bytes.
- [ ] Compute every containing-artefact URL map once.
- [ ] Include resource URL map in physical variant reuse keys.
- [ ] Handle standalone CSS containing artefacts when present.
- [ ] Keep inline CSS relative to HTML.
- [ ] Integrate large-resource warning deduplication.
- [ ] Confirm stale manifest cleanup across profile and builder ownership.
- [ ] Add counters for union size, deduplication and bytes read.
- [ ] Remove any per-module resource-planning fallback left from Phase 3.
- [ ] Update benchmark cases when resource planning changes measured workloads.

### Tests

- [ ] same project resource across many pages
- [ ] same package resource across many entries
- [ ] two packages with same relative resource path
- [ ] project/package collision
- [ ] provider/project collision
- [ ] standalone CSS relative URL
- [ ] inline CSS relative URL
- [ ] route-depth physical variant distinction
- [ ] deterministic output under reversed module/package completion order
- [ ] profile-separated manifests
- [ ] stale output deletion
- [ ] large warning once

### Audit and validation

- [ ] Per-function/root facts are the reachability authority.
- [ ] Final vectors are contiguous and deterministic.
- [ ] No complete resource map is cloned per entry.
- [ ] No resource output is read before conflict validation succeeds.
- [ ] Run `cargo fmt --all`, `just validate` and `just bench-check`.
- [ ] Stop for review.

## Phase 9 - incremental resource updates and dev serving

### Context

Use the separated resource content fingerprint to avoid semantic rebuilds for opaque byte-only changes.

### Checklist

- [ ] Change watch reporting from revision-only to changed-path batches.
- [ ] Add exact resource watch targets from the latest successful build.
- [ ] Include package resources outside the project entry root when present.
- [ ] Include parent interests for missing resources.
- [ ] Add `ResourceBuildState` to successful dev state.
- [ ] Classify changed paths before invoking the compiler.
- [ ] Implement the opaque resource-only fast path.
- [ ] Rehash and rewrite changed emitted resources.
- [ ] Update fingerprints for un-emitted resource dependencies without reload.
- [ ] Broadcast reload only when an emitted resource changed.
- [ ] Fall back to full build for every unsafe or unknown case.
- [ ] Preserve previous successful outputs on fast-path read failure.
- [ ] Render structured dev diagnostics for resource IO failures.
- [ ] Add counters proving compiler stages were skipped.
- [ ] Add font/image/application MIME mappings listed above.
- [ ] Add origin-mounted binary serving tests.
- [ ] Add exact WOFF2 response bytes and content type test.
- [ ] Keep broad HTTP hardening deferred.
- [ ] Add non-recording before/after dev rebuild measurements.

### Tests

- [ ] emitted opaque resource edit avoids compiler executor
- [ ] un-emitted resource edit updates state without reload
- [ ] provider file edit uses full build
- [ ] source/config edit uses full build
- [ ] delete/rename/kind change uses full build
- [ ] unknown file uses full build
- [ ] failed prior build uses full build
- [ ] package resource outside entry root is watched
- [ ] missing resource creation retriggers build
- [ ] concurrent change during fast path queues another cycle
- [ ] WOFF2, WebP and AVIF serving
- [ ] output directory changes remain ignored

### Audit and validation

- [ ] Fast path writes through normal output ownership.
- [ ] Watchers do not become a second dependency resolver.
- [ ] Mtime is not the content fingerprint.
- [ ] No semantic cache is mutated by byte-only changes.
- [ ] Run `cargo fmt --all`, `just validate`, `just bench-check` and `just bench-frontend-check`.
- [ ] Stop for review.

## Phase 10 - documentation, examples and tooling completion

### Context

The compiler surface is complete. Finish teaching, contributor references, roadmap status and external tooling without hiding deferred work.

### Checklist

- [ ] Update all relevant canonical unsuffixed language references:
  - [ ] package/import path references renamed to dependency clauses
  - [ ] grouped and namespace dependency forms
  - [ ] visibility and public re-exports
  - [ ] Path values and compile-time-only rules
  - [ ] constants, const records and const templates
  - [ ] template basics and directives
  - [ ] `.mtf` resource paths
  - [ ] plain Markdown literal link contract
  - [ ] external provider contracts
- [ ] Add focused canonical Path/resource reference files and route them from the language index.
- [ ] Update paired Basic teaching pages.
- [ ] Update package, project structure, HTML builder and dev-server docs.
- [ ] Update compiler and build-system implementation maps.
- [ ] Update scaffold templates and example projects.
- [ ] Update README snippets where dependency syntax appears.
- [ ] Update `CONTRIBUTING.md` or agent guidance only if workflow changed.
- [ ] Update the progress matrix to current support and coverage.
- [ ] Keep deferred limitations explicit in row notes.
- [ ] Update the roadmap:
  - [ ] mark this plan active/complete at the correct time
  - [ ] remove stale active items
  - [ ] preserve later link/backend/reuse ordering
  - [ ] add the deferred follow-up section below
- [ ] Complete external VS Code grammar coverage for dependency clauses, Path and `$literal`.
- [ ] Search generated docs for old current-syntax `import @`.
- [ ] Keep intentional removed-syntax examples clearly labeled.
- [ ] Rebuild release docs.
- [ ] Inspect changed routes and generated output.

### Audit and validation

Main repository:

```bash
cargo fmt --all
just validate
moth build docs --release
```

Highlighting repository:

```bash
npm test
npm pack --dry-run
```

- [ ] Verify source, docs and matrix agree.
- [ ] Verify no generated HTML was edited manually.
- [ ] Stop for review.

## Phase 11 - final deletion, performance and closeout audit

### Context

Prove that the migration left one current path from source syntax to output and no hidden legacy authority.

### Deletion audit

- [ ] no `TokenKind::Import`
- [ ] no current source parser for `import @...`
- [ ] no token rescan for dependency clauses
- [ ] no expanded grouped path payload per leaf
- [ ] no old `ImportShellId` or `FileImport`
- [ ] no `@/` public-root path
- [ ] no `@./` or parent resource fallback
- [ ] no test-only production `ExpressionKind::Path`
- [ ] no `CompileTimePathBase::RelativeToFile`
- [ ] no directory resource value
- [ ] no eager Path-to-String formatting
- [ ] no `RenderedPathUsage`
- [ ] no module-wide asset reachability authority
- [ ] no HTML/CSS/Markdown string scanner
- [ ] no public absolute resource path
- [ ] no PathBuf runtime-asset identity
- [ ] no provider source auto-copy
- [ ] no last-writer template body mode
- [ ] no balanced `$css`
- [ ] no per-module tracked-asset planner
- [ ] no route-specific URL outside the resource link plan
- [ ] no full compiler invocation for safe opaque byte-only dev edits
- [ ] no stale import grammar in built-in or external syntax highlighting
- [ ] no compatibility wrapper or feature flag

### Correctness audit

- [ ] every authored Path validates once
- [ ] every emitted resource is reachable
- [ ] every stable origin resolves to one agreeing source
- [ ] every output path is validated before IO
- [ ] every public resource ref is portable and backend-neutral
- [ ] every TIR, handoff and HIR walker handles resources exhaustively
- [ ] every collision reports both owners
- [ ] every runtime Path misuse uses a structured diagnostic
- [ ] every package remains self-contained
- [ ] every physical variant uses the correct containing-artefact URL map
- [ ] every dev fast path has a conservative full-build fallback

### Performance audit

Use counters to prove:

- [ ] one path syntax row per authored path
- [ ] one preparation resolution per resource literal
- [ ] one content hash per changed source per build state
- [ ] one emitted byte read per unique planned resource
- [ ] no resource-table clone per entry
- [ ] no repeated provider source parse for one fingerprint
- [ ] no compiler stage in safe resource-only dev updates
- [ ] no benchmark regression outside agreed noise

Run:

```bash
cargo fmt --all
cargo run --quiet -- tests --audit
just validate
just bench-check
just bench-frontend-check
```

Perform:

- [ ] final read-only architecture audit
- [ ] final style-guide audit
- [ ] final integration-suite ownership review
- [ ] final roadmap and progress-matrix review
- [ ] release docs build and route inspection

### Closeout

- [ ] Resolve every required finding.
- [ ] Record exact validation evidence in the capsule.
- [ ] Mark the plan complete in the roadmap.
- [ ] Replace the active capsule with a concise accepted-baseline capsule.
- [ ] Preserve deferred items under their owning roadmap section.
- [ ] Commit the final closeout.
- [ ] Stop.

## Required end-to-end contracts

The final suite must cover:

### Dependency grammar

- extensionless namespace dependency
- direct selection
- namespace alias
- filtered namespace
- nested provider namespace
- explicit grouped re-export
- provider-backed clause
- removed import diagnostic
- config and `.mtf` restrictions
- collision and casing behavior

### Path values

- direct resource literal
- compile-time annotation
- aliases
- records and collections
- compile-time control flow
- runtime rejection
- visibility and cross-module export
- extension/source-kind errors
- missing, directory, case and containment errors

### Templates and resources

- const/runtime insertion
- slots, wrappers, branches and loops
- public resource-bearing templates
- generated functions
- reactive templates
- CSS and `$literal`
- containing-artefact relative URLs

### Build output

- project and package placement
- provider-managed resources
- exact byte output
- deduplication
- conflicts
- stale cleanup
- deterministic order
- large-resource warnings
- no output for unused dependencies

### Incremental dev

- opaque byte-only fast path
- provider full rebuild
- structural full rebuild
- missing resource recovery
- package watch targets
- MIME and binary streaming

## Deliberately deferred work

Record these in `docs/roadmap/roadmap.md` under a dedicated resource/path follow-up section. Reflect them in existing progress-matrix row notes rather than creating noisy standalone rows unless implementation status needs one.

### Language and source-kind follow-ups

- resource-aware plain Markdown links and images
- extensionless opaque resource escape syntax
- whole-directory publication
- asset-only package/facade sugar
- Path in options, choices, maps or generic applications
- runtime Path values and filesystem APIs
- Path values in `config.moth`
- external URL/resource-reference type
- URL validation, fetching or rewriting
- configured physical resource roots outside `entry_root`

### Builder and pipeline follow-ups

- output filename hashing
- content-addressed resource names
- image/font/CSS transforms
- CDN/public-origin rewriting
- user-configurable resource processors
- managed imported CSS with parsed internal URL dependencies
- automatic CSS extraction into standalone artefacts
- resource compression and preloading policy
- cross-entry chunk/resource packaging beyond exact deduplication

### Provider follow-ups

- user-authored provider plugins
- remote provider sources
- provider transformation pipelines
- persistent provider/resource caches
- package-manager resource embedding and precompiled resource archives

### Dev-server follow-ups

- cache-control policy
- conditional requests and ETags
- range requests
- compression
- richer response metadata
- general HTTP hardening beyond required MIME support

Deferred work must not weaken the implemented core:

- no heuristic string scanning
- no bypass of module visibility
- no absolute-path public identity
- no implicit runtime Path conversion

## Final accepted end state

After this plan:

```text
source text
-> one token and PathSyntax table
-> retained dependency clauses and resource literals
-> Stage 0 dependency graph plus early resource source registry
-> bound source/provider interfaces
-> AST Path values and structural resource anchors
-> TIR / public folded values / runtime handoff
-> HIR per-function resource facts
-> exact entry/package reachability
-> ResourceLinkPlan
-> containing-artefact relative URLs
-> ordinary validated OutputFile bytes
-> manifest-owned output and incremental dev state
```

The programmer-facing rule remains small:

> Extensionless top-level paths bind semantic dependencies. Explicit-extension expression paths are compile-time resource values. Only reachable resource uses are emitted.
