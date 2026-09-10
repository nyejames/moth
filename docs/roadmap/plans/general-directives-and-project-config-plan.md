# General directives and project configuration

## Status

- Status: queued, with the design interview accepted.
- Current slice: not started.
- Blockers: the permanent directive references must be published and obsolete reactive `$` syntax must be removed before this plan activates.
- Next action: establish the rebased activation baseline and start Phase 0.

Intended location: `docs/roadmap/plans/general-directives-and-project-config-plan.md`.

## Purpose and authority

Extend Moth's existing template directive family into a constrained general directive system. Deliver `$config`, `$project` and `$html_builder` as its first non-template consumers. Replace the `#Config` qualifier and recognised configuration records rather than supporting both systems.

The accepted decisions below are the implementation contract from the maintainer's design interview. The preceding documentation work publishes them into permanent references. At activation, read those references and reconcile any subsequent maintainer-approved changes. An older implementation or stale queued plan does not override this interview. Resolve a genuine later design conflict explicitly rather than silently choosing another behaviour.

Implementation names and internal Rust layouts may follow the activation tree. Source syntax, ownership boundaries, phase ordering, defaults and exclusions in this plan are requirements, not optional suggestions.

## Required reading

Read `AGENTS.md`, the complete codebase style guide and the language cheatsheet. Read both `docs/compiler-design-overview.md` and `docs/build-system-design.md` in full for this cross-stage change. Also read:

- `docs/compiler-data-layout-design.md` for retained syntax, source identities, spans and diagnostic storage.
- `docs/src/developer-docs/language/overview.mtf` and its relevant canonical unsuffixed references.
- `docs/src/docs/directives/directives.mtf`, published before activation.
- `docs/src/docs/project-structure/project-config.mtf`, `build-inputs.mtf` and `entry-config.mtf`.
- `docs/src/docs/functions/calls-and-access.mtf`, `parameters-and-defaults.mtf` and the constants references.
- `docs/src/docs/templates/template-directives.mtf`, `template-slots.mtf` and `child-wrappers.mtf`.
- `docs/src/docs/resources/file-paths.mtf` and `file-values.mtf`.
- The full testing guide and the applicable validation-guide sections under `docs/src/developer-docs/style-guide/`.
- Both progress matrices, `docs/roadmap/roadmap.md` and `index.md` as a locator.

Use the current authority paths when files have moved. Record the replacement path in working notes. Never restore an obsolete subsystem solely because an anchor below names its former location.

## Delivery boundary

This plan delivers:

1. One directive recognition and ownership model, with shared call-shaped argument parsing and validation.
2. Compiler-owned `$config` declaration metadata in config and source files.
3. Config-only `$project` and `$html_builder` invocations with strict signatures.
4. Typed folded configuration handoffs, without recognised-name record scanning or implicit project-field input providers.
5. Updated generated config, migrated first-party config inputs and truthful documentation/status.
6. Targeted recognition of the agreed deferred directive names, without their semantics.

The later HTML entry work delivers `$page`, explicit root-purpose enforcement, page-metadata migration and runtime title mutation. Until that cutover, the current HTML entry and `page_*` mechanism remains the one working page path. This plan may recognise `$page` as unavailable, but cannot make it a working or no-op directive.

## 1. Directive vocabulary and ownership

`$` introduces a registered compiler or builder directive. It is not a binding mode, wiring operator, ordinary expression or source-defined callable. `with` is not introduced.

The registry contract separates owner, permitted context, argument signature, processing phase and effects. These are facts about a directive, not a plugin framework with arbitrary callbacks into compiler semantics.

| Category | Permitted owner and effect |
|---|---|
| Template directive | Compiler or builder. Existing constrained template formatting, composition and lexical contracts apply. |
| Declaration modifier | Compiler-owned when it changes language semantics. `$config` is the first implemented source modifier. |
| Standalone project configuration | Compiler-owned project contract or builder-owned settings, valid only in `config.moth`. |
| Standalone normal-root metadata or purpose | Builder-owned metadata consumed by entry planning. `$page` is delivered later. |
| Source block or structural selection | Compiler-owned only. All such implementations are deferred. |

