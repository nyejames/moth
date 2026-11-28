# AUD-0028: Directive implementation redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `frontend.directive_implementation` - directive specifications, recognition and source/template application handoffs.
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: `f6e65e4d6d84faf03ee19f3efa65c6726777823b` on `docs/const-foundation-profile-independent-semantics`. No check runs returned for that revision. Source inspection only; local compiler/test execution unavailable. The audit branch differs from the active branch only in audit records.

## What was inspected

- `src/compiler_frontend/style_directives/{mod,specs,builtins,compatibility,source}.rs`: registry creation and queries, source kinds and operands, template compatibility masks, handler effects, source attachment capture/folding and statement-run application. The ordered built-in registry is the recognition owner.
- `src/compiler_frontend/tokenizer/{lexer,stream,text_modes}.rs`: directive recognition, template frame body-mode selection, bracket balancing and discarded/literal body tokenization. Inspected the relevant lexer tests.
- `src/compiler_frontend/ast/templates/template_head_parser/{mod,core_directives,handler_directives,directive_args}.rs`: head compatibility and its per-tag conflict witnesses, source-versus-template context rejection, core/handler dispatch, operand parsing and formatter effects. Inspected the corresponding template-body consumer and formatter factories for markdown, HTML, CSS, escaped HTML and code, plus focused head/body tests. Raw has a separate core implementation.
- `src/compiler_frontend/headers/{file_parser,types,qualified_references}.rs`: source modifier recognition, declaration/dependency eligibility, route operand capture/consumption, retained metadata, path remapping and source identity rebinding; `headers/tests/source_directives_tests.rs` including the route/documentation stack regression. Compared header and AST statement recognition. Searched caller and constructor references across the repository; inspected ordinary, dependency and synthetic header construction.
- Context only: source annotation downstream attachment, config and public export metadata, numeric constraint kind grouping and template formatter bodies. This is not an audit of const evaluation, generic materialisation, the TIR evaluator, constraint proof semantics, or build-system publication.

## Authorities read

- `AGENTS.md`, `docs/roadmap/audit-guide.md`, `docs/roadmap/audit-kinds/{correctness,redundancy}.md`, `docs/roadmap/audit-log.md`, and `docs/roadmap/audits/README.md`.
- `docs/docs/directives/directives.mtf`, `docs/docs/templates/template-directives.mtf`, language overview, progress matrix and relevant compiler architecture/ownership sections of `docs/compiler-design-overview.md`.
- Relevant style, testing, validation and builder-capability guidance: frontend owns the closed directive set; no builder registration of directives; later stages consume owned facts rather than reinterpreting earlier syntax. Preserve diagnostic names, exact source spans and deterministic order.

## Existing findings and active plans checked

- Historical source-directive Correctness audit AUD-0018 and its closeout concern prior source parsing/context failures. Its old coverage row remains intact. The candidates here concern registry representation, template compatibility/handler state and a leftover route remapping path, not those prior parser failures.
- Read the bug queue, open findings and relevant completed audit records. The numeric-proof redundancy candidates and existing template-helper/constraint correctness entries have different owners.
- Compared the active const-foundation branch against main; the registry, lexer/template dispatch and header files implicated below are unchanged in the returned diff. The active plan changes const evaluation and constraint semantics; those downstream paths are excluded. No deferred directive or proposed extension is treated as a defect or reason to retain unused configuration.

## Findings

### AUD-0028-F01: Directive class and source kind are encoded redundantly

- State: `closed`
- Kind: `Redundancy`

#### Evidence

`style_directives/specs.rs` defines both `StyleDirectiveKind` and `StyleDirectiveContexts`. The latter is a bit-mask-shaped value with template-head and source-modifier bits. Every core/handler spec in `builtins.rs` passes `TEMPLATE_HEAD`; `StyleDirectiveSpec::source_modifier` sets `SOURCE_MODIFIER` itself. The built-in registry is immutable and no supported builder registers directives. Consequently the separate context argument allows a contradictory spec without representing another supported class.

