# HTML page directives and runtime title

## Status

- Status: queued, with the design interview accepted.
- Current slice: not started.
- Blockers: shared directive recognition, retained syntax, compile-time call arguments and directive-based project configuration must be delivered.
- Next action: refresh the activation tree and start Phase 0.

Intended location: `docs/roadmap/plans/html-page-directives-and-runtime-title-plan.md`.

## Purpose

Make `$page` the explicit HTML entry declaration on a normal module root. Replace reserved `page_*` metadata extraction with folded directive arguments and add capability-checked runtime title mutation.

Preserve one directory-scoped semantic module, at most one page per directory, the module-owned implicit `start`, directory-derived routes and non-executing dependency access. Ordinary files do not become pages or gain independent entry bodies.

The accepted interview replaces the old entry-local `config:` proposal and the rejected ordinary-file page proposal. This plan carries forward the old proposal's useful metadata, resource, document-shell and runtime-title requirements, not its syntax, inactive-section machinery or migration diagnostics.

## Required authority and activation reading

Read `AGENTS.md`, the full codebase style/testing guides and both compiler/build architecture authorities. Read the current language route and these canonical references:

- `docs/src/docs/directives/directives.mtf`.
- `docs/src/docs/project-structure/module-roots.mtf`, `entry-runtime-and-fragments.mtf`, `html-routing-and-artifacts.mtf`, `entry-config.mtf`, `project-config.mtf` and `build-inputs.mtf`.
- Template basics, directives and constant-template references.
- `docs/src/docs/resources/file-paths.mtf` and `file-values.mtf`.
- `docs/src/docs/packages/core/io/io.mtf` and the binding/capability contracts.
- `docs/compiler-data-layout-design.md`, both progress matrices and the validation guide.

Read the owning lifetime/builder-lifecycle references if a handoff changes. This plan consumes existing mandatory validation paths and does not implement the separate deferred memory programme.

Establish a new baseline after rebasing. Preserve later approved native-result-slot, Wiring, source/token, diagnostic and backend changes. Current code-path names below are locators, not a reason to restore old APIs.

## 1. Exact page declaration

```moth
$page
```

```moth
$page(
    title = "Moth",
    description = "A language for creating reliable software",
    lang = "en",
    favicon = @assets/favicon.svg,
)
```

`$page` is HTML-builder-owned normal-root metadata and an entry-purpose declaration. It is not a compiler type modifier or an attribute on the next declaration.

### Placement and cardinality

| Location | `$page` |
|---|---|
| Top level of a normal module root in an application project | Valid, at most once. |
| The explicitly selected source in synthetic single-file HTML mode | Valid as that synthetic module's root, at most once. |
| Ordinary non-root `.moth` file in a directory project | Invalid. |
| Support root or external project-package facade | Invalid. |
| `config.moth` | Invalid. |
| Function body, ordinary branch, loop, declaration initializer or template head | Invalid. |
| Inside an `export:` block | Invalid. |
| A second `$page` in the same root | Duplicate directive diagnostic with both locations. |

A page directive never promotes a helper file, package source, support root or facade into a normal module. Private dependency-package roots never become application entries through dependency use.

`$page` describes the whole root regardless of source position. It does not enable a mode for subsequent statements or partition the file. Style places it early, after any earlier constants or dependencies needed by its arguments. Argument visibility remains ordinary file visibility. Same-file forward references are invalid even though the purpose applies to the whole root.

Bare `$page` is the default invocation. `$page()` is invalid under the general no-empty-parentheses rule. Positional and named arguments use the shared call owner, without a page-specific list parser.

### Signature

The following is the positional order. Every parameter is optional `String?` with default `none`, where `none` means no page override. Present values are ordinary compile-time Strings, including structural resource-bearing Strings. This uses normal optional/default handling rather than inventing an omitted-argument value category.

| Parameter | Purpose |
|---|---|
| `title` | Initial title text. |
| `description` | Description meta element content. |
| `lang` | HTML language attribute override. |
| `favicon` | Icon URL, including a tracked resource string. |
| `body_style` | Body style attribute override. |
| `head` | One composed folded string inserted into the document head. |

Unknown parameters are errors. Values must finish through ordinary compile-time checking and folding. Runtime bindings, runtime function calls and escaping control flow cannot supply metadata. A source `$config` constant may supply a value after its normal resolution.