Builders may consume folded metadata and supply lifecycle/entry inputs. They cannot add source declarations, rewrite functions, change nominal identity, mutate HIR, alter visibility or bypass borrow/lifetime rules. Template extension powers do not authorise arbitrary source parsers or function transformations.

Config-only is a context/phase restriction, not a second registry and not synonymous with compiler ownership. `$html_builder` is HTML-builder-owned and config-only. `$project` is compiler-owned at the language boundary and config-only. The existing build owner still applies project settings. `$config` is compiler-owned and valid in both bootstrap and selected-source declaration contexts.

Users and source packages cannot define, alias, import or override semantic directives. Frontend names are authoritative. Duplicate registrations and name conflicts are rejected rather than resolved by last registration. Reuse one registration authority for recognition, permitted placement, diagnostics and tooling. Context is checked after name recognition so a known directive in the wrong place receives a placement diagnostic.

Directive names do not reserve the corresponding ordinary identifiers. In particular, `$project`, `$config` and `$html_builder` do not reserve `project`, `config`, `Config` or `html_builder` outside directive syntax. File semantics for plain `config.moth` and the dependency root `@project` remain separately reserved.

## 2. Syntax and shared argument rules

The implemented non-template forms are:

```moth
$config
version #String = "0.1.0"

$project(name = "my_site", version = version, entry_root = "src")

$html_builder
```

A directive with supplied arguments uses `$name(arguments)`. A directive with no supplied arguments uses `$name`. Empty parentheses are invalid even when all parameters have defaults. Bare invocation still runs ordinary required-parameter and default validation.

Use the normal call parser and its parameter-slot routing for positional arguments, named arguments, ordering, duplicate/unknown arguments, expression boundaries, contextual types, default filling, commas and multiline whitespace. Ordinary calls retain their existing empty-parenthesis syntax. The directive entry boundary alone rejects an authored empty `()`.

For example, these use the same signature:

```moth
$project("my_site", "0.1.0")
$project(name = "my_site", version = "0.1.0")
```

Directive arguments and defaults are compile-time values. Directive-specific validation rejects runtime values, exclusive/mutable arguments and escaping control flow. Normal constant-expression checking remains authoritative. Sharing syntax does not add arbitrary compile-time execution, runtime directive calls or a new expression grammar.

The descriptor fixes the directive's form. `$project` and `$html_builder` are standalone items. They never modify the next declaration. `$config` modifies exactly one following declaration, ignoring ordinary lexer trivia such as whitespace and `--` comments. It does not cross an intervening source item or block boundary. A dangling modifier is an error. Duplicate `$config` modifiers on one declaration are errors, not idempotent annotations.

Source modifiers may occupy their own line or precede their target on the same line where ordinary token boundaries permit it. Documentation uses the separate-line form. Standalone directives follow ordinary item boundaries. There is no semicolon statement terminator, comma-separated `with` clause or new block delimiter.

### Preserve template semantics while sharing machinery

Move existing template directive argument parsing onto the same argument-list owner. Preserve each existing directive's accepted value domain, contextual meaning, head compatibility, formatter behaviour and literal-body mode. Register parameter expectations in the one signature authority rather than adding a second named-argument implementation.

Slot labels, indices and helper templates remain their existing compiler-known argument categories. Sharing parsing must not turn a slot label into a source lookup, force a helper-only wrapper to render before slot composition or allow runtime argument evaluation. Use the owning template semantic contracts for these specialised arguments and the shared parser for list structure and slot routing.

Current call syntax owns semantic parsing. Header preparation may retain an invocation's balanced token range with the existing delimiter owner, but must not parse a parallel argument language. The invocation's arguments are semantically parsed once through the shared owner when its receiving context is available. Reuse retained facts thereafter.

## 3. `$config` declarations

### Exact source forms

```moth
$config
analytics #Bool = false

$config
max_retries #Int = 3

$config
api_url #String

$config
label #String? = none
```

`$config` takes no arguments. The binding name is the build-input name. `#` remains the compile-time binding marker. The explicit type is the contract type. The initializer after `=` is the fallback value.

