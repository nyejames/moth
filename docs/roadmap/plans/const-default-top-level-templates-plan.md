# Const-default entry templates and source documentation directives

## Status and purpose

- Status: Queued for compiler implementation; `$export` prerequisite merged
- Current slice: Branch merged with main; canonical language, compiler and builder documentation updated, no compiler changes
- Next: Reinspect current `$export` header/AST ownership and begin Phase 0 inventory and implementation when activated

Deliver two related cleanups in one breaking change:

1. In executable `@entry.moth`, a bare direct `[...]` is const-required. Explicit `~[...]` is a runtime fragment.
2. Make `$doc` and `$todo` ordinary following-construct source directives with compile-time String operands. Keep `$note` as the sole template-comment directive.

Remove superseded template-comment extraction and `#[...]` handling. No syntax aliases, migration diagnostics or unfinished parallel pathways.

## Accepted language surface

### Entry fragments

```moth
[$in title:Moth]
[$md:# Welcome]

name = "Moth"
~[: Hello, [name].]
```

- Bare direct `[...]` in an executable `@entry.moth` must fold. Its output is compile-time page or builder-root-slot metadata, never a runtime template or HIR fragment.
- `~[...]` is one runtime body fragment, even when fully foldable. It uses the existing implicit entry start and successful-publication contract. Const folding of subexpressions remains legal.
- Recognise `~` with an immediately following `TEMPLATE_HEAD` at a direct statement boundary. Allow horizontal whitespace (`~ [...]`) but not a newline. Consume the prefix without changing the authored template token range. This is not an expression operator or reactivity marker.
- Remove direct `#[...]` without alias or migration-specific diagnostic. Ordinary `#=` bindings retain their meaning.
- Direct `[$in name: ...]` still folds to an active builder-registered slot. Runtime `~[$in name: ...]` remains unsupported. Nested, assigned and aliased inserts remain template-composition helpers.
- Ordinary template expressions, nested templates, values and standalone `.mtf` content keep their current const/runtime rules. Only direct entry-root templates change default phase. Helper/config/API-only roots and imported/dormant entries preserve their restrictions.
- `[$note: ...]` is valid as a comment template, including directly in an entry. It discards its balanced body, produces no fragment/slot and cannot be used as an explicit runtime fragment (`~[$note: ...]`). Distinguish absent output from an empty String fragment.

### Source `$doc` and `$todo`

```moth
$doc [: Parses a source file into tokens.]
$todo [: Improve Unicode escape diagnostics.]
parse_source |source String|:
    $todo [: Implement the remaining escape cases.]
    assert(false, "Not implemented")
;
```

- `$doc value` attaches to the next **declaration** (including eligible local declarations). `$todo value` attaches to the next **declaration or executable statement**, including a complete branch, loop or return. They are source directives in ordinary code, not free-standing statements or templates.
- The operand is exactly one compile-time `String` expression. A template is the common form (`$doc [: text]`), but constant String values also work. The source directive does not add implicit Markdown formatting: use `$doc [$md: ...]` to request Markdown. Evaluate through existing constant-expression/template infrastructure; preserve authored spans and diagnose runtime values or unsuitable structural resource text using existing typed lanes.
- `$doc` records documentation text **with its target declaration identity**, not an unattached page fragment. Keep this in compiler-owned, non-HIR module metadata for future consumers. `$todo` is compile-time validation-only initially: it has no runtime effect, warning or builder emission. Do not add unused persistent TODO metadata until a consumer exists.
- `$doc` and `$todo` are invalid in template heads. `$note` is template-head-only and retains balanced-body discard behavior. Remove `Doc`/`Todo` from `TemplateType::Comment` and related head compatibility and tokenizer modes. Keep `Note`.
- Source directives cannot annotate template text, expression subtrees, arguments, parameters or arbitrary tokens. A construct is either an accepted declaration/statement target or an authored wrong-target diagnostic, never a search for the next *eventually eligible* construct.

### Shared source-directive attachment and stacking

**Every consecutive source directive applies to the same next construct independently.** This is syntax attachment, not an implicit instruction to reinterpret other directives.

```moth
$doc [$md: Calculates a value.] $todo [: Review edge cases.]
calculate |value I32| -> I32:
    return value + 1
;

$fast_math $infallible
wrap_value |value I32| -> I32:
    return value + 1
;
```