`head` is one string/template value. Compose multiple pieces in an earlier constant or an ordinary compile-time template. There are no typed head-node declarations, metadata blocks, `+=`, multiple-call merging or last-write-wins rules.

## 2. Explicit root purpose

Top-level executable work or direct output requires a recognised purpose whose contract has an applicable consumer. `$page` is the only implemented HTML execution/output purpose after this plan. Future `$test` is documented but cannot satisfy validation until implemented.

| Authored normal-root contents | Result after this plan |
|---|---|
| `$page` alone | Emit a complete page with default document metadata. |
| `$page` and root runtime work | Activate the root's compiled `start` once for that page. |
| `$page` and static/runtime fragments | Emit/hydrate fragments in existing source order. |
| Functions, types, constants and other API declarations without purpose | Valid API-only root, no page. |
| Runtime binding, call or executable control flow without purpose | Root-purpose diagnostic. |
| Direct const `#[...]` or runtime `[...]` output without purpose | Root-purpose diagnostic. |
| Assigned or returned template values | Ordinary values, not direct output. Their containing runtime code follows normal placement rules. |
| Semantically classified `$doc`, `$note` or `$todo` templates without purpose | Documentation/comment processing only, no page and no purpose requirement. |
| `$test` before its implementation | Recognised deferred-feature diagnostic, never a no-op purpose. |

Use the authored root-activity classification, not optimised HIR liveness, to decide whether a purpose is required. An `if false` containing root runtime work, an immutable runtime binding that happens to hold a constant and an empty direct output fragment still have their authored roles. Constants marked `#` remain compile-time declarations.

The builder consumes explicit compiler-owned facts. It does not classify templates by rendered length or look for `$doc` text in emitted strings. Comment/documentation exemptions follow semantic category, not accidental empty output.

Keep root discovery honest: an unmarked root with authored executable/output work must not disappear merely because entry filtering now selects only `$page`. Retain existing prepared normal-root candidates for purpose validation. Where classification needs semantics, compile the candidate through the canonical service before rejecting or accepting it. Do not introduce a second parser, an all-helper-body scan or an ordinary-file entry inventory.

Apply the same purpose policy to roots compiled as semantic providers. A consumer cannot make invalid unpurposed root activity legal by leaving it dormant. This does not mean a valid purpose must execute in every context: dependency access and `check` remain non-activating.

### Future alternative purpose

Document `$test` as a later way to give top-level code a non-page purpose, potentially consumed by HTML-aware testing tools. It will require a separate command, lifecycle, selection, reporting and failure design. Do not implement those contracts here, invent a test-runner command or treat every metadata directive as permission to execute code.

Purpose is distinct from ordinary metadata. A future deprecation or documentation directive will not authorise root runtime work merely because it is present. Conflicting purpose declarations need an explicit future compatibility contract, not registration order.

## 3. Preserve module and route structure

Only normal module-root files can declare application pages. Preserve the existing one-root-per-directory rule. The suffix of `@page.moth`, `@site.moth` or another normal-root filename remains cosmetic.

With `entry_root = "src"`, each example below produces its output only when its root declares `$page`:

| Source root | HTML output |
|---|---|
| `src/@site.moth` | `index.html` |
| `src/about/@page.moth` | `about/index.html` |
| `src/docs/basics/@module.moth` | `docs/basics/index.html` |

An ordinary `src/about.moth` cannot declare a page. An ordinary `index.moth` gains no new meaning. Add no route override parameter, filename-derived routes, page aliases or automatic redirection.

The normal module at `entry_root` may remain declaration-only while child modules declare pages. An HTML application needs at least one page anywhere in its own selected application source tree, not necessarily a homepage at `entry_root`.

Dependency resolution and file-value resolution remain module-root-relative. Resource containment, support/package boundaries, source namespace collisions, site-origin policy and URL rendering are unchanged. `.moth` still has no file-value meaning. `$page` does not create a source expression that resolves a page URL.

One module still owns one semantic compilation, one type environment and its existing root `start`. There are no per-file starts, per-page copies of module semantics or new nominal identities. Entry assembly activates only the selected module. Depending on a module exposes declarations without running its root or inserting its fragments.

### Build, check and development policy

