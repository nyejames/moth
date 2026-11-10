# Const-default direct top-level templates

## Status

- Status: Queued behind the ongoing `$export` directive work
- Current slice: Design and implementation plan reviewed, code not started
- Blockers: The `$export` header/parser change must merge into main before implementation
- Next action: Refresh this branch from main when requested, inspect the merged header owners and start Phase 0

## Goal and accepted source contract

In an executable `@entry.moth`, direct top-level `[...]` is const-required by default. Only `~[...]` requests runtime fragment publication:

```moth
@content content as introduction

[$in title:Moth]
[$md:# Welcome]
[introduction]

name = "Moth"
~[: Hello, [name].]
```

- **Bare direct `[...]`:** Must fold under the existing const-template rules, even when a runtime fallback would be possible. Contributes a compile-time entry fragment or registered builder-root slot. Produces no runtime fragment statement, runtime slot or template HIR.
- **Explicit `~[...]`:** Contributes a runtime body fragment via the existing entry-start mechanism, even if the content itself can be folded. Const folding within that runtime expression remains permitted. Only the output-producing template's publication *phase* is forced.
- **Placement:** `~` is a direct entry-fragment prefix, not a general template expression operator or reactive/mutable-template feature. Accept `~[...]` and same-line `~ [...]` (horizontal whitespace only). A newline between `~` and `[` breaks the construct. Recognise this using existing `MUTABLE` and `TEMPLATE_HEAD` tokens at an entry statement boundary.
- **Removed syntax:** `#[...]` is invalid as a direct entry fragment. Remove its parser path with no alias, warning, migration-specific error or backwards-compatibility layer. Retain ordinary compile-time `#=` bindings and all other supported `#` syntax.
- **Ordinary templates unchanged:** `[...]` inside function bodies, binding initialisers, expressions, nested templates and `.mtf` sources keeps its current foldable-or-runtime semantics. Only *direct* top-level fragments acquire the const-required default.
- **Builder slots:** A bare direct `[$in name: ...]` continues to target a compiler-validated, builder-registered destination. `~[$in name: ...]` remains invalid as a runtime named insert. Nested, stored and aliased `$in` remain ordinary composition helpers.
- **Comment categories:** A direct bare `[$doc: ...]` produces documentation metadata, not body HTML. `[$note: ...]` and `[$todo: ...]` discard their balanced bodies without evaluating them or producing fragments. These semantic categories take precedence over the default const *output* requirement. An explicit runtime marker on a direct comment (`~[$doc: ...]`, `~[$note: ...]` or `~[$todo: ...]`) is invalid because comments cannot publish runtime output. Preserve `$doc` Markdown folding, source order, site-root spelling and resource-text diagnostics.
- **Root roles:** Only an executable `@entry.moth` (or a valid synthetic entry) may contribute direct output. Ordinary helpers, API-only facades, config and imported/dormant entries preserve their existing rules and diagnostics.

Template body syntax, control flow, slots, wrappers, formatters, structural Strings, numeric semantics and const-loop limits do not change.

## Invariants and implementation boundaries

Stage 2 is the sole owner of direct-fragment recognition, const-header creation, insertion indexes and runtime-fragment counts. Stage 3 retains local declaration ordering. AST/TIR owns template category, validation and folding. HIR only receives runtime fragments. The builder consumes existing fragment and slot metadata without scanning source.

- Keep the existing synthetic `HeaderKind::ConstTemplate`, `TopLevelConstFragment`, `create_top_level_const_template`, TIR preparation/fold authority, `PushStartRuntimeFragment` and runtime insertion-index handoff.
- Header parsing must **consume the `~` marker**, then retain only the original `TEMPLATE_HEAD`-to-close token range for the implicit start body. Do not fabricate an opening token, copy token streams, retokenize or add a lexer mode. Preserve the original source identity and template span.
- Count precisely one runtime slot per valid output-producing `~[...]`. Normal runtime statements, static fragments and comments create none. Static/runtime body order and destination routing must match authored order.
- Explicitly marked runtime fragments keep the existing staged publication rule: publish only after the entry succeeds. Static fragments survive startup failure. The implicit entry remains dormant unless selected, and empty/metadata-only entries remain valid.
- Constant fragments and builder slots keep structural resource/site-root pieces through metadata and URL rendering. Their existing link, reuse, cache/fingerprint and JS/HTML-Wasm paths must consume unchanged semantic facts. Source edits may legitimately invalidate caches; semantic output and runtime requirements must remain equivalent after source migration.
- No second fragment representation, parallel comment evaluator, builder-specific source classification, duplicate authority or speculative server/request rendering feature.

## Required references

