# General directives and project configuration

## Status and scope

- Status: Accepted design, queued after the project-structure and builder-slots work is integrated.
- Roadmap file: `docs/roadmap/plans/general-directives-plan.md`.
- Deliver: a constrained compiler-owned directive grammar, core ownership of existing template formatters, `$config` input contracts and `$entry` entry selection.
- Preserve: optional `config.moth` and its existing named `project` and builder-settings records, including the selected builder's current schema, defaults and validation policy.

The build command selects the builder and compilation boundary before configuration is evaluated. `config.moth` remains a self-contained compile-time bootstrap source, not an ordinary module. Do not move configuration into `@` or `+` roots. Builders provide schemas, packages, source capabilities and output policies, not directive names or parsing and semantic callbacks.

## 1. Directive contract

A directive modifies an existing construct. Template-head directives modify their template, module-root directives modify their module and other source directives modify the following eligible construct.

A directive may supply an additional compiler-owned constraint on its target, but it is not a new declaration, general attribute, user macro, function call or independent control-flow construct. Neither builders nor user code can define, import, alias or override directives. Directive names do not reserve the corresponding ordinary identifiers. **The directive's known kind determines its attachment context; its position never changes that kind.** A source modifier at the beginning of a module file still attaches to the following declaration or statement.

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
- Adjacent call syntax such as `$code("rust")`, `$project(...)` and `$entry()` is invalid. A parenthesised **record value** is allowed only as a single whitespace-separated operand, when the directive's contract permits a record. Multiple positional or named operands and directive-owned argument lists are not supported. Ordinary function calls remain unchanged.
- Template heads already support static slot names and indices. Preserve `$slot title`, `$slot 1`, `$slot title = "Fallback"` and `$in title`. These are one-operand forms and their names are not runtime variable lookups. Preserve the template head's independent compatibility rules and its ability to contain more than one *distinct* compatible directive.

The `builder-slots` prefix/suffix spelling is normalised to the single-value form in this work: `$before value` and `$after value`, rather than `$before = value` and `$after = value`. Preserve all wrapper, named-slot and runtime no-output semantics. The previous assignment spelling is removed without a compatibility mode.

Recognise a directive's known name before checking context. Each static compiler definition specifies eligible targets, phase, operand kind and compatibility. Diagnose unknown names, known names in the wrong context, duplicate or incompatible modifiers, missing/extra/malformed operands, dangling source modifiers and non-constant values at their authored spans. Known future names retain deferred-feature diagnostics where already recognised; recognising a name does not implement its semantics. Avoid a runtime-extensible registry, a catch-all directive-effect engine and extra annotation states on unrelated AST/HIR nodes. **Do not diagnose a misplaced known directive as a generic invalid `# General directives and project configuration

## Status and scope

- Status: Accepted design, queued after the project-structure and builder-slots work is integrated.
- Roadmap file: `docs/roadmap/plans/general-directives-plan.md`.
- Deliver: a constrained compiler-owned directive grammar, core ownership of existing template formatters, `$config` input contracts and `$entry` entry selection.
- Preserve: optional `config.moth` and its existing named `project` and builder-settings records, including the selected builder's current schema, defaults and validation policy.

The build command selects the builder and compilation boundary before configuration is evaluated. `config.moth` remains a self-contained compile-time bootstrap source, not an ordinary module. Do not move configuration into `@` or `+` roots. Builders provide schemas, packages, source capabilities and output policies, not directive names or parsing and semantic callbacks.

## 1. Directive contract

A directive modifies an existing construct. Template-head directives modify their template, module-root directives modify their module and other source directives modify the following eligible construct.

A directive may supply an additional compiler-owned constraint on its target, but it is not a new declaration, general attribute, user macro, function call or independent control-flow construct. Neither builders nor user code can define, import, alias or override directives. Directive names do not reserve the corresponding ordinary identifiers. **The directive's known kind determines its attachment context; its position never changes that kind.** A source modifier at the beginning of a module file still attaches to the following declaration or statement.

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
- Adjacent call syntax such as `$code("rust")`, `$project(...)` and `$entry()` is invalid. A parenthesised **record value** is allowed only as a single whitespace-separated operand, when the directive's contract permits a record. Multiple positional or named operands and directive-owned argument lists are not supported. Ordinary function calls remain unchanged.
- Template heads already support static slot names and indices. Preserve `$slot title`, `$slot 1`, `$slot title = "Fallback"` and `$in title`. These are one-operand forms and their names are not runtime variable lookups. Preserve the template head's independent compatibility rules and its ability to contain more than one *distinct* compatible directive.

The `builder-slots` prefix/suffix spelling is normalised to the single-value form in this work: `$before value` and `$after value`, rather than `$before = value` and `$after = value`. Preserve all wrapper, named-slot and runtime no-output semantics. The previous assignment spelling is removed without a compatibility mode.

 character or an unknown directive.**