The source modifier also carries `SourceModifierTarget`, whose `directive_kind()` translates the four non-constraint variants directly into `SourceDirectiveKind::{Config,Export,Doc,Todo}`, and returns the embedded kind for `SourceConstraint`. Actual header and statement target checks dispatch on the resulting kind; the source-constraint grouping already lives on `SourceDirectiveKind::is_source_constraint`. The target enum permits `SourceConstraint(Doc)` even though that is not a valid constraint. In `source.rs`, `source_modifier_contract` performs the kind/context projection; the statement run repeats the projection rather than using the same owner. `headers/file_parser.rs` and `headers/qualified_references.rs` already use the shared projection.

#### Counter-explanation tested

A multi-context directive or independently selected target policy would justify separate facts. Neither exists in the closed built-in construction paths; the actual target validators use the semantic kind, not an independent target policy. Registry context predicates are important for diagnostics, but do not require independently stored flags. A supported registration path whose context cannot be derived from the kind, or a real target-policy consumer beyond kind projection, would disprove the proposed deletion. Separate source/header and statement eligibility decisions remain meaningful.

#### Violated contract or cost

Multiple representations of one closed recognition fact create impossible states and coordinated maintenance. Adding a source kind must currently update the target-to-kind translation as well as actual eligibility checks. This conflicts with single ownership and deletion of unnecessary abstraction; no performance improvement is claimed.

#### Root owner

`style_directives/specs.rs`, `builtins.rs` and `source.rs`, with registry, header and statement recognition consumers.

#### Suggested correction

Derive occurrence eligibility directly from the directive class. Carry the source semantic kind and operand contract directly, rather than translating a second enum into the kind. Use the shared source contract projection for all recognition consumers. Remove superseded representations and constructor arguments, rather than retain compatibility wrappers. Keep explicit authored target checks at their existing owners.

#### Fix scope and preserved invariants

Preserve the closed names, registry order, separate source/template contexts, operand cardinality and types, unknown-name versus wrong-context ordering, export route handling, target/file-role legality, duplicate annotation ordering and distinct lexical constraint kinds. Do not restructure downstream constraint verification or add a directive registration API.

#### Required validation

Run the focused directive registry, source-directive header and statement tests, lexer directive recognition tests and template head context/diagnostic tests. Check the supported-name suggestion ordering and known-name wrong-context diagnostics in both directions. Validate legal and illegal targets separately, including the three constraint kinds, without changing the accepted source surface. Broader branch gate per validation guide.

#### Linked findings

F03 concerns the separate handler-specific representation; neither requires accepting the other.

### AUD-0028-F02: Raw and code compatibility bits have no distinct consumer

- State: `closed`
- Kind: `Redundancy`

#### Evidence

`style_directives/compatibility.rs` defines `RAW_DIRECTIVE` and `CODE_DIRECTIVE`, includes both in its compile-time slot-bound assertion and reserves a slot per bit. `builtins.rs` places them in `$raw` and `$code` presence masks alongside `MEANINGFUL_ITEM` and `FORMATTER_DIRECTIVE`. Repository reference searches for both constants lead only to these definitions/assertions and the producer masks. No `required_absent_tags`, `blocks_future_tags` or dedicated presence query consumes either distinct bit. Both specs block future `FORMATTER_DIRECTIVE` with `MultipleFormatters`; that is the same rule as the other formatter directives. The head state records per-tag presence/witness slots, so the unused tags are populated even though no conflict can be decided by them. Conflict attribution retains the actual directive name and span separately.

#### Counter-explanation tested

`$raw` and `$code` have different semantics and lexical modes, but those are owned by core/handler dispatch and body-mode selection, not these compatibility bits. Their names remain distinguishable in the conflict witness. A rejection rule, presence query, external encoding or diagnostic attribution that uses the individual bits would disprove the deletion; none appeared in the inspected construction/consumer paths.

#### Violated contract or cost

Unconsumed metadata, extra state slots and custom mask construction for an otherwise shared compatibility rule. This is structural redundancy, not a measured allocation or speed claim.

#### Root owner

`style_directives/{compatibility,builtins}.rs` and the template-head compatibility state.

#### Suggested correction

Delete both unused bits and their reserved slots. Express the two formatter compatibility specs using the existing `blocks_same(FORMATTER_DIRECTIVE, MultipleFormatters)` rule; the equivalent `$md` mask can use it too. Keep the slot-bound invariant accurate. No extra helper or parallel policy is needed.