| Operation | Zero `$page` roots |
|---|---|
| Directory-project `check` | Valid when source/configuration is otherwise valid. Check still diagnoses unpurposed root work. |
| Single-file `check` | A declaration-only synthetic root is valid. Runtime/output work without purpose is invalid. |
| HTML application `build` or `dev` | Structured no-page-entry diagnostic. A package facade is not an implicit HTML page. |
| Single-file HTML output | Requires `$page` even though the selected file is a synthetic module root. |
| Separate package-only operations | Keep their explicitly defined package policy. Do not invent new commands here. |

Every declared application page is an entry candidate, including a metadata-only page and a descendant page not imported by the site root. Reuse the existing normal-module root inventory rather than discovering arbitrary `.moth` files as entries.

Do not fabricate an `index.html` homepage to satisfy development tooling. Serve existing descendant routes normally. If the current tool needs a launch-page field, choose the real homepage when present, otherwise the lexicographically first normalised page route. This is a deterministic tool launch choice only, not a redirect or route alias. Direct requests to an absent site root retain ordinary missing-route behaviour.

## 4. Metadata and resources

### Ordinary root visibility

Metadata arguments may use earlier same-file constants, dependency-bound constants, explicit `@project` members, resolved source `$config` values and the existing foldable expression/template surface. Dependencies and helper declarations stay outside the directive call.

Header preparation retains the invocation and its ordinary dependency/reference facts. Module AST checks and folds it once. Metadata is stored in that module's non-HIR lane. It creates no ordinary declaration, public export, runtime value or project global.

Validate a selected module's own metadata even when that module is used as a provider. Consumers never inherit or apply provider page metadata. Entry activation does not compile metadata later.

### Document behaviour to preserve

Use one resolved document-metadata path for HTML-JS and existing HTML-Wasm output. Preserve the existing document shell's behaviour:

- Present page title is the base title. Otherwise use the current route-title fallback, then project name where appropriate.
- Apply the builder's title prefix and postfix to the initial base title.
- Absent description emits no description element.
- Absent language, favicon and body style use their documented HTML document defaults.
- Absent head content contributes no extra markup.
- Escape title text and attribute values in the same rendering contexts as today.
- Insert the explicitly authored folded `head` content as HTML, without escaping it as a title or flattening its meaningful whitespace.

An explicitly supplied empty String remains a present value where current page-metadata semantics allow it. `none` means no override, not an empty String or a request to merge values. Retain existing domain checks rather than expanding this migration into HTML sanitisation or metadata validation redesign.

Project and page signatures remain separate. These explicit document fallback rules are not a generic project/entry schema inheritance system.

### Structural strings

Preserve resource and site-root anchors through all directive argument folding, copies, metadata storage, identity remaps and output planning. A tracked favicon such as `@assets/favicon.svg` enters the normal resource pipeline. The config-file prohibition on file-value paths does not apply to normal-root metadata.

Retain authored metadata use locations so missing-resource and output-conflict diagnostics identify the correct argument. Keep semantic resource identity, physical byte source, output placement and URL context separate.

The existing graph-active file-reference rule continues to apply before reachability. Later output unions include only resources actually used by selected page metadata, fragments and reachable code. Do not scan rendered HTML, CSS, HIR constant names or arbitrary strings for resources.

A page containing only resource-bearing metadata still emits its needed resources and complete document. It does not need non-empty executable HIR, an artificial fragment or a generated runtime just to become an entry.

## 5. Runtime title

Expose `io.set_title` through the existing Core IO/binding capability path. It accepts one ordinary shared String-content input using the current canonical String conversion contract, returns no source-visible value and has no recoverable result channel. Use current native return-slot conventions rather than introducing a `Void` or Result wrapper.

```moth
$page(title = "Initial title")

io.set_title("Loaded")
```

The call changes the live document title to its supplied text. It does not reapply the initial document's title prefix/postfix, alter the route, mutate `$page` metadata or rewrite the static output file.

Give the call a stable external function identity and an explicit browser document-title capability requirement in per-function link facts. HTML-JS advertises the capability. Emit the helper only for reachable calls on a supported host. Convert Moth Strings through the canonical content path, not JavaScript's incidental object stringification.

A JavaScript backend is not automatically a browser. Reachable calls on standalone/embedded JavaScript hosts without the capability are rejected before lowering. Existing whole-module HTML-Wasm remains unsupported for this call and rejects it through the same target-contract family. The later mixed backend can retain the call in a JavaScript-owned function under its ordinary affinity rules, but this plan does not implement that partitioner or a Wasm title bridge.