| Declaration | Missing external input |
|---|---|
| Non-optional type, no initializer | Required-input diagnostic. |
| Non-optional type with initializer | Use the validated fallback. |
| Optional type with no initializer or with `= none` | Resolve to absence. These have the same normalised absence default. |
| Optional type with a present initializer | Use that present fallback. |

Only a single explicitly typed top-level compile-time binding is eligible. Reject inferred `#=` configuration contracts, runtime/mutable bindings, multi-bind targets, parameters, struct/record/choice fields, function-local declarations and type/value-position directives. Ordinary constants still require initializers. Only a marked configuration contract receives the no-initializer rule above.

Source-level placement includes top-level declaration items in normal files and module roots wherever an ordinary compile-time constant is legal. The existing `export:` block remains a declaration grouping until its later migration, so an otherwise legal `$config` constant there uses ordinary visibility rules. Support and facade declarations keep their existing package-isolation and public-surface restrictions. A purpose directive is unnecessary for compile-time declarations.

The initial type domain stays `String`, `Int`, `Float`, `Bool`, `Char` and one optional layer around those primitives. Retain the early, explicit primitive annotation requirement rather than adding alias resolution, inferred types, `Number`, collections or user-defined records to input contracts. A later numeric/API change needs its own accepted contract.

### Bootstrap versus source defaults

In `config.moth`, a fallback may use the existing allowed single-file compile-time surface and earlier visible helper values. It must finish as a supported primitive or optional primitive. Same-file forward references remain errors. This is evaluation inside the existing config service, not a second evaluator before it.

In source modules, defaults remain self-contained primitive literals or `none`. Names, projections, expressions, templates, calls, casts, collections, records and references to other configuration values remain invalid defaults. Source-contract preparation can normalise the literal without opening provider interfaces or evaluating module AST.

A source contract resolves before that module's AST semantics. Its resolved value is an ordinary folded constant, with no runtime wrapper, new `TypeId` category or HIR directive node.

### Contract identity, matching and precedence

Configuration declarations and project metadata are separate systems.

- Only `$config` declarations create input contracts.
- `$project(version = "1.0")` creates project metadata, not a `version` input provider.
- An ordinary `version #String = "1.0"` is a helper constant, not an input provider.
- Passing `release_version` to `$project(version = release_version)` does not rename its input contract.
- `@project` publishes only the predefined project fields, not all config helpers or input declarations.

Matching declarations within one compilation boundary agree on name, primitive type, optionality, required/default state and normalised default value. Explicit overrides do not excuse conflicting defaults. Same-file duplicate declaration and no-shadowing rules still apply. Matching names in separate selected modules can refer to one input contract without merging their local declaration identities.

Bootstrap contracts resolve in this order:

1. Explicit CLI/programmatic input for that name.
2. A compatible builder-provided primitive global.
3. The declaration's validated fallback or optional absence.
4. A required-input diagnostic.

Selected-source contracts resolve in this order:

1. The already-resolved matching bootstrap `$config` contract, after compatibility validation.
2. Explicit input for a source-only contract.
3. A compatible builder-provided primitive global.
4. The common source fallback or optional absence.
5. A required-input diagnostic.

Remove the fixed-project-field provider tier and its override-blocking semantics. Remove automatic input supply by predefined `$project` fields, even when names happen to match. An explicit input without a bootstrap or selected-source contract is unknown, even if it matches a project metadata field. Diagnose unknown inputs only after the selected source-contract inventory is available.

Preserve the current typed `--input name=value` conversion rules, including lower_snake_case names, splitting at the first `=`, exact Bool/numeric/quoted Char/quoted String recognition, String fallback, malformed-quote rejection and exact type matching. Bare `none` remains String text. A present `T` may satisfy `T?`, but `Int` does not silently satisfy `Float`. Programmatic input uses the same carrier and conversion policy.

Each project/package boundary owns its own namespace. Consumer input does not satisfy dependency contracts implicitly. Keep provenance and stable effective-value fingerprints with the existing owners. Resolution origin and source location are diagnostic provenance, not semantic fingerprint inputs.

### Configuration dependence survives folding

Project identity, source-discovery and compiler-control settings remain fixed-only. Reject `$config` dependence in `$project`'s `name`, `entry_root` and `template_const_loop_iteration_limit` arguments, including dependence through earlier helper constants. An ordinary compile-time value without input dependence remains valid.