### Three attachment locations

- **Template head:** template directives modify their containing template. Static keys and compile-time values belong to existing TIR composition/formatter contracts, not to a new declaration or scope. Template-head rules do not constrain blank lines in rendered body text.
- **Module root:** designated root directives modify the implicit module root and must appear as the first meaningful item of the root file, on their own line. Leading comments and whitespace are permitted. `$entry` is the only such directive in this plan, has no operand and introduces no binding, scope or runtime activation. Blank lines after `$entry` are ordinary source whitespace because it does not target the next item.
- **Source construct:** source modifiers modify one immediately following eligible declaration or statement. A compound target includes its header and body. The modifier must be contiguous with its target, permitting intervening comment-only lines but no empty physical line, unrelated item, intervening scope or new file. Multiple compatible modifiers may stack under the same rule. A prefix in file-initial position is still a construct modifier.

A known name is classified before position or target validation. A misplaced known root directive must never silently attach to the next declaration, and a misplaced construct directive must never silently gain module scope. No general rule allows arbitrary root directives merely because they are first. An invalid modifier must not swallow the next valid item during diagnostic recovery.

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
- Retain the `builder-slots` forms `$slot`, `$in`, `$fresh`, `$before`, `$after`, `$note`, `$todo`, `$doc`, `$raw` and `$md`, including per-occurrence slot defaults, builder-root direct named inserts, formatter conflicts, wrapper opt-outs and `.mtf` default Markdown. Only the `$before`/`$after` operand separator is normalised here.
- The HTML builder continues to receive `title`, `description`, `lang`, `favicon`, `body_style`, `stylesheets` and `head` through the delivered **direct top-level const `$in` slots**. Preserve slot-specific validation, resource-origin deduplication, rendered-head deduplication and JS/Wasm output parity. Do not revive reserved `page_*` metadata or the `$page` proposal.

Delete the merged builder directive API, the HTML directive registration owner, last-registration-wins behaviour and unused formatter adapters. Keep builder-provided slot schemas, packages, config schemas and output capabilities: these are data contracts, not builder-defined directive syntax.

## 4. `$entry` and entry selection

```moth
-- @home.moth
$entry

#[$in title: Home]
#[: <main>Welcome</main>]
```

`$entry` is valid at most once, as the first meaningful item of a normal `@` root or explicitly selected synthetic single-file normal root. Reject operands, call shape, duplicate markers, placement after another source item, inline trailing source on the marker line and use in helpers, `+` roots, facades, test roots, `config.moth`, function/branch bodies, export blocks or template heads. A misplaced known `$entry` must produce a precise placement diagnostic.

It marks only the containing normal root as **eligible** for builder entry selection. It does not activate its implicit `start`, create another function, add a scope, change import visibility or require authored runtime work. Every normal root retains its compiler-owned dormant `start` and receives ordinary type/borrow/lifetime validation whether marked or not. Unmarked roots may contain valid dormant top-level runtime work and direct templates without becoming application entries; imports never activate provider starts. Dependency-package markers do not automatically become consumer application entries.

Publish one non-exported eligibility fact in existing completed root metadata. Stage 0 and the selected builder consume that fact without reparsing source or testing HIR activity, constant names or rendered output. An entry with only static content or named slots is valid. `check` permits zero entries; application `build` and `dev` require at least one suitable marked entry where their selected builder requires application entries. Preserve package-only outcomes, descendant selection, directory-derived HTML routes, no-homepage behaviour, static/runtime fragment ordering, start failure handling and JS/Wasm lifecycle.

Switch HTML selection from activity-based page discovery to marked-root facts. Require `$entry` on actual first-party and scaffolded page roots. Keep the completed builder-slot metadata system; a named `$in` at the root contributes only to the selected active entry. Remove the previous unpurposed-root-work rejection, old activity-based page emission and any surviving reserved `page_*` extraction. Reject `$page` rather than treating it as an alias or no-op.

## 5. Owners and implementation sequence

Use current paths as locators and reconcile them against the integration tree. Preserve one semantic owner per fact:

- `src/compiler_frontend/style_directives/` and `ast/templates/` own registered compiler template names, template-head parsing, formatters and TIR composition.
- Existing tokenization/header preparation and `declaration_syntax/` own source directive recognition and attachment; retain compact modifier facts with the declaration/statement shell. No source rescans or second header grammar.
- `src/compiler_frontend/build_config/` and `single_source_compilation/config.rs` own exact input resolution and config-source folding. `src/build_system/project_config/` and `builder_surface/config_schema.rs` apply folded `project` and active-builder records unchanged.
- `module_compilation/`, the completed root metadata lane, Stage 0 selection and `projects/html_project/` own entry eligibility, builder selection and final output. Builders consume completed facts, never AST/TIR authority.