Unreachable calls and calls removed by normal static specialisation create no target requirement. Their frontend source still receives normal validation. A host that advertises the capability but lacks the promised document facility has violated its host contract. There is no silent runtime no-op or fallback on unsupported hosts.

`io.get_title`, reactive title binding, runtime metadata mutation beyond title and automatic title updates are outside this work.

## 6. Migration and removal

Replace these reserved metadata uses:

| Old recognised name | `$page` argument |
|---|---|
| `page_title` | `title` |
| `page_description` | `description` |
| `page_lang` | `lang` |
| `page_favicon` | `favicon` |
| `page_body_style` | `body_style` |
| `page_head` | `head` |

Migrate operational docs pages, actual starter source, packages/examples that emit HTML and integration/benchmark inputs that require HTML artefacts. Preserve necessary helper constants and ordinary imports. A composed head can be an earlier `head_content #= ...` used by `$page(head = head_content)`.

Delete reserved HIR-constant scanning, entry-scope prefix matching, key-name tables, wrong-value/duplicate metadata diagnostics superseded by shared argument validation and legacy fallbacks. All former `page_*` names become ordinary identifiers. Add no migration scanner, alias or special warning for them.

Delete the implicit rule that runtime work or fragments alone select a page. Delete homepage-at-entry-root enforcement, replacing it with command-specific no-page policy. Preserve directory routing, output conflicts, manifests and ownership.

Update `page_template` so generated `src/@page.moth` contains `$page` and uses its metadata arguments. Retain the new config generator's `$config` version example and required `$project` version argument. Test the combined real scaffold after this cutover.

Do not mass-insert `$page` into every test source. Frontend-only declaration cases and negative placement/purpose cases have different intents. Public HTML positive cases requiring an artefact should author the directive explicitly. Internal helpers must not bypass the public policy to make missing-purpose cases pass.

## 7. Owner map and handoff

Start with these current owners, then follow any rebase moves:

- `src/compiler_frontend/headers/` and `declaration_syntax/` for root role, retained directives, fragments and authored activity.
- Shared directive signatures/call arguments and module AST folding.
- `src/compiler_frontend/module_compilation/` for non-HIR metadata and canonical results.
- `src/build_system/create_project_modules/` and `build.rs` for prepared candidates and entry planning.
- `src/projects/html_project/page_metadata.rs`, `document_config.rs` and `document_shell.rs`.
- `src/projects/html_project/html_project_builder.rs`, `path_policy.rs`, `js_path.rs` and `output_plan.rs`.
- `src/projects/html_project/resource_output_plan.rs` and `structural_url_renderer.rs`.
- `src/projects/routing.rs`, `check.rs` and `dev_server/`.
- `src/projects/html_project/new_html_project/start_page_scaffolding.rs`.
- Core IO registration and the existing JS package-binding/target-capability owners.

Later stages consume resolved purpose, metadata, start identity and resource facts. They do not query arbitrary directive strings. The frontend never depends on an HTML configuration container to decide ordinary language semantics. Metadata remaps and source-span ownership follow the current compiler data-layout authority.

## 8. Implementation phases

### Phase 0: Baseline and inventories

- [ ] Refresh authorities, active branch, source state and real validation baseline.
- [ ] Confirm delivered directive/config infrastructure and the existing module-owned start boundary.
- [ ] Inventory page-name scanning, activity-based entry selection, homepage checks, scaffold source and title capability owners.
- [ ] Classify every relevant fixture and operational source before migration.

### Phase 1: `$page` syntax and metadata

- [ ] Register the exact root-only singleton signature and consume the shared call path.
- [ ] Retain source context and fold arguments with ordinary module visibility.
- [ ] Publish owned non-HIR metadata and structural resource uses with remaps.
- [ ] Cover wrong placement, cardinality, constant arguments and metadata-only roots.

### Phase 2: Entry selection and purpose validation

- [ ] Make `$page` presence the HTML entry declaration, independent of content or runtime liveness.
- [ ] Diagnose unpurposed authored root runtime/output without silently filtering its candidate away.
- [ ] Preserve helper-file, support-root and facade restrictions.
- [ ] Preserve one canonical module/start and non-executing dependencies.
- [ ] Relax the homepage requirement, enforce application no-page policy and allow zero-page checks.
- [ ] Keep synthetic single-file output explicit and development launch selection deterministic.