A folded value equal to a literal is not evidence that it was fixed. Reuse existing value-dependency/provenance facts to preserve this distinction through folding. Do not discover it by rescanning argument source, and do not add runtime taint metadata.

## 4. `$project`

`config.moth` contains exactly one `$project` invocation. It publishes validated project identity/settings through the existing config result and predefined `@project` interface. It does not create a local source symbol named `project`.

The parameter order is the following order for positional calls:

| Parameter | Type | Default | Configurable value permitted |
|---|---|---|---|
| `name` | `String` | Required | No. |
| `version` | `String` | Required | Yes. |
| `entry_root` | `String` | `"src"` | No. |
| `author` | `String?` | `none` | Yes. |
| `license` | `String?` | `none` | Yes. |
| `template_const_loop_iteration_limit` | `Int` | The existing compiler-owned default limit | No. |

Retain existing field-domain and output/source-path validation. In particular, project name validation and the requirement that a directory project's entry root be strictly beneath its project root remain intact. Preserve the current numeric limit policy through its one owner rather than copying a magic number into multiple signatures.

`version` has no declaration default. `"0.1.0"` belongs to the starter `$config` example, not to `$project`'s signature. A literal version is valid when the author wants a fixed version. A configurable version must come from an earlier explicit `$config` declaration. Missing version is diagnosed even when an unused input contract named `version` exists.

The signature is closed. Reject arbitrary metadata fields, misspelled standard fields and `metadata = ...`. No custom project metadata storage or open-record fallback is retained. Ordinary helper records in config remain ordinary private constants. Project-wide reusable constants belong in ordinary source/support packages. A possible future metadata directive has no current syntax or implementation commitment.

Keep explicit `@project` imports, predefined field identity, field-level provenance and the existing project-package exposure restrictions. Changing required `version` from optional to `String` must be reflected in the synthetic interface and its consumers, not only in config validation.

## 5. `$html_builder`

For an authored HTML directory project, `config.moth` contains exactly one `$html_builder` invocation. Bare `$html_builder` supplies all defaults. The command selects the builder first. This invocation never selects a backend, builder or runtime platform.

Migrate the existing HTML settings into one closed directive signature. The positional order is the table order. Existing spellings remain, including the `html_` prefixes, to keep this work a configuration migration rather than a separate settings redesign.

| Parameter | Type | Default |
|---|---|---|
| `dev_output` | `String` | `"dev"` |
| `release_output` | `String` | `"release"` |
| `origin` | `String` | `"/"` |
| `page_url_style` | `String` | `"trailing_slash"` |
| `redirect_index_html` | `Bool` | `true` |
| `html_lang` | `String` | `"en"` |
| `html_title_prefix` | `String` | `""` |
| `html_title_postfix` | `String` | `""` |
| `html_favicon` | `String?` | `none` |
| `html_inject_charset` | `Bool` | `true` |
| `html_inject_viewport` | `Bool` | `true` |
| `html_inject_color_scheme` | `Bool` | `true` |
| `html_inject_core_css` | `Bool` | `true` |
| `html_body_style` | `String` | `""` |

`page_url_style` retains its closed values `trailing_slash`, `no_trailing_slash` and `ignore`. Preserve origin-path validation, non-empty language and present-favicon checks, output-root containment, distinct profile ownership and manifest-conflict handling. These settings consume permitted folded input values and do not declare their own `$config` contracts.

The signature's defaults replace repeated fallback/default filling in downstream settings readers. Keep specialised domain checks with their existing owners. Configuration signatures and page signatures stay distinct. Preserve explicitly documented HTML document fallbacks, but add no generic cross-scope field inheritance or merge system.

Config still rejects source dependencies and authored file-value paths because its source graph does not exist yet. A favicon here can be a literal URL. Tracked file resources belong in the later root-local `$page` metadata path.

No inactive-builder registration or validation is implemented. Known unavailable directives error. Unknown directives error. Multi-builder source/configuration awaits structural `$feature` selection. The HTML-only initial product does not justify scattering hard-coded builder-name conditionals through compiler semantics.