- Source directives may share a physical line with horizontal whitespace between them, or occupy consecutive lines. No commas. The **target construct begins on the next source line**, never after the final directive on the same line. This preserves a simple end boundary for ordinary expression operands.
- A following `STYLE_DIRECTIVE` at the **outer source-operand parse depth** ends the current directive's operand and starts the next annotation. No source directive is a legal operand of another source directive. Parse balanced expressions/templates normally: `$md` inside `$doc [$md: ...]` belongs to the nested template and cannot terminate the outer `$doc`. An operand may span lines while balanced. Avoid a separate token scanner; use the existing expression/token cursor boundaries.
- Comment-only lines can intervene between directives and their target. An empty physical line ends attachment and yields a dangling-directive diagnostic. Keep the existing LF/CRLF and blank-line facts.
- Consume the **single contiguous directive run once** and bind the resulting ordered annotations to the **immediately following** declaration or statement. Do not have each directive rescan past the others. No attachment crosses a declaration, statement, scope close, EOF or empty line.
- Validate each directive's context, operand and target independently. Apply its semantic interpretation to the same completed target; perform mode selection before proof/checking, independent of source order. Reversing the order of different directives cannot alter their meaning. A directive cannot weaken another's requirements or silently override its effect.
- No universal pairwise compatibility matrix. A combination fails only if at least one directive's **own** target, operand or semantic requirements are violated. Two constraints that happen to be vacuously satisfied may coexist.
- **No duplicate directive names within one attachment run, without exceptions.** `$doc $doc`, `$todo $todo`, `$fast_math $fast_math` and repeated `$config` or `$export` all diagnose, even with identical operands or when semantically harmless. A later annotation may repeat on a different target. Report the second directive with the first one's authored span as a related site. One `$doc` or `$todo` operand must contain all intended text, including newlines; do not concatenate repeated annotations.
- Existing `$config` target/operand semantics and the merged `$export` directive's target and visibility semantics remain authoritative. Extend their attachment plumbing to accept a run and enforce the common no-duplicates rule, without broadening eligible targets. Source and template-head placement errors remain typed and specific.

Example: `$fast_math` chooses the numeric mode and `$infallible` checks actual escape/trap behavior. Both may succeed even with arithmetic (e.g. valid wrapping integer addition). `$safe_math` checks for executed fast arithmetic, rather than rejecting a syntactically present `$fast_math` marker. **This plan does not implement those future constraints**, but should leave this common attachment model usable by them.

## Ownership, reuse and deletion

Stage 2 classifies top-level declarations and direct entry fragments; AST statement parsing owns statements inside executable bodies. Each owns its attachment *at that existing parse boundary*, using the same registered directive contracts and source-trivia rules. Do not create a second source scanner, parallel lexer mode, speculative annotation framework or separate directive compatibility pass.