### Phase 3: Metadata cutover and source migration

- [ ] Replace HIR-name extraction with resolved module metadata in every HTML output path.
- [ ] Preserve shared shell defaults, escaping, raw head content and resource plans.
- [ ] Migrate operational pages, starter source and intended artefact fixtures.
- [ ] Delete old name tables, implicit activity triggers and redundant diagnostics together.
- [ ] Keep existing JS/Wasm document-shell parity without expanding unsupported runtime capabilities.

### Phase 4: Runtime title

- [ ] Add the typed IO operation, stable identity and explicit capability requirement.
- [ ] Emit only reachable HTML-JS helpers with canonical String conversion.
- [ ] Reject unsupported reachable targets before lowering.
- [ ] Verify the supplied runtime title overrides the initial document title without changing metadata or routes.

### Phase 5: Documentation, validation and Slice review

- [ ] Update project/template/IO references and both relevant progress matrices to actual support.
- [ ] Rebuild generated docs through the compiler and update index entries if owners moved.
- [ ] Run focused tests and the complete code-bearing gate.
- [ ] Complete the required Slice review and retire this plan in its completion commit.

Each phase leaves one coherent path. Do not retain a period where both reserved names and `$page` independently configure the same page, or where unknown purposes silently authorise execution.

## 9. Required coverage

| Contract | Primary evidence |
|---|---|
| Placement/cardinality | Positive normal root and synthetic root, ordinary file/support/facade/config/body/export/template rejection, duplicate locations and empty `()` rejection. |
| Page existence | Bare `$page`, all-default explicit metadata, runtime-only page, static-only page and metadata-only page. |
| Purpose errors | Root call, immutable runtime binding, mutable binding, control flow, const/runtime direct output and empty fragment without purpose. Preserve error after static false selection. |
| Exempt content | Declaration-only root and semantically classified doc/note/todo templates do not require a page. A normal assigned template is not a fragment. |
| Module isolation | Dependencies do not activate provider start, apply metadata or concatenate fragments. Module compiled once with ordinary visibility. |
| Routes | Cosmetic root renaming preserves output, descendant pages work with no homepage and ordinary `about.moth`/`index.moth` do not become entries. |
| Commands | Zero-page `check` succeeds for valid declarations, application build/dev fail, single-file HTML requires `$page` and package-only policy is not rewritten. |
| Arguments | Positional/named/default routing, runtime-value rejection, earlier/dependency/config constants, forward-reference rejection and unknown names. |
| Document parity | Title fallback/prefix/postfix, empty versus absent values, description, language, favicon, body style, raw head and preserved whitespace across supported HTML outputs. |
| Resources | Metadata-only favicon emission, correct use spans, URL contexts, missing input, unused resource non-emission and conflicts with HTML/JS/Wasm outputs. |
| Name release | Ordinary `page_*` constants of otherwise legal types have no metadata effect and no migration warnings. |
| Title capability | Reachable HTML-JS helper, exact runtime assignment, unsupported standalone JS and HTML-Wasm rejection, no helper for inactive/unreachable calls. |
| Starter | Real `moth new html` project checks/builds, contains explicit version config and page declarations and retains version override behaviour. |
| Output ownership | Profile/builder manifests, stale-page removal and resource collisions remain validated before writing. |

Use the existing manifest and backend-specific expectations. Keep one primary owner per public behaviour. Subsystem tests cover real registry/remap/call invariants, not copies of end-to-end examples.

## 10. Completion criteria

A successful implementation makes page creation explicit without adding a second module/entry model. It removes the old metadata scanner and activity inference, preserves route/resource behaviour and distinguishes compilation, purpose validation and entry activation.

Run the current `just validate` gate and the docs release build after source-doc migration. Review source errors versus infrastructure errors, remap ownership, eager retention, shared call parsing, redundant metadata forms, wrapper APIs and fixture duplication. Mark affected audit evidence stale only under the existing audit-log rules, and do not claim a new structured audit from a Slice review.

`$feature`, `$layout`, `$export`, `$async`, `$checked`, `$test`, custom metadata and directive-block semantics remain deferred. No arbitrary-file page model is left as hidden scaffolding.