Synthetic single-file operations without authored `config.moth` keep their explicit synthetic configuration path. They do not invent `$project`/`$html_builder` source declarations, public input providers or an implicit configurable version.

## 6. Bootstrap and compiler handoffs

Preserve one compiler-owned config service and one canonical module service:

```text
Command selects capabilities and types explicit inputs
-> compiler config service prepares the one config source once
-> config constants and explicit input contracts are resolved/folded in source order
-> shared directive call validation produces folded argument facts
-> existing build/config owners apply validated project and HTML settings
-> publish predefined @project fields and bootstrap input results separately
-> Stage 0 constructs the project/provider graph
-> collect and validate selected source input contracts
-> canonical module compilation consumes resolved inputs
```

A config source is not a module and produces no `start`, HIR, borrow facts or runtime output. A root metadata invocation later uses ordinary module AST visibility, not this isolated config service.

Retain source identities, original argument/key locations and live span ownership until the existing terminal producer boundary. Do not freeze source tables too early or retain them forever just for diagnostics. Apply string/identity remaps once through the owning payloads. Follow the activation tree's compact-token and diagnostic contracts rather than restoring older rich token storage.

Later consumers receive typed configuration and metadata. They do not compare directive-name strings, reopen AST, inspect TIR, scan HIR constants or invoke formatter factories to reconstruct meaning.

## 7. Deferred vocabulary and limits

Recognise the agreed future names for useful diagnostics: `$feature`, `$layout`, `$export`, `$async`, `$checked`, `$test` and the queued HTML `$page`. Keep ownership/context information sufficient to distinguish an unknown name, wrong context and recognised-but-unimplemented capability.

A deferred registration cannot validate successfully as a no-op or count as a working root purpose. Move existing reserved/deferred `async:` and `checked:` recognition to the `$async:` and `$checked:` spellings without implementing their bodies. Release their old keyword reservations when that recognition moves. Ordinary identifier and named-region rules then apply to `async` and `checked`, without migration-specific recognition. Retain independent async vocabulary such as `yield` unless its own authority changes. The implemented `export:` keyword block stays intact until the separate export migration.

The final body grammar, predicate grammar and semantic effects are not implemented. For malformed syntax, preserve the most useful existing syntax diagnostic. For a recognisable future use, explain that its support is deferred without pretending full argument/body validation happened.

The future source-block family uses `$name(...): ... ;` or `$name: ... ;`. Preserve only the minimal descriptor/header facts needed to recognise this family. Use existing delimiter/recovery infrastructure. Add no full directive-block AST/HIR, execution engine, inactive-source skipper or broad registry hooks.

Future `$feature` is structural selection before graph/declaration publication. Ordinary static `if` continues to validate both branches before selecting executable work. Unsupported directives inside an ordinary `if false` remain errors. Source-module `$config` values cannot drive pre-graph selection. Future feature declarations belong in config-only compiler directives, with inputs resolved during bootstrap.

The structural feature predicate details, the feature-declaration directive's spelling and `$export` block support stay deferred. Source cannot inspect physical target, backend, OS or architecture. No macros, user-defined directives, arbitrary statement annotations, `shallow_copies` or `compile_time` semantics are added.

## 8. Generated starter and migration

The config generator for `moth new html` must emit this pattern, preserving the existing scaffold's selected project name and any intentionally retained starter settings:

```moth
$config
version #String = "0.1.0"

$project(
    name = "my_site",
    version = version,
    entry_root = "src",
    author = "",
    license = "MIT",
)

$html_builder(
    page_url_style = "trailing_slash",
    redirect_index_html = true,
    html_lang = "en",
)
```

`my_site` represents the existing escaped generated project name. Preserve the current generator's source-string escaping. The version declaration and explicit argument are required starter content, not an optional documentation example.

Migrate all operational `config.moth` inputs and actual `#Config` contracts needed by the repository's commands, fixtures, benchmarks, package scaffolds and docs build. Custom open project metadata must move to ordinary source constants where needed. Update consumers instead of silently deleting still-used data.

Release recognised `project`/`html` record names and `Config` identifier reservations. Ordinary records with these names acquire no settings effect. Missing required directives then use normal missing-configuration diagnostics. Remove qualifier-only parsers, special spacing rules, record scanning and migration-only compatibility paths. A user-defined type named `Config` follows ordinary type syntax, so removal does not mean banning every textual `#Config` substring.

