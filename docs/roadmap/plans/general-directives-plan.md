# General directives and project configuration

## Status and scope

- Status: ACTIVE. Fixed root roles and builder slots are integrated.
- Current slice: Final review and closeout.
- Blockers: None known.
- Next action: Run the final review gate, confirm the checkpoint sequence and close the plan.
- Roadmap file: `docs/roadmap/plans/general-directives-plan.md`.
- Deliver: a constrained two-context directive grammar, frontend ownership of existing template formatters and `$config` input contracts.
- Preserve: optional `config.moth` and its existing named `project` and builder-settings records, including the selected builder's current schema, defaults and validation policy.

Delivered: cleanup removed the last qualifier-era scans, duplicate contract facts, dead cursor helpers and unused directive-spec vocabulary; the roadmap, design scope, cheatsheet and progress matrices describe the delivered surface. `just validate-full` passes.

Delivered: `$config` carries build-input contracts. The marked binding's ordinary `#T` annotation is the contract type, its initializer is the fallback, and only marked bindings may omit one. The retired input qualifier no longer parses, and authored sources, tests, CLI fixtures and docs use the directive.

Delivered: source directives are recognised before context. `$config` is the only source modifier; `$name` lexes in every mode, a known name in the wrong context is a typed error, and attachment breaks on a blank physical line, EOF or a non-declaration item. A marked binding records `Header::config_input`.

Delivered: the frontend owns `$html`, `$css`, `$escape_html` and `$code` with `StyleDirectiveRegistry::merged` and the builder directive hook deleted. Template-head operands are one bare compile-time value, an attached `(` is rejected as the removed call spelling, and `$before`/`$after` take one bare operand. Authored sources, templates, fixtures, benchmarks and the shipped wrapper package use the bare spellings.

Baseline: `cargo test -p moth --lib` filters `build_config`, `style_directives` and `project_config` passed (34, 14 and 61 tests). Focused integration cases `template_style_code_success`, `template_style_css_success`, `html_builder_slot_head_dedup`, `template_runtime_children_wrapper_applied_once`, `template_children_named_slot_wrapper_application`, `html_builder_root_slot_metadata_only_entry` and `html_builder_slot_metadata_scalars` passed their configured backends. `cargo run --quiet -p moth-cli --bin moth -- build docs --release` built 88 files. HTML's live schema makes its configuration section optional.

The build command selects the builder and compilation boundary before configuration is evaluated. `config.moth` remains a self-contained compile-time bootstrap source, not an ordinary module. Do not move configuration into structural module roots. Builders provide schemas, packages, source capabilities and output policies, not directive names or parsing and semantic callbacks.

## 1. Directive contract

A directive modifies an existing construct. Template-head directives modify their template and source directives modify the following eligible construct.

A directive may supply an additional compiler-owned constraint on its target, but it is not a new declaration, general attribute, user macro, function call or independent control-flow construct. Neither builders nor user code can define, import, alias or override directives. Directive names do not reserve the corresponding ordinary identifiers. **The directive's known kind determines its eligible target.** A source modifier at the beginning of a file still attaches to the following declaration or statement. No directive attaches to a module root.

### Forms and operands

A directive has **at most one optional compile-time operand**:

```text
$name
$name value
$name key = value
```

- `value` uses the existing compile-time expression grammar accepted for a const-record field value. A directive may restrict the shape and semantic type of that value, or require it to be known at an earlier compiler phase. Do not add a second directive expression parser or evaluator.
- `key = value` is **one named operand**, not a parameter list. The key is a statically recognised name with no explicit type annotation. Some template directives use the key as a slot identifier and the value as its default. The directive determines whether a bare operand denotes a static label, a positional index or a compile-time value.
- No directive operand may contain an authored type annotation, runtime value, side effect, escaping control flow or mutable/exclusive evaluation. The type of an operand is resolved from its ordinary expression and the directive's fixed contract. Type annotations on the **following declaration**, such as `api_url #String`, remain ordinary language syntax.
- An operand begins on the directive's line; a balanced value may continue over subsequent lines. A bare directive ends at its line boundary. A source directive occupies its own line and attaches to the immediately following eligible construct on the next meaningful line, **without intervening blank physical lines**. Contiguous comment-only lines are permitted, but a blank line within the comment run breaks attachment. Leading indentation is not an empty line. Template heads use their existing `,`, `:` and `]` boundaries. CRLF and LF observe identical logical boundaries.
- Adjacent call syntax such as `$code("rust")` or `$config()` is invalid. A parenthesised **record value** is allowed only as a single whitespace-separated operand, when the directive's contract permits a record. Multiple positional or named operands and directive-owned argument lists are not supported. Ordinary function calls remain unchanged.
- Template heads already support static slot names and indices. Preserve `$slot title`, `$slot 1`, `$slot title = "Fallback"` and `$in title`. These are one-operand forms and their names are not runtime variable lookups. Preserve the template head's independent compatibility rules and its ability to contain more than one *distinct* compatible directive.