- **Directive registry:** `src/compiler_frontend/style_directives/{specs,builtins,registry}.rs` owns names, contexts, operand and target contracts. `$doc`/`$todo` become source-only; `$note` remains template-only. Reuse `STYLE_DIRECTIVE` tokenisation and context-aware diagnostics.
- **Top-level attachment:** `headers/file_parser.rs` currently holds one `PendingSourceModifier` and `source_modifier_target_for`/`header_accepts_source_modifier`. Adapt the existing owner to collect one run and retain the relevant typed syntax/operand source range with each annotation, rather than creating independent lookahead loops. After `$export` merges, reuse its actual attachment representation and eliminate any obsolete single-modifier forwarding paths. Apply top-level target validation once the header kind is known.
- **Statement attachment:** `ast/statements/body_dispatch.rs` currently rejects `STYLE_DIRECTIVE` in statement position. Allow `$todo`, plus `$doc` only before a declaration statement, at ordinary statement boundaries. Parse/attach the run to the next actual statement without skipping ineligible statements or crossing scope boundaries. Keep no directives in executable HIR. Root-runtime statement directives must survive header preparation into this owner.
- **Operand validation:** Use the existing const-expression and template parser/TIR fold, not a direct lexer-level template scanner or dedicated documentation formatter. At the source operand's outer parse depth, the next directive token is a terminator; nested template-head directives are ordinary nested syntax. Preserve positions, associations, value type/constness diagnostics and balanced multiline operands. The annotated target must still start on a later line.
- **Doc metadata:** `ast/templates/doc_fragments.rs`, `ast/module_ast/finalization/finalizer.rs`, `hir/hir_builder/metadata.rs` and `module_metadata.rs` currently own unattached `AstDocFragment`/`ModuleDocFragment` and extraction by walking `PushStartRuntimeFragment` nodes. Replace that extraction with declaration-attached folded docs. Reuse the existing compiler metadata publication boundary, give doc records a stable target identity suitable for cross-module use and remove orphaned AST scans, shape-only enums and unused helpers. No builder-specific interpretation yet.
- **Comment removal:** Delete `Doc`/`Todo` template-head classification and old documentation-comment folding. Keep the template-local `Note` discard behavior and ensure a direct top-level note yields **no output** rather than an empty body fragment. Avoid making note cancellation a special case in the HTML builder.
- **Template classification:** `headers/top_level_classifier.rs` routes bare direct `TEMPLATE_HEAD` to const header construction; `MUTABLE` followed by `TEMPLATE_HEAD` to existing runtime capture. `headers/const_fragments.rs` and `start_capture.rs` retain the balanced source ranges, synthetic const declarations and runtime insertion indexes. Consume only the `~` marker before capturing the runtime template range.
- **Delete old hash-template path:** Remove `headers/hash_items.rs`, `HeaderFileItem::Hash` and their obsolete registration/requests. Keep `TokenTag::HASH`, valid constant declaration syntax and ordinary diagnostic handling for stray `#`.
- **Existing downstream:** Keep `TopLevelConstFragment`, `PushStartRuntimeFragment`, TIR, builder slots, typed metadata, entry activation and JS/HTML-Wasm assembly. No additional fragment registry, HIR node or build-side syntax recognition. Review `arena/capacity.rs` only for evidence-based redundant const-template counting.

## Implementation sequence

### Phase 0 - Refresh and audit

1. The `feature/export-directive` work has merged and this PR branch now includes main. Read `AGENTS.md`, the current style/test/validation guides, `docs/developer-docs/language/overview.mtf` and its canonical unsuffixed directive/template/constant/entry references, plus relevant compiler/build design handoffs. Recheck merged `$export` registration, attachment and metadata ownership before writing compiler code. Refresh again only if main advances.
2. Inventory `[$doc`, `[$todo`, `[$note`, standalone `$doc`/`$todo`, direct `#[`, unmarked runtime entry fragments and consumer tests across source, test cases, docs, examples, scaffolds, packages and benchmarks. Classify matches by actual syntax context. Rust attributes, quoted snippets and intentional invalid cases must not be blindly rewritten.
3. Select existing test owners and obtain focused baseline evidence. Preserve current docs/entry roles and builder capabilities.

### Phase 1 - Directive cutover and deletion

1. Change registry contexts and remove `Doc`/`Todo` from template-head parser/types and their specialized lexer behavior. Retain `Note` and typed wrong-context diagnostics.
2. Implement one contiguous source-directive-run attachment rule, reusing the merged `$export`/`$config` owner. Permit same-line directive stacking and recognize the next outer source directive as an operand terminator, but keep the construct on the following line. Validate unique directive names once per run with two-span diagnostics. Attach top-level declarations in header preparation and statements/local declarations at their AST parse owner. Preserve blank-line, comment, LF/CRLF, balanced operand and scope boundaries.
3. Validate source directive String operands through existing const semantics, including balanced template operands and imported constants. Retain precise authored argument/target diagnostic spans. Attach and publish `$doc` metadata to its declaration identity, validate/drop `$todo` with no runtime work.
4. Remove the old runtime-template doc extraction, transient comment-fragment intermediates and redundant conversions. Ensure direct `[$note]` disappears without output. No new generic attributes framework.

### Phase 2 - Entry template cutover

1. Change direct template classifier and header dispatch. Use existing `create_top_level_const_template` for bare direct templates and existing balanced runtime capture after consuming `~`. Do not materialize a false `#` token or leak `~` into AST.
2. Delete obsolete hash-template handlers. Confirm runtime-fragment counters and static insertion indexes remain correct with statements and source directives interleaved.
3. Preserve builder-root `$in` validation, structural resources/site-root facts, JS/HTML-Wasm ordering, successful-only runtime publication and static-only/metadata-only output.