Keep the existing operational page template and `page_*` extraction working at this plan's completion. The later page cutover migrates those together. Template argument-parser consolidation may still require fixture updates here, but it does not authorise early `$page` activation.

## 9. Current owner map

Recheck these anchors at activation:

| Area | Starting locations |
|---|---|
| Directive definitions and registry | `src/compiler_frontend/style_directives/` |
| Lexer and retained declaration syntax | `src/compiler_frontend/tokenizer/`, `headers/`, `declaration_syntax/` |
| Current qualifier syntax | `src/compiler_frontend/declaration_syntax/build_config_contract.rs` |
| Shared call arguments | Focused owner under `src/compiler_frontend/ast/expressions/` |
| Input types and resolution | `src/compiler_frontend/build_config/` |
| Config service | `src/compiler_frontend/single_source_compilation/config.rs` |
| Schema registration | `src/builder_surface/definition.rs`, `config_schema.rs` |
| Config application | `src/build_system/project_config.rs`, `project_config/validation.rs`, `src/projects/settings.rs` |
| HTML settings | `src/projects/html_project/html_project_builder.rs`, `document_config.rs`, `src/projects/routing.rs` |
| Generated project source | `src/projects/html_project/new_html_project/start_page_scaffolding.rs` |
| Command inputs and dev retention | `src/projects/cli.rs`, `check.rs`, `dev_server/` |

Extend or consolidate these owners. Do not add a broad `utils` module or a trait hierarchy merely to hide directive dispatch.

## 10. Implementation phases

### Phase 0: Refresh and contract baseline

- [ ] Read the routed authorities and inspect the rebased source, status and concurrent-work boundaries.
- [ ] Confirm obsolete reactive `$` syntax is gone. Preserve Wiring semantics and native-result-slot work already delivered.
- [ ] Inventory registry, argument, config, scaffold and test owners. Record required removals and the existing page path that stays active.
- [ ] Run the activation baseline gate. Record real failures rather than treating historical results as current.

### Phase 1: Recognition, signatures and shared arguments

- [ ] Add owner/context/form descriptors by extending the existing directive registration authority.
- [ ] Reject registry conflicts deterministically, including duplicate builder registrations.
- [ ] Reuse call syntax and slot routing for template and source directive arguments.
- [ ] Retain balanced invocation ranges during preparation without another expression parser.
- [ ] Add empty-parenthesis, placement, unknown and deferred-name diagnostics.
- [ ] Keep deferred block and purpose forms non-executable. Move existing async/checked deferred diagnostics to directive spellings and update their feature-gated coverage/lane descriptions without implementing blocks. Keep real `export:` support unchanged. Review duplication and syntax regressions.

### Phase 2: Explicit `$config` contracts

- [ ] Replace the qualifier with a prefix on ordinary explicitly typed compile-time declarations.
- [ ] Preserve primitive/default normalisation and implement the exact attachment/placement rules.
- [ ] Make bootstrap contracts independent of `$project` metadata. Remove fixed-field input supply.
- [ ] Keep same-name compatibility, resolution precedence, package isolation and source-only input validation.
- [ ] Preserve input-dependence facts through helpers for fixed-only receiving fields.
- [ ] Migrate source contracts and delete the old qualifier machinery in the same coherent cutover.

### Phase 3: Config directive consumers

- [ ] Implement the exact closed `$project` and `$html_builder` signatures.
- [ ] Require authored name/version and required singleton config directives. Fill defaults once.
- [ ] Apply specialised domain validation through the existing build/config owners.
- [ ] Publish predefined `@project` metadata separately from explicit input-contract results.
- [ ] Remove open metadata, recognised-record extraction and inactive-section handling.
- [ ] Migrate real configuration sources and dependent metadata consumers. Keep single-file synthetic defaults distinct.

### Phase 4: Scaffolding and complete removal

- [ ] Update `config_template` with the required `$config` version example and explicit version argument.
- [ ] Exercise the actual `moth new html` output, not just a hand-written equivalent.
- [ ] Delete obsolete types, schemas, validators, reservations, fallback paths and tests with no remaining owner.
- [ ] Keep page extraction operational and record the precise later page cutover boundary.
- [ ] Update canonical docs, the cheatsheet, both relevant progress matrices and index entries to the actually delivered surface.