The `builder-slots` prefix/suffix spelling is normalised to the single-value form in this work: `$before value` and `$after value`, rather than `$before = value` and `$after = value`. Preserve all wrapper, named-slot and runtime no-output semantics. The previous assignment spelling is removed without a compatibility mode.

Recognise a directive's known name before checking context. Each static compiler definition specifies eligible targets, phase, operand kind and compatibility. Diagnose unknown names, wrong-context uses, duplicate or incompatible modifiers, malformed operands, dangling modifiers and non-constant values at authored spans. Known future names retain deferred-feature diagnostics where already recognised; recognition does not implement their semantics. A known misplaced directive must not collapse into a generic invalid-character or unknown-name diagnostic. Avoid a runtime-extensible registry, catch-all effect engine or annotation states on unrelated AST/HIR nodes.

### Two attachment locations

- **Template head:** template directives modify their containing template, using existing TIR composition and formatter contracts. Template-head syntax, head compatibility and body whitespace are validated by the existing template parser.
- **Source construct:** source modifiers modify the immediately following eligible declaration or statement. A compound target includes its own header and body. The modifier occupies its own line; contiguous comment-only lines may intervene, but a blank physical line, unrelated item, intervening scope or file boundary breaks attachment. Compatible modifiers may stack. A modifier at the beginning of a file still targets the following construct.

The compiler classifies each directive by its name and its permitted target context, not by whether it appears near the top of a file. No module-root directive category or root attachment mechanism exists. An invalid source modifier must not consume the next valid item during recovery.

Existing proposal-only block spellings such as `$async:` and `$checked:` are **not** authorised by this grammar. Any future syntax introducing a new execution scope or control flow requires its own language-design decision.

## 2. Configuration and build-input contracts

Keep the existing configuration model. In the selected build directory, `config.moth` is optional. When present it uses ordinary top-level compile-time constants, including the special **names** `project` and the selected builder's registered configuration section (for example `html`). These names have no special status outside the config source.

```moth
-- config.moth
$config
version #String? = "1.0.0"

project #= (
    name = "my_site",
    version = version,
)

html #= (
    origin = "/",
)
```

The project record is optional and defaults follow the completed project-structure contract. Retain existing derived-name validation, optional project version, `@project` synthetic fields, fixed-only settings, exact diagnostic limits, output-root normalisation and source/package visibility restrictions. The active builder section remains subject to **its existing required/optional presence rule**, closed schema and domain checks. Missing `config.moth` retains the existing default path. Do not make the active section optional or create inactive-section syntax-only evaluation as part of this work.

Unknown top-level names remain ordinary private config helper constants. They do not configure the project or selected builder, but still obey normal config-source syntax, declaration ordering and constant-folding validation. Names belonging to other builders receive no special treatment unless they are the selected builder's registered section; this is **not** a promise to skip semantic validation of their expressions. Builder selection never comes from the source, and a nested `config.moth` is inert unless its directory is separately selected as a build root.

Do not introduce `$project`, `$builder`, `$html_builder`, a generic builder-settings directive, source-level builder selection or a second config language. Reuse `ConfigSchemas`, the existing folded named-record extraction and the canonical config compiler service. Only the explicit input-contract spelling changes in this plan.

### `$config`

`$config` takes no operand and modifies exactly one explicitly typed top-level compile-time `#` binding in either selected `config.moth` or an eligible selected source module:

```moth
$config
api_url #String

$config
max_retries #I32 = 3

$config
label #String? = none
```

The binding name is the input key, its ordinary declared type is the exact receiving contract and its initializer is the missing-input fallback. Only marked bindings may omit an initializer. Reject inferred `#=`, runtime or mutable bindings, multiple targets, fields, parameters, local declarations and duplicate or dangling `$config` modifiers. Remove `#Config of T` syntax, not the underlying typed-input contracts or their resolution service.