Retain authored source spans until their final diagnostics are emitted, then use the existing source-table freeze boundary. Hand off typed folded facts and stable names, not complete AST/TIR stores. Preserve existing config/input, project-field, entry, implementation, resource and physical-variant invalidation domains. Directive syntax should not add a second cache-key or runtime representation.

### Implementation phases

1. **Integration baseline and canonical docs.** Confirm the merged project-structure and `builder-slots` contracts, current `AGENTS.md`, language/implementation style, testing guide and compiler/build design authorities. Run focused baseline tests for config/defaults, input contracts, formatter output, wrapper/slot behaviour, HTML entry routes and JS/Wasm builds. Align accepted-design documentation to the existing named `project`/builder records **before** documenting directive grammar; retain truthful current-support labels until implementation lands. Verify the selected builder's present section requirements from the live schema.
2. **Directive grammar and template migration.** Extend the compiler's existing static template specifications with the smallest source modifier/entry recognition needed. Route directive kinds to three attachment contexts without introducing separate directive parsers or a general attachment object. Track whether the preceding source boundary contained an empty physical line or only contiguous comments. Enforce single operands, same-scope attachment, ownership, placement and invalid call shapes. Migrate `$code` and applicable `$css` operands and `$before`/`$after` values without changing template semantics. Move HTML formatter implementations into the frontend and delete builder registration paths.
3. **Typed input cutover.** Attach `$config` to ordinary typed declaration shells, reuse existing prepared contract facts and config resolver, migrate `#Config of T` authoring and remove obsolete qualifier parsing. Keep named configuration records and active builder schemas untouched.
4. **Entry cutover.** Publish `$entry` root facts, select marked HTML entries, migrate first-party sites and `moth new html` templates and remove activity-based selection and old root-purpose diagnostics. Confirm direct named builder slots remain restricted to marked active entries.
5. **Cleanup and validation.** Remove dead paths, stale tests and duplicated parsing or evaluation. Update permanent docs, source examples, cheatsheet, progress matrices and roadmap to match delivered behaviour. Reconcile the queued HTML page/runtime-title work to retain only independently required runtime-title work. Complete focused and full validation and a code-bloat audit.

Documentation owners to reconcile: `docs/docs/directives/{directives,directives-basic}.mtf`, `docs/docs/project-structure/{project-config,project-config-basic,build-inputs,build-inputs-basic,entry-config,entry-config-basic}.mtf`, template directive/slot/wrapper references, `docs/compiler-design-overview.md`, `docs/build-system-design.md`, the cheatsheet, progress matrices, scaffold docs and `docs/roadmap/roadmap.md`. In particular, remove claims that `$project`, `$builder`, `$html_builder` or `$page` are accepted future directives, and replace architecture text requiring directive bootstrap arguments or metadata recovered from `page_*` names. The named-record project and active-builder schema contracts are the accepted configuration design. Update future constraint/test/layout/feature plans only where their assumed directive grammar or attachment boundaries conflict with the single-operand rule; do not implement those features here.

## 6. Acceptance tests and gates

Use existing subsystem tests and project/CLI integration harnesses. Put tests under the repository's `tests/` policy. Reuse fixtures and test actual public outcomes rather than introducing an independent directive harness.

- **Grammar and attachment:** bare, single-value and `key = value` operands; template named/positional slot forms; const-only resolution and operand type/phase errors; balanced multiline operands; no operand type annotations, directive calls or multi-argument lists; ordinary `f()` unchanged. Test all three distinct locations, first-file construct modifiers, following-item target restrictions, compatible stacked modifiers, misplaced root markers and comments/blank-line boundaries below. Reject general `$name:` blocks, stray directives and deferred names with context-specific diagnostics.
### Placement and diagnostic matrix

Use focused lexer/header tests for boundary recognition and existing project-command tests for semantic attachment. Include both positive and negative cases for every supported directive context. Diagnostics must be deterministic, identify the **directive site**, include the target site when present and recommend the correct context or immediate action. A second location is mandatory for duplicate and conflicting modifiers, and where an unrelated intervening declaration blocks attachment. Use the existing typed diagnostic reason and source-span infrastructure rather than adding string-matching parser heuristics.