### Phase 5: Validation and Slice review

- [ ] Run focused tests during iteration and the code-bearing final gate from the current validation guide.
- [ ] Run the documentation release-build gate after executable documentation migration.
- [ ] Verify each required test contract below has one primary owner and remove redundant fixtures.
- [ ] Perform the Slice review below and record exactly which commands ran.
- [ ] Retire this plan and its roadmap entry in the completion commit after durable decisions and remaining work are in permanent references.

Each phase ends with ownership, duplication, diagnostics and test review. Avoid committing an intermediate state that silently accepts directives without implementing their required behaviour.

## 11. Required coverage

Use manifest-backed language/project cases for public behaviour, subsystem-local tests for parser/registry invariants and compiler integration tests for multi-module boundaries. Extend existing owners rather than copying equivalent tests.

| Contract | Required checks |
|---|---|
| Argument sharing | Positional/named/default combinations, multiline expressions, unknown/duplicate/missing arguments, extra suffixes, nested calls/templates, empty directive `()` rejection and unchanged ordinary `f()`. |
| Template parity | Every existing directive family, slot/index/wrapper interpretation, raw/balanced/discarded bodies, formatter compatibility and const-required arguments. |
| Registry safety | Core override, duplicate builder names, wrong context, unknown directive and known deferred directive. |
| `$config` syntax | Required/present/absent input forms, explicit primitive type, invalid modifier targets, missing target, duplicate modifiers and invalid argument forms. |
| Defaults | Source literal-only rejection, bootstrap earlier folded helpers, forward-reference failure, optional absence normalisation and no type inference from overrides. |
| Input resolution | CLI/global/default order, bootstrap/source matching, conflicting defaults despite override, unknown input after source inventory and package-boundary isolation. |
| Metadata separation | `$project(version = "1.0")` does not provide or block source input `version`. An explicit same-name `$config` does. Renamed argument slots do not rename contracts. |
| Required version | Missing version fails even if a version contract exists. Literal version works. Earlier `$config` version works. `none` fails for required String version. |
| Fixed-only settings | Direct and helper-derived configurable name, entry root and loop-limit arguments fail. Equivalent fixed constants work. |
| Strict project | Unknown/custom/`metadata` arguments fail, predefined `@project` fields remain available and helper/input bindings are not automatically published. |
| HTML settings | All signature defaults, closed URL style, output containment/profile conflicts and browser document defaults remain correct. |
| Reclaimed names | Ordinary `Config`, `config`, `project` and `html` bindings/types work where ordinary naming permits. They have no hidden configuration effects. |
| Stage ownership | Retained syntax consumed once, source/span remaps preserved, no config HIR and no later AST/TIR/name rescanning. |
| Static selection | Both ordinary-if branches remain frontend-valid, inactive runtime requirements disappear normally and unknown directives are not excused by a false condition. |
| Starter output | Generated config includes `$config` followed by `version #String = "0.1.0"`, supplies version explicitly, builds/checks on the current page path and accepts `--input version=0.2.0`. |

Do not add `$test` fixtures as if a user test runner exists. Compiler testing infrastructure remains independent of the future source directive.

## 12. Completion and Slice review

Review changed owners from their entry points. Confirm one authority for parsing, routing, config resolution and typed output, with no wrappers preserving removed APIs. Check source-location accuracy, user diagnostic versus infrastructure lanes, unnecessary allocations, obsolete comments, dead schema machinery and nominal-type pollution.

Confirm the repository can generate and compile a fresh starter project, configuration metadata cannot supply implicit input contracts and missing `$project` version never falls back to `"0.1.0"`.

Run `just validate` or its maintainer-approved replacement from the activation validation guide. Run `moth build docs --release` or the equivalent Cargo command when source docs change. Report any missing gate honestly. Do not mark acceptance solely from tests using private entry points that bypass public command validation.

This work does not implement structural features, layouts, export migration, directive blocks, async, checked analysis, tests, multi-builder source selection or the HTML page cutover.