Follow `AGENTS.md` and the current worktree's task-reading guides. Read the canonical unsuffixed references for templates (`template-basics.mtf`, `template-directives.mtf`, `template-control-flow.mtf`), constants (`const-templates.mtf`, constant bindings/folding) and project structure (`entry-runtime-and-fragments.mtf`, `entry-config.mtf`). Read the relevant sections in `docs/compiler-design-overview.md` and `docs/build-system-design.md` for Stage 2 ordering, Stage 4 TIR, compiler metadata, entry activation and HTML output. Read `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`, the cheatsheet, progress matrix and roadmap.

The accepted language references are permanent semantic authorities. This file describes implementation work and is deleted when that work lands.

## Code ownership and deletion opportunities

Inspected on this PR's original base. Recheck after the `$export` branch merges and this branch is refreshed.

- **Classify and dispatch:** `headers/top_level_classifier.rs` turns a direct `TEMPLATE_HEAD` into a const item and `MUTABLE` immediately followed by `TEMPLATE_HEAD` into a runtime item. `headers/file_parser.rs` dispatches with the correct cursor position, `NestingDepth`, role checks, source modifiers and export-block rejection.
- **Delete old indirection:** Remove `headers/hash_items.rs`, its `HashItemRequest`, the `HeaderFileItem::Hash` variant and the registration in `headers/mod.rs`. Keep `TokenTag::HASH` and `#=` declaration classification. Other unmatched `HASH` tokens follow the ordinary start-body/error path.
- **Reuse fragment owners:** `headers/const_fragments.rs` owns const-header construction and preferably registration. `file_state.rs` retains counts, header path, spans and runtime insertion indexes. `start_capture.rs` retains balanced runtime-template capture. Avoid an extra forwarding wrapper.
- **Consolidate documentation:** `ast/module_ast/emission/emitter.rs`, `ast/templates/doc_fragments.rs`, `top_level_templates.rs` and `ast/module_ast/finalization/finalizer.rs` should use the single AST/TIR document path described below.
- **Keep runtime body simple:** `ast/statements/body_dispatch.rs` sees only `TEMPLATE_HEAD` after the marker is stripped and continues to emit `PushStartRuntimeFragment`. Reject runtime-marked non-output comments with an authored diagnostic.
- **Check counts and allocation:** Preserve `headers/types.rs` and `parse_file_headers.rs` metadata. Review `arena/capacity.rs` for double-counting const headers and fragment counts now that static fragments are common. Change heuristics only with an explicit sizing rationale or evidence.

Do not create a new `~` token, another general prefix parser, a fragment-mode enum threaded to HIR or a parser in the HTML builder. Where specialization is real (const header versus runtime body capture), keep it local.

### Documentation-comment handoff: required correction

The existing `collect_and_strip_comment_templates` walks `PushStartRuntimeFragment` nodes, but the new unmarked `[$doc]` enters `HeaderKind::ConstTemplate` instead. A simple classifier swap would silently lose `AstDocFragment` metadata, while a normal const fold of `$note`/`$todo` would turn non-output categories into empty body-fragment records.

Implement **one** documentation metadata path at the AST direct-const-template boundary:

1. Parse the default direct template through the existing TIR owner. Classify its finished root by `TemplateType::Comment` versus an output-producing template. Do not reparse the head in Stage 2 or pre-count comments by scanning token text.
2. For `Doc`, fold its Markdown once using the existing doc folding rules and produce `AstDocFragment`. For `Note`/`Todo`, discard without runtime work or evaluating their bodies. Ordinary output templates use the existing const folding result and builder destination logic.
3. Return a small semantic outcome representing **body/slot fragment**, **documentation metadata** or **discarded comment**. Key that outcome by the existing synthetic header path, then collect it by the authored `TopLevelConstFragment` order. This preserves docs order and keeps comments out of body fragment lists. Avoid separate spans-to-order sorting and parallel fold maps.
4. After routing all valid direct documentation through this path, remove the obsolete runtime-body comment-stripping walk and related tests/helpers if they have no other valid caller. If a remaining use exists, identify and justify it before keeping shared code. Reuse its existing Markdown/site-root/resource-text folding logic rather than creating a second implementation.
5. Diagnose `~[$doc]`, `~[$note]` and `~[$todo]` at AST template-category validation. A failed module publishes no partial counts or metadata. Document why the header-level provisional count is not an output commitment.

Retain the canonical distinction: an actual empty `String` fragment is still output, while `$doc`, `$note` and `$todo` are non-output categories.

## Implementation sequence

### Phase 0 - Rebase gate and inventory