| Situation | Required outcome |
|---|---|
| `$entry` first meaningful item of a normal `@` root, including a comment banner or leading whitespace | Valid; marks the implicit module root |
| `$entry` after a declaration, runtime statement, template, export group or another directive | Error: root directive must be first; name the root and point to the authored `$entry` |
| Second `$entry` or the same root marker on a helper, `+` facade, test root, `config.moth` or nested scope | Error: duplicate with original site or wrong root/context |
| `$entry` with a value, call suffix, trailing source or on the same line as another item | Error: root directive occupies its own line and takes no operand |
| `$config` at the beginning of an eligible source/config file followed by the correct declaration | Valid construct modifier, never root directive |
| `$config` followed immediately by its declaration, or by one or more adjacent comment-only lines | Valid; comments do not consume the target |
| `$config` followed by one or multiple empty lines, including whitespace-only lines or an empty line between comment lines | Error: modifier separated from target; no declaration is silently claimed |
| `$config` at EOF, before a close-scope, before a function or another ineligible construct | Error: dangling or wrong-target directive, with expected declaration kind |
| Compatible source modifiers adjacent to each other then a declaration | One attachment to the declaration; incompatible or repeated modifiers identify both sites |
| `$md` as bare source, `$slot` inside a function, `$config` in a template head, `$entry` in a template head | Known directive in wrong context; no generic invalid-character error |
| `$slot` and `$in` inside valid heads, invalid heads, or alongside an incompatible formatter | Existing template attachment, exclusivity and conflict diagnostics preserved |
| Unknown spelling, deferred feature, malformed/extra operand or runtime-dependent operand | Distinct error reason and precise authored span; no successful no-op |
| LF, CRLF, leading indentation, comments at EOF and balanced multiline operand values | Same semantics and primary diagnostic location across newline forms |
| Malformed directive followed by an ordinary declaration or template | Recover without consuming the unrelated item or hiding its own diagnostics |

For examples, `$config\nvalue #String` attaches, `$config\n-- explanation\nvalue #String` attaches and `$config\n\nvalue #String` is rejected. Here `\n` denotes one physical line break. A template directive's body may contain blank lines, and the text following an already valid `$entry` is not a next-construct attachment. Do not introduce a new directive family or enable deferred semantics solely to satisfy this matrix; implement the applicable cases alongside each directive's activation.

- **Formatting:** equivalent `$code` language aliases, HTML/Markdown/CSS/escaping, highlighter classes, CSS warnings and optional inline mode where still supported; nested formatting, `.mtf` defaults, TIR opaque/resource anchors, compile-time/runtime wrappers, `$fresh`, named-slot defaults and contributions and output parity under a minimal non-HTML builder.
- **Inputs/config:** optional/required/defaulted `$config`; source-literal versus bootstrap-helper fallback restrictions; rejected `#Config of T`; exact U64 and F16/F32 numeric cases, optional wrapping and unsupported input types; same-name conflicts, explicit precedence, unknown-input timing, fingerprint/provenance and independent package namespaces. Preserve current `project #= (...)`, active `html #= (...)`, project-field fixed-only rules, omitted-config defaults, active-section requirement and unknown ordinary helper behaviour. Confirm no directive config syntax is accepted.
- **Entries:** placement-first `$entry` diagnostics, comments, duplicates and invalid contexts; marked static/metadata-only/runtime and descendant entries; unmarked but validated dormant work; `check` with zero entries; no-entry application errors; package-only/facade roots; synthetic single-file root; no-homepage routes; `start` error behaviour; JS and Wasm outputs; direct root `$in` eligibility and resource emission.
- **Migration:** first-party docs site, scaffold, representative packages, paired `@`/`+` roots, independently selected nested build roots and both output profiles. Confirm no surviving builder directive registry, `#Config` qualifier parser, reserved `page_*` extraction or operational `$page`, `$project`, `$builder` and `$html_builder` paths.

Run focused tests throughout, `just validate` at subsystem integration, `just validate-full` on the final tree and the prescribed docs release build. Record commands, results and any genuine blockers. Review all touched paths against `moth-code-review-guide.md`: remove duplicate evaluators, unnecessary clones, large temporary stores, redundant wrappers, stale comments, unused metadata, oversized fixtures and diagnostics losing source spans. No phase may leave two supported public syntaxes for one feature.

## Exclusions and completion

This work does not implement `$export`, `$test`, `$layout`, `$feature`, `$fast_math`, `$safe_math`, `$infallible`, `$async`, `$checked`, `$mon`, speculative constraints, slot `$required`/`$unique`, arbitrary const-function formatter evaluation, automatic stylesheet dependency propagation, a new config-file format or a Moth-native build script. The separate CSS inline-only redesign and runtime `io.set_title` remain independently scoped. No generic directive extension API, conditional config evaluation, custom metadata registry or general source macro facility is introduced.

Complete when all implemented directives obey the constrained attachment and operand grammar (apart from root-only `$entry`), the frontend owns all existing template formatters, `$config` uses the existing exact build-input service, `config.moth` still uses named project/builder records and `$entry` alone determines HTML application entry eligibility, with canonical docs and tests matching the delivered surface.