#### Fix scope and preserved invariants

Keep the one-formatter rule, first conflicting item and exact diagnostic reason/order. Preserve all formatter behavior and `$supplied`/`$slot`/`$note` compatibility. Do not change lexical body-mode selection as part of this purely structural deletion.

#### Required validation

Run focused template-head conflict tests, including `$raw`/`$code` and every other formatter in both orders, and legal formatter plus `$fresh` or `$supplied` combinations. Check that earlier/incoming names and spans remain correct, not merely that some error occurs. Preserve the separate balanced-body lexer tests.

#### Linked findings

The correctness entry about neutral directives overwriting lexical body mode is a different root cause in tokenizer state, not evidence for keeping the unused compatibility bits.

### AUD-0028-F03: Handler specs allow absent formatter and style identity

- State: `closed`
- Kind: `Redundancy`

#### Evidence

`StyleDirectiveHandlerSpec::formatter_factory` is an `Option<FormatterFactory>` and `StyleDirectiveEffects::style_id` an `Option<&'static str>` in `style_directives/specs.rs`. All five `StyleDirectiveKind::Handler` registrations (`md`, `html`, `css`, `escape_html`, `code`) in `builtins.rs` specify both a factory and a style identity. The handler constructor has no other supported producer. `handler_directives.rs::apply_handler_style_directive` branches on whether the factory exists; the effects application similarly branches on style identity. No registered handler is style-only or has a deliberately absent identity. The `$raw` formatter is core-owned, not a handler spec with no factory. All five handler entries have formatter exclusivity.

#### Counter-explanation tested

An independently composable style-only handler would need optional formatter/identity effects. The closed registry contains none and builders cannot register one. `style_id` is deliberately distinct from `Formatter.id` and must not be merged with it. Optional string operands for CSS/code and the factory's real failure path are also necessary; their presence does not justify a missing factory. A reachable handler with no factory or no semantic label would disprove the proposal.

#### Violated contract or cost

Optional fields represent unsupported handler states and require conditional dispatch in the sole consumer. The formatter class's mandatory effects could be expressed directly, reducing maintenance and making missing required fields impossible.

#### Root owner

`style_directives/specs.rs`, `builtins.rs`, `ast/templates/template_head_parser/handler_directives.rs`.

#### Suggested correction

Make the factory and semantic style identity required for the handler class. Delete the unreachable absence branches and simplify the constructor/effects shape to match current use. Retain an explicit child-template suppression effect where needed and the no-operand versus optional-string contract. Avoid building a new general handler framework.

#### Fix scope and preserved invariants

Preserve raw's core dispatch; all five handler style labels, formatter IDs and outputs; CSS/code optional operands and their error messages; no-operand rejection; formatter exclusivity and child-template suppression. A missing/failed operand remains a user diagnostic at the same span, not an infrastructure failure.

#### Required validation

Run handler/directive and template formatting tests for all five handlers, legal optional operands and invalid operand/formatter combinations. Compare the emitted template/formatter behavior and errors to the baseline, including code's literal bracket behavior. Retain existing output-level tests.

#### Linked findings

F01 concerns recognition, not handler dispatch. The lexical body-mode correctness finding requires independent investigation.

### AUD-0028-F04: Header remapping retains the removed export annotation route

- State: `closed`
- Kind: `Redundancy`

#### Evidence

`headers/file_parser.rs::handle_symbol_item` resolves export mode, checks eligibility and removes the `Export` annotation from `source_directives` before publishing the header. The `HeaderExportMode::Public` route is the published owner. Direct-selection dependency clauses carry export mode; synthetic header constructors initialise empty directive vectors. `headers/tests/source_directives_tests.rs::doc_and_route_stack_on_one_target` asserts that the stacked declaration retains only `Doc` in `source_directives` while the route is in export mode.

`headers/types.rs::Header::remap_path_ids` nevertheless remaps `self.export_mode` and then loops through every retained `source_directives` entry to call `annotation.operand.remap_path_ids(remap)`. In `style_directives/source.rs`, that operand method acts only on `PublicRoute`, with a comment explaining the former second copy. The only authored `PublicRoute` producer is the pending source run and its route is consumed by the export mode conversion. No retained ordinary header annotation can reach the remapping branch after export removal. The route case in operand source identity rebinding likewise deserves review, although expression-range rebinding remains necessary.