- Supported types: `String`, `Bool`, `Char`, `I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32`, `U64`, `F16`, `F32`, `F64` and one optional layer. Reject `Byte`, `Dec`, aliases, user-defined and aggregate types.
- A non-optional `T` without initializer requires an external input. With an initializer it uses that fallback. An optional `T?` without initializer or with `= none` defaults to absence. A present optional initializer supplies the fallback value.
- Config-source fallbacks reuse the current self-contained config evaluator and may depend on earlier permitted helper constants. Selected-source fallbacks remain self-contained primitive literals or `none` because contract preparation precedes full AST binding. No imports, forward references or runtime expressions are enabled in config bootstrap.
- All selected bootstrap contracts resolve unconditionally even when no config field consumes them. Source contracts follow the existing selected-source and check-only inventory rules. Normal `if` never conditionally creates or removes a contract.
- Same-name contracts within a compilation boundary must match exact type, optionality, required/default state and normalised fallback, even if explicitly overridden. Distinct module bindings retain independent identities and separately compiled dependency packages retain independent input namespaces.
- Bootstrap precedence is explicit CLI/programmatic input, compatible selected-builder primitive global, fallback/absence, then required-input error. Source precedence is compatible resolved bootstrap contract, explicit source-only input, compatible builder global, fallback/absence, then required-input error.
- Retain the delivered exact CLI literal spelling/category path, direct destination materialisation, fixed-width range/rounding and finite checks, typed API matching, optional wrapping, diagnostic provenance and value/dependency fingerprints. Never pass numeric values through an implicit `I32` or `F64`. Report unknown explicit inputs only after selected-source contract discovery. Ordinary helpers and `@project` fields never implicitly declare inputs.

Migrate `#Config of T` to `$config` plus `#T` in real sources, first-party templates, tests and documentation. Reuse the existing contract normalisation and `src/compiler_frontend/build_config/` implementation rather than duplicating it for directive metadata. Remove the old qualifier parser and any redundant wrappers only once its remaining consumers have migrated.

## 3. Compiler-owned template formatters

Move `$html`, `$css`, `$escape_html` and `$code` from HTML-builder registration into the frontend's existing template directive owner. They are available under every builder. Transfer formatter code, shared validation/escaping helpers and focused tests; do not clone them or leave the frontend calling into `projects/html_project/`. Keep the existing HTML builder's document shell and styling asset/output policies where they belong.

- Retain formatter results, warnings, HTML escaping, indentation handling, supported code-language aliases and highlighted CSS classes. `$code` continues to suppress nested template interpretation; CSS retains its balanced-body handling. Preserve opaque TIR anchors and structural resource pieces.
- The call-shaped code formatter operand becomes a single compile-time operand, for example `[$code "rust": ...]`. Its supported language values and early lexical requirements remain unchanged. Do not add arbitrary runtime language selection or a new formatter-expression engine.
- CSS's eventual inline-only design is separate. If its mode split remains at activation, keep current modes and change `$css("inline")` to `[$css "inline": ...]` using the single-operand grammar. If the inline-only design has already landed, retain that grammar and do not restore the old mode or its operand. Do not move stylesheet assembly into the directive subsystem.
- Retain the `builder-slots` forms `$slot`, `$in`, `$fresh`, `$before`, `$after`, `$note`, `$todo`, `$doc`, `$raw` and `$md`, including per-occurrence slot defaults, direct entry-root named inserts, formatter conflicts, wrapper opt-outs and `.mtf` default Markdown. Only the `$before`/`$after` operand separator is normalised here.
- The HTML builder continues to receive `title`, `description`, `lang`, `favicon`, `body_style`, `stylesheets` and `head` through the delivered **direct top-level const `$in` slots in `@entry.moth`**. Preserve slot-specific validation, resource-origin deduplication, rendered-head deduplication and JS/Wasm output parity. Do not revive reserved `page_*` metadata or the `$page` proposal.

Delete the merged builder directive API, the HTML directive registration owner, last-registration-wins behaviour and unused formatter adapters. Keep builder-provided slot schemas, packages, config schemas and output capabilities: these are data contracts, not builder-defined directive syntax.

## 4. Entry and module integration boundary