### Phase 3 - Focused regressions and migration

Extend existing header/AST directive tests (including `parse_file_headers_entry_runtime_tests.rs`, module-role and template tests) and only a small set of meaningful integration cases.

- **Attachment:** `$doc` then `$todo` on separate lines and the same line, in reverse order, same-line zero-operand markers, comments between annotations, blank lines, EOF, scope close, adjacent targets, tabs, LF/CRLF and balanced multiline operands. Reject target-on-directive-line. Prove nested `$md` inside `$doc [$md: ...]` does not end the source operand. No commas. Order independence must be semantic, not just lexical.
- **Duplicates:** any repeated source directive name on one target diagnoses, including repeated `$doc`, `$todo`, `$config`, `$export` and future parameterless constraint markers. Test both same-line and separate-line duplicates, identical versus different operands and two-span diagnostics. A directive on a later independent target is valid.
- **Targets:** docs on eligible declarations (including locals), TODOs on declarations/expressions as statements/returns/branches/loops and top-level entry runtime. Reject docs on non-declaration statements, annotations in template heads, notes as source directives, ineligible helper/config/export positions and dangling runs with typed sites.
- **Values:** constant String, template String, explicitly formatted Markdown, constant imports, dynamic interpolation or runtime-only String rejection, wrong type and structural resource/text constraints. Verify docs carry the **correct target**, not only matching text, and have no body/runtime fragment.
- **Template output:** direct note produces no output; ordinary empty template remains an empty output fragment. Nested note remains discarded. Old `[$doc]`/`[$todo]` template forms fail with proper context diagnostics.
- **Entry syntax:** `~[...]`, `~ [...]`, `~\n[...]`, CRLF, EOF, nesting, other mutable syntax and removed `#[...]`. Bare runtime interpolation must fail const validation. Marked constant output still reserves a runtime slot.
- **Build and failure:** interleave static fragments, two runtime fragments, entry statements, source directives and named builder slots. Check authored output order, resources, metadata deduplication and failure publication. Exercise supported JS and HTML-Wasm capabilities, static-only entries and CLI scaffold.

Migrate real old entry `#[...]` to `[...]`, bare runtime entry fragments to `~[...]` and template-head `[$doc]`/`[$todo]` to declaration/statement-attached source directives where the intent is clear. Where an old standalone doc fragment has **no evident target**, determine its intended owner rather than silently attaching it to the next unrelated construct. Replace template `[$todo]` comments inside template content with `[$note]`. Update docs-site source, sample generators, packages, benchmarks and expected assertions with the owning implementation slices.

The final-design canonical references, paired Basic pages, cheatsheet and compiler/build architecture descriptions were updated proactively on this planning branch. During implementation reconcile every changed behavior, new diagnostic and actual target with those authorities rather than recreating parallel documentation. Remove old formatter exceptions and obsolete examples where any remain. Regenerate documentation outputs rather than hand-editing `docs/_release`. Reconcile the deferred constraints plan's blanket direct `$safe_math`/`$fast_math` incompatibility with semantic constraint validation. Check progress, index and audit staleness rules only where warranted.

### Phase 4 - Validation and Slice review

Run focused affected unit/integration tests throughout. At integration run `just validate` plus uncovered relevant selections. Before merge run `just validate-full`, `moth check docs --terse` and `moth build docs --release` (or documented equivalents), plus generated CLI starter build. Review stale syntax by semantic context, not raw grep counts.

Final Slice review must verify: a single owner for each attachment/metadata fact, no per-directive rescans or order dependence, depth-aware same-line operand termination, common unique-name checks with correct spans, independent semantic diagnostics, removal of old doc/stripping and `hash_items` pathways, unchanged HIR/fragment/build contracts, no unnecessary allocations and no duplicative tests. If performance policy changed, benchmark against focused evidence.

The implementation is complete only when old template-head documentation forms and old direct fragment spelling are gone, documentation is target-attached and supported projects build. Delete this plan **and** its roadmap entry in the final implementation commit.
