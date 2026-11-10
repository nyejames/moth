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

| Existing owner | Required action |
|---|---|
| `headers/top_level_classifier.rs` | Change `TEMPLATE_HEAD` at a top-level statement boundary to a const-fragment item. Classify `MUTABLE` + immediately following `TEMPLATE_HEAD` as runtime, without accepting a newline. Keep `HASH` as an ordinary token when it is not part of another valid declaration. |
| `headers/file_parser.rs` | Dispatch both direct forms with the correct pre-advanced cursor position and role checks. Preserve `NestingDepth` handling, source-modifier rules and export-block rejection. |
| `headers/hash_items.rs` and `headers/mod.rs` | Delete the obsolete hash-item module, `HashItemRequest` and `HeaderFileItem::Hash` branch once the new const dispatch owns its work. Do **not** delete `TokenTag::HASH` or its `#=` declaration classification. |
| `headers/const_fragments.rs`, `file_state.rs` and `start_capture.rs` | Reuse const-header construction and balanced runtime-template capture. Keep const count, header path, span and runtime insertion index in the existing file-state owner. Prefer placing const-fragment registration alongside its current construction logic rather than introducing another forwarding wrapper. |
| `ast/module_ast/emission/emitter.rs`, `ast/templates/doc_fragments.rs`, `top_level_templates.rs` and `ast/module_ast/finalization/finalizer.rs` | **Important:** Refactor documentation handling as described below, while continuing to use the one TIR folding path. |
| `ast/statements/body_dispatch.rs` | Runtime entry parser sees only a plain `TEMPLATE_HEAD` after Stage 2 strips `~`; the existing `PushStartRuntimeFragment` remains the only emitted runtime-fragment node. Reject an explicitly runtime, non-output comment using an authored template diagnostic. |
| `headers/types.rs`, `parse_file_headers.rs`, `arena/capacity.rs` and downstream assembly | Keep meaningful existing counters and metadata. Inspect template-capacity estimates for redundant counting of const headers/fragments, now more common. Change a heuristic only with an explained sizing rationale or evidence. |

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

| Contract | Regression evidence |
|---|---|
| Marker/boundaries | `~[...]`, `~ [...]`, `~\n[...]`, CRLF, leading indent, EOF, adjacent entries, `;~[...]` and nested template brackets. Removed `#[...]` must fail without a migration-only diagnostic. |
| Entry roles | Active and synthetic entries work. Helper/facade/config/export placements are rejected correctly. Ordinary assigned/nested templates and mutable bindings/writes are unchanged. |
| Const requirement | Bare direct literal/imported constant, static `if`/`loop`, structural resource and named `$in` fold. Bare direct runtime interpolation diagnoses instead of creating runtime HIR. |
| Runtime guarantee | `~[...]` with dynamic data and entirely constant content both produce exactly one runtime `PushStartRuntimeFragment` and slot, with no const body record. |
| Ordering and failure | Interleave static body fragments, builder slots, two runtime fragments and other entry statements. Assert source order, destination content, resources and successful-only runtime publication. A failing entry retains static output and publishes no staged runtime output. |
| Documentation | `[$doc]` contributes ordered metadata with no HTML/runtime slot, including Markdown/site-root text and an invalid structural-resource observation. `[$note]`/`[$todo]` discard without output. `~[comment]` diagnoses. Keep nonempty versus empty body-fragment semantics distinct. |
| Targets/builders | Compare appropriate JS and HTML-Wasm output, resource linking and metadata-only/static-only entries. Respect unsupported backend capabilities. |

During migration, change real `#[...]` entry fragments to `[...]` and bare *runtime* entry fragments to `~[...]`. Update documentation site entries, CLI scaffold, package examples, executable benchmarks, tests, expected diagnostics and integration manifests as needed. Change only actual entry syntax, not ordinary expression templates or quoted Rust attributes. Migrate and run focused tests together rather than expecting the pre-migration suite to pass mid-cutover.

Update canonical unsuffixed template/constant/entry references, paired Basic pages, the cheatsheet and affected compiler/build architecture prose. Replace outdated examples and stale comments. Check syntax highlighting and developer tooling only for actual old-syntax assumptions. Do not edit generated `docs/_release/**` manually. Update progress and `index.md`/audit-log staleness only when the repository rules require it.

### Phase 3 - Integrated validation and Slice review

Run targeted parser/AST tests and selected runtime integration cases after each coherent slice. At the integrated checkpoint run `just validate` plus affected checks not included in the routine gate. Before merging into main, run `just validate-full` against the final integrated tree as required by `validation.mtf`. Build the docs site with `moth check docs --terse` and `moth build docs --release` (or documented Cargo equivalents). Generate and build a CLI starter project.

Audit remaining old entry spellings and runtime bare fragments after source migration. Review by semantic context, not raw grep count.

End with the required Slice review of semantic correctness, explicit stage ownership, no duplicated TIR/documentation paths, deletion of old hash/comment glue, typed diagnostics, meaningful test coverage and documentation consistency. Check that static-heavy pages do not incur newly inflated arena allocations or runtime artifact requirements. Benchmark or profile only when the affected path has a concrete performance hypothesis.

Completion means the new spelling is the sole accepted direct top-level output syntax and all supported examples and backend contracts are validated. Delete this plan **and** its roadmap entry in the same final implementation commit.