#### Counter-explanation tested

During pending attachment the operand must retain a route until export mode is established. Removing that parse-time variant would lose a real fact. But the path remapping loop in question runs on a published header, not pending annotations. A current production constructor retaining a `PublicRoute` operand in published `source_directives` would disprove the dead branch; the inspected producers and route/documentation test establish the opposite invariant.

#### Violated contract or cost

Cleanup from the earlier single-route-owner change stopped short of its consumer. A dead remapping loop and helper still imply that two route copies are live, obscure the invariant and add maintenance/traversal. No measured performance claim.

#### Root owner

`headers/types.rs` and `style_directives/source.rs`, with `headers/file_parser.rs` as the producer.

#### Suggested correction

Remove the stale path-remapping loop and operand helper if the final caller inventory confirms the published-header invariant. Keep the actual export-mode route remap and the pending operand representation. Review the stale route-only source-rebind arm at the same boundary, retaining expression-range rebinding; prefer the smallest deletion over a new wrapper/type hierarchy.

#### Fix scope and preserved invariants

Preserve header export mode and routes, dependency-clause export routes, source directive names/spans and expression operand ranges, path identity across fork merge, source rebinding, and existing diagnostics and ordering. Do not change route grammar, export policy or config publication.

#### Required validation

Run source-directive header tests (especially `doc_and_route_stack_on_one_target`), retained path remap/source identity tests and routed public-export/dependency integration cases. Check that both a routed export and another retained operand survive source rebinding and that the route exists only in export mode.

#### Linked findings

None. This is independent of recognition simplification in F01.

## Resolution (2026-11-24)

- **F01 accepted and closed.** The directive kind determines occurrence
  eligibility and carries the source kind and operand contract directly.
- **F02 accepted and closed.** All formatters share the same exclusivity tag.
  Conflict diagnostics retain both authored names and spans.
- **F03 accepted and closed.** Every handler supplies its formatter and a
  separate semantic style label. Child-template suppression remains explicit.
- **F04 accepted and closed.** Published export routes remap and rebind through
  export mode alone. Retained expression operands still rebind to their source.

Neutral directives also leave the frame's lexical body mode unchanged. Tests
cover code and CSS with `$fresh` or `$supplied` in both head orders and a
genuinely non-identity source rebind. Independent review checked the affected
boundaries. Focused directive, template, lexer and header tests and the
`namespaced_public_exports` and `namespaced_reexport_origins` HTML integration
cases passed.

## Checked and clean

- Header and statement attachment share operand capture/folding while validating their different eligible targets locally. Sharing recognition does not justify merging those stage-specific decisions. Source modifier runs and template-head comma-separated items have different grammars and are not redundant parsers.
- The structural path scan excludes directive-owned route operands even when later attachment rejects the directive. Dropping that scan without an equivalent producer-owned fact could misclassify a route as a dependency or constant reference; no deletion proposed.
- `$slot` and `$supplied` already share optional target parsing where their grammar agrees. `$in` requires a named target; its different validation is not evidence of duplicate policy. Per-directive duplicate rejection, formatter exclusivity, slot/comment exclusivity and the guard's body requirement are distinct contracts.
- Actual formatter effects, body-mode choices and named conflict witnesses are meaningful even when F02's unused tag bits are deleted. The semantic style ID and formatter ID are distinct. Lexical constraint span slots are necessary to label the separately authored guarantees.

## Limitations

This is a partial source-trace audit at the named revision, not an exhaustive directive or template semantics audit. No compiler executable, test suite, documentation release build or full gate was run. Check-run query returned no runs for the audited revision. Excluded: exhaustive operand expression semantics, const folding/TIR evaluation and generic materialisation, downstream config/export publication, all numeric constraint proof/lowering, complete formatter implementation/output semantics, and all template execution/lifetime rules. These exclusions avoid overlap with ongoing const-foundation work. The correctness finding is recorded separately in `docs/bug-reports.md`; these candidates are not accepted fixes.