1. **After** the `$export` work merges, refresh this branch from main. Re-read `AGENTS.md`, changed authorities and the export/header integration points. Do not implement against old `export:` or stale `HeaderFileItem` cases.
2. Search source/templates and fixture strings for direct `#[...]` and bare runtime `[...]` by context. Prioritise `*.moth`/`*.mtf`, `tests/cases`, docs source, scaffold, packages and benchmarks. Avoid treating Rust `#[...]` attributes and quoted code examples as Moth syntax. Preserve intentional invalid cases.
3. Identify existing primary test owners and record focused baseline results where feasible. This is not a request to introduce a new test framework or pin a pre-implementation commit in the plan.

### Phase 1 - Header and AST cutover

1. Reverse direct-template classification at `top_level_classifier`. Remove the old `HASH` item path and `hash_items.rs` rather than moving its wrapper unchanged. Reuse `create_top_level_const_template` and existing state bookkeeping.
2. Recognise `~` + next-token `TEMPLATE_HEAD` at an actual statement boundary. Make the parser consume the marker and opening, capturing only the balanced template tokens into start-body ranges. Confirm `~foo`, `~=` writes and nested/value-position `~` remain on their existing paths.
3. Preserve precise role/export/source-modifier diagnostics, file-provider ordering, structural resource discovery and declaration counts. Keep synthetic entry and reached-facade handling aligned with the current merged export implementation.
4. Implement the doc/comment outcome and deletion above. Keep runtime annotation validation at the existing AST template-category owner, not as a second head parser.

### Phase 2 - Focused tests and migration

Use nearby header tests (including `parse_file_headers_entry_runtime_tests.rs` and module-role tests), the existing AST template tests and a small number of real `tests/cases` programs. Extend or consolidate existing scenarios before adding new ones.

- **Marker and boundary:** `~[...]`, `~ [...]`, `~\n[...]`, CRLF, indentation, EOF, adjacent entries, `;~[...]` and nested brackets. Removed `#[...]` fails without a migration-specific error.
- **Context:** Active and synthetic entries work. Helper, facade, config and export placements reject appropriately. Ordinary assigned/nested templates and mutable reads/writes retain existing behavior.
- **Const enforcement:** Bare direct literals, imported constants, static `if`/`loop`, resources and named `$in` fold. A runtime-dependent interpolation in bare direct output diagnoses instead of creating HIR.
- **Runtime guarantee:** Both dynamic and completely foldable `~[...]` produce exactly one `PushStartRuntimeFragment` and runtime slot without const-body metadata.
- **Interleaving and failures:** Mix static body fragments, builder slots, two runtime fragments and other statements. Assert order, destinations, resources and success-only runtime publication. A failing entry keeps static output and publishes no staged runtime fragments.
- **Comments:** `[$doc]` produces ordered metadata without HTML or a runtime slot, including Markdown/site-root handling and resource-observation diagnostics. `[$note]`/`[$todo]` discard. `~[$doc]` and other runtime-marked comments reject. Distinguish empty output from no output.
- **Builder parity:** Exercise JS, relevant HTML-Wasm capabilities, resource linking and metadata-only/static-only entries without claiming support for unavailable targets.

During migration, change real `#[...]` entry fragments to `[...]` and bare *runtime* entry fragments to `~[...]`. Update documentation site entries, CLI scaffold, package examples, executable benchmarks, tests, expected diagnostics and integration manifests as needed. Change only actual entry syntax, not ordinary expression templates or quoted Rust attributes. Migrate and run focused tests together rather than expecting the pre-migration suite to pass mid-cutover.

Update canonical unsuffixed template/constant/entry references, paired Basic pages, the cheatsheet and affected compiler/build architecture prose. Replace outdated examples and stale comments. Check syntax highlighting and developer tooling only for actual old-syntax assumptions. Do not edit generated `docs/_release/**` manually. Update progress and `index.md`/audit-log staleness only when the repository rules require it.

### Phase 3 - Integrated validation and Slice review

Run targeted parser/AST tests and selected runtime integration cases after each coherent slice. At the integrated checkpoint run `just validate` plus affected checks not included in the routine gate. Before merging into main, run `just validate-full` against the final integrated tree as required by `validation.mtf`. Build the docs site with `moth check docs --terse` and `moth build docs --release` (or documented Cargo equivalents). Generate and build a CLI starter project.

Audit remaining old entry spellings and runtime bare fragments after source migration. Review by semantic context, not raw grep count.

End with the required Slice review of semantic correctness, explicit stage ownership, no duplicated TIR/documentation paths, deletion of old hash/comment glue, typed diagnostics, meaningful test coverage and documentation consistency. Check that static-heavy pages do not incur newly inflated arena allocations or runtime artifact requirements. Benchmark or profile only when the affected path has a concrete performance hypothesis.

Completion means the new spelling is the sole accepted direct top-level output syntax and all supported examples and backend contracts are validated. Delete this plan **and** its roadmap entry in the same final implementation commit.