The build-system design owns root filenames, visibility, dormant `start`, entry eligibility, source discovery, builder selection, package facades, command routing and entry-specific tests in [module roots and entry roles](../../build-system-design.md#module-roots-and-entry-roles) and [entry and package link planning](../../build-system-design.md#entry-and-package-link-planning). Its accepted roles are `@mod.moth` for the normal API facade, `@package.moth` for the scoped or external package facade and `@entry.moth` for executable entry work. One directory remains one structural module and its available roles share one atomic compilation outcome.

This directive work neither registers nor implements a module-root directive. A source annotation never selects an entry. The builder consumes the completed `@entry` presence fact from root-role implementation, not authored activity or rendered contents. Direct root `$in` contributions retain their ordinary template-head parsing and reach builder-owned destinations only under the selected active entry. Config remains independent in `config.moth`.

The fixed-root-roles cutover is complete and this plan assumes its filename and root-role contracts. Do not migrate root filenames, alter interface publication, change source discovery or rebuild entry planning here.

`$export` remains a future declaration modifier, not a module-root directive. Its accepted scope is declarations in `@mod.moth` and `@package.moth`. It is never accepted in `@entry.moth`, ordinary helpers, templates or `config.moth`. This plan defines no operand or attachment grammar for it and does not implement it.

## 5. Owners and implementation sequence

Use current paths as locators and reconcile them against the integration tree. Preserve one semantic owner per fact:

- `src/compiler_frontend/style_directives/` and `ast/templates/` own registered compiler template names, template-head parsing, formatters and TIR composition.
- Existing tokenization/header preparation and `declaration_syntax/` own source directive recognition and attachment; retain compact modifier facts with the declaration/statement shell. No source rescans or second header grammar.
- `src/compiler_frontend/build_config/` and `single_source_compilation/config.rs` own exact input resolution and config-source folding. `src/build_system/project_config/` and `builder_surface/config_schema.rs` apply folded `project` and active-builder records unchanged.
- The build-system design owns `@entry` eligibility, `start`, Stage 0 selection and entry assembly in [module roots and entry roles](../../build-system-design.md#module-roots-and-entry-roles) and [entry and package link planning](../../build-system-design.md#entry-and-package-link-planning). This plan consumes those facts where template builder-slot validation requires entry context; no source directive republishes entry facts.

Retain authored source spans until their final diagnostics are emitted, then use the existing source-table freeze boundary. Hand off typed folded facts and stable names, not complete AST/TIR stores. Preserve existing config/input, project-field, entry, implementation, resource and physical-variant invalidation domains. Directive syntax should not add a second cache-key or runtime representation.

### Implementation phases

1. **Integration baseline and canonical docs.** Confirm the merged fixed-root-roles and `builder-slots` contracts, current `AGENTS.md`, language/implementation style, testing guide and compiler/build design authorities. Run focused baseline tests for config/defaults, input contracts, formatter output, wrapper/slot behaviour, HTML entry routes and JS/Wasm builds. Align accepted-design documentation to the existing named `project`/builder records **before** documenting directive grammar; retain truthful current-support labels until implementation lands. Verify the selected builder's present section requirements from the live schema.
2. **Directive grammar and template migration.** Extend existing static template specifications with only the source modifier recognition needed. Route directive kinds to two attachment contexts without introducing separate parsers or a general attachment object. Track whether the preceding source boundary contained an empty physical line or only contiguous comments. Enforce single operands, same-scope attachment, ownership, placement and invalid call shapes. Migrate `$code` and applicable `$css` operands and `$before`/`$after` values without changing template semantics. Move HTML formatter implementations into the frontend and delete builder registration paths.
3. **Typed input cutover.** Attach `$config` to ordinary typed declaration shells, reuse existing prepared contract facts and config resolver, migrate `#Config of T` authoring and remove obsolete qualifier parsing. Keep named configuration records and active builder schemas untouched.
4. **Cleanup and validation.** Remove dead paths, stale tests and duplicated parsing or evaluation. Update permanent docs, source examples, cheatsheet, progress matrices and roadmap to match delivered behaviour. Complete focused and full validation and a code-bloat audit.

Documentation owners to reconcile: `docs/docs/directives/{directives,directives-basic}.mtf`, `docs/docs/project-structure/{project-config,project-config-basic,build-inputs,build-inputs-basic}.mtf`, template directive/slot/wrapper references, `docs/compiler-design-overview.md`, `docs/build-system-design.md`, the cheatsheet, progress matrices and `docs/roadmap/roadmap.md`. Fixed roots, entry config, scaffolds and routing are owned by [module roots and entry roles](../../build-system-design.md#module-roots-and-entry-roles) and [entry and package link planning](../../build-system-design.md#entry-and-package-link-planning) in the build-system design. In particular, remove claims that `$project`, `$builder`, `$html_builder` or `$page` are accepted future directives, and replace architecture text requiring directive bootstrap arguments or metadata recovered from `page_*` names. The named-record project and active-builder schema contracts are the accepted configuration design. Update future constraint/test/layout/feature plans only where their assumed directive grammar or attachment boundaries conflict with the single-operand rule; do not implement those features here.

## 6. Acceptance tests and gates

Use existing lexer/header, template, config and project/CLI test owners. Preserve typed diagnostics with accurate authored spans, error recovery and LF/CRLF parity. Each case should exercise a distinct failure.

### Placement and diagnostic matrix

| Situation | Required outcome |
|---|---|
| `$config` begins a selected source or config file and immediately precedes a typed compile-time binding | Valid next-construct modifier, no file-wide meaning |
| `$config` followed by contiguous comment-only lines and then its declaration | Valid, comments do not consume the target |
| One or more blank lines between `$config` and its target, including whitespace-only and comments separated by blank lines | Error at directive: attachment broken |
| `$config` at EOF, before unrelated code, wrong declaration kind, close scope or nested/local scope | Specific dangling or wrong-target error, preserve the next item |
| Compatible consecutive source modifiers | Attach to the same eligible construct; repeated or conflicting modifiers cite both sites |
| Known template directive in ordinary source or `$config` inside template head | Known wrong-context diagnostic, not unknown name or generic invalid character |
| Unknown directive, deferred feature, malformed/runtime operand, type annotation or extra operand | Distinct typed error with correct span |
| `$slot`, `$in`, `$before`, `$after` and `$fresh` in compatible/incompatible template heads | Existing template composition and exclusivity diagnostics |
| Bare, value and named-value operands plus balanced multiline values | Accept only each directive's supported compile-time form |
| LF/CRLF, indentation, comments at EOF and malformed directives followed by valid source | Equivalent semantics, deterministic diagnostics and no swallowed source |

No module-root directive is registered or accepted, even when written at the beginning of a file. `@entry` selection and export isolation follow [module roots and entry roles](../../build-system-design.md#module-roots-and-entry-roles).

### Functional and migration coverage

- **Formatting:** preserve `$code` language aliases, HTML/Markdown/CSS/escaping, highlighted classes, warnings, nested formatting, `.mtf` defaults, TIR/resource anchors, runtime/const wrappers, `$fresh`, named slot defaults and contributions and output parity with a minimal non-HTML builder.
- **Inputs/config:** test required/optional/defaulted `$config`, exact types and numeric materialisation, config helpers and source literals, duplicate contracts, resolution precedence, fingerprints, invalid contexts, unknown input timing and independent package namespaces. Reject old `#Config of T` syntax only when the syntax cutover lands. Preserve `project #= (...)` and active builder records and schema defaults.
- **Entry integration smoke tests:** verify direct `@entry.moth` `$in` slot consumption for static/metadata-only entries. Root discovery, filenames, selection, exports, no-entry outcomes, starts and JS/Wasm lifecycle remain owned by [module roots and entry roles](../../build-system-design.md#module-roots-and-entry-roles) and [entry and package link planning](../../build-system-design.md#entry-and-package-link-planning).
- **Cleanup:** verify no surviving builder directive registry, legacy `#Config` qualifier path, call-shaped general directive parser, duplicated evaluator or operational `$project`, `$builder` and `$html_builder` path.

Run focused tests during implementation, `just validate` after subsystem integration, `just validate-full` on the final tree and the prescribed docs build. Record commands/results and real blockers. Audit against `moth-code-review-guide.md` for duplicated evaluators, redundant wrappers, stale metadata, unnecessary allocations, lost source spans and compatibility shims.
## Exclusions and completion

This work does not implement `$export`, `$test`, `$layout`, `$feature`, `$fast_math`, `$safe_math`, `$infallible`, `$async`, `$checked`, `$mon`, speculative constraints, slot `$required`/`$unique`, arbitrary const-function formatter evaluation, automatic stylesheet dependency propagation, a new config-file format or a Moth-native build script. The separate CSS inline-only redesign and runtime `io.set_title` remain independently scoped. No generic directive extension API, conditional config evaluation, custom metadata registry or general source macro facility is introduced.

Complete when every implemented directive uses only template-head or next-construct attachment with one optional compile-time operand, the frontend owns all existing template formatters, `$config` uses the existing exact build-input service and `config.moth` retains named project/builder records. Entry eligibility is exclusively owned by fixed root roles; canonical docs and tests match the delivered surface.
