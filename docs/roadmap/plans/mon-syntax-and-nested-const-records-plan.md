# MON syntax and nested const records

## Status

- Status: Phase 1 argument parsing consolidation accepted; implementation active.
- Current slice: Phase 2A - cut over syntax and nested const values.
- Blockers: none. The source/token-layout Phase 3 closeout and maintainer-managed post-Phase-3
  cleanup are accepted on main.
- Next action: integrate parenthesized grouping and recursive const-record construction through the
  shared argument owner.

## Purpose and authority

Use parenthesised MON syntax for supplied arguments and value construction. Support inline nested anonymous const records through the existing call/constructor argument owner. Keep pipe-delimited parameters and nominal type declarations on their declaration owners.

This is a breaking cutover, with deletion and simplification of displaced parser paths. It implements compile-time records and shared parsing only. Runtime anonymous records and the MON serialisation format/compiler-native builder remain separate work.

The permanent contracts are published:

- `docs/src/docs/language-overview/mon-syntax.mtf` owns shared notation, grouping, entries, layout and context distinctions.
- `docs/src/docs/constants/const-records.mtf` owns compile-time folding, nested values, projection and whole-record restrictions.
- `docs/src/docs/structs/anonymous-records.mtf` owns the accepted deferred runtime surface, including recursive local composition and hidden nominal identity.
- `docs/src/docs/functions/calls-and-access.mtf`, `parameters-and-defaults.mtf` and `docs/src/docs/structs/construction-and-fields.mtf` retain their receiving contracts.
- `docs/src/docs/design-scope/deferred-and-outside-scope.mtf` separates accepted MON syntax/runtime work from the format contract still to define.

Those references own source semantics. This plan owns implementation, deletion, remaining publication work and acceptance. Preserve explicit nominal construction. Sharing syntax does not make const records, runtime hidden structs and call argument slots one semantic representation.

## Activation gate

Activation is released. Source-owned fixed tokens, retained syntax ranges, the complete Phase 3 exit
and the maintainer-managed post-Phase-3 cleanup are accepted on main.

At Phase 0, record the full implementation SHA, worktree status and baseline results in local working
notes. Reload the published MON contracts and refresh all implementation locators before source
edits. Use a dedicated implementation worktree and leave any unrelated worktree unchanged.

Complete MON before Wiring, native result slots/Core constant evaluation and explicit data-layout Phase 4 reactivation. General directives consume the delivered shared parser later. Runtime records keep their numeric-semantics prerequisite. Existing package pauses remain in force. The roadmap owns serial order.

Use the delivered token/cursor and diagnostic APIs. Preserve exact spans and fixed layouts. This plan neither resumes later diagnostic-layout phases nor restores adapters deleted by prior work.

## Documentation already prepared

The following work is done, not a new implementation phase to repeat:

- [x] Publish canonical and Basic MON syntax references under `language-overview/`.
- [x] Publish canonical and Basic runtime anonymous-record references under `structs/`.
- [x] Rewrite both const-record references for named-only parentheses and inline nesting.
- [x] Clarify shared call syntax, declaration/default distinctions and explicit nested nominal construction.
- [x] Route the references through the compiler-facing language index and the Language basics and Structs pages.
- [x] Publish the MON syntax/runtime/format distinction in Design Scope.
- [x] Make the runtime implementation work item consume the published grammar and recursive local-value contract.

New syntax examples are documentation code blocks, not executable compiler fixtures. The references explicitly label unimplemented support. Live configs, scaffolds, Rust comments, tests, benchmarks and implementation-status cells are unchanged by this preparation.

The documentation release-build gate remains required. Preparation checked text structure and reference wiring only where recorded in the commit/PR. It did not establish compiler acceptance of the new syntax. Keep the status notices until the owning implementation is accepted, then remove only the notices for capabilities actually delivered. Runtime and format deferral remains.

## Required reading

Read `AGENTS.md`, the full style guide and the language cheatsheet. Read compiler/build architecture references in full, `docs/compiler-data-layout-design.md`, the complete testing guide and the applicable validation-guide sections.

Use `docs/src/developer-docs/language/overview.mtf` to load all affected unsuffixed language references, especially MON, constants, calls, parameters, returns, structs, choices, collections, templates, directives and project configuration. Read the routed memory references for retained values and runtime-record boundaries. Read both progress matrices and the roadmap. Use `index.md` only as a locator.

## Fixed implementation contract

### One shared argument owner

Extend and simplify the existing call/constructor parser. One owner traverses parentheses, commas, trivia, named entries, access markers and initializer boundaries. Ordinary expression parsing and constant evaluation remain shared compiler services.

| Receiving context | Entry policy | Value policy |
|---|---|---|
| User calls, named constructors and source receiver calls | Existing positional-then-named signature routing | Existing receiving context |
| Host calls and builtin members | Existing positional-only contract | Existing receiving context |
| Statement intrinsics such as `assert` | Existing synthetic signature and routing | Existing effect and lazy-evaluation rules |
| Anonymous const record | Named-only, empty permitted | Const-required |
| Implemented template directive argument lists | Existing signature and specialised categories | Existing compile-time/category contract |
| Later general directives | Registered signature and directive entry rule | Const-required |
| Later runtime anonymous record | Named-only, non-empty | Ordinary runtime expressions |

The future MON reader uses the same owner with named-only and const-required policies plus its eventual supported-value checks. Add no reader, builder, file extension, CLI command, serialisation IR or unused extension hook now.

Use small typed receiving-context data rather than separate parser implementations, unrelated booleans, callback registries or forwarding wrappers. Reuse and simplify existing context/expectation types. The receiving kind, naming policy and evaluation policy must have one owner each.

Known signatures select and retain a parameter slot before parsing its expression. Final validation consumes that slot for defaults, types and access without rerouting. Anonymous records establish ordered named fields with duplicate detection and no invented callee signature, unknown-parameter check or fabricated parameter slots.

Keep one list loop. Ordinary entries use one expression path. Existing template-only categories may interpret their payload through their current narrow owner within that loop. Slot labels do not become source lookups, and helper wrappers are not eagerly rendered or rejected for failing an ordinary-value fold. This is category handling, not another delimiter parser.

The current template helper rejects any comma after its single expression. Replace that private separator rule with the shared list rule, including one trailing comma, while enforcing each directive's actual arity. Extra arguments remain errors. Keep bare/no-argument and specialised payload rules at their receiving boundary rather than copying parentheses handling.

Const-required contexts reuse normal constant-expression checks and final folding validation recursively. They do not use a token whitelist or second evaluator. Calls do not become globally const-only or named-only. Preserve generic inference, contextual `none`/numeric/cast handling, default filling, access checks, evaluation order and assertion-message laziness.

### Grouping, nesting and type boundaries

Use the shared named-entry recogniser at the first significant token after `(`. An immediate named initializer selects a record. An immediate `)` selects the empty-record shape. Otherwise parse an ordinary group. Inspect only the local prefix, not the complete enclosed list or the commas of nested calls. Share existing trivia rules. Newlines never replace commas.

Every anonymous field supplies one expression. Field labels are not declarations or bare names for later fields. Preserve surrounding-scope lookup, repeated labels in different children and ordinary multi-return arity rules. Infer field types without inventing expected types for `none`, empty containers or `cast`. Grouping retains its ordinary postfix, place and immediate cast-target behaviour.

Const nesting includes anonymous children, previously bound const-record children and explicitly constructed foldable nominal children. It adds no nested type declarations, structural conversion, arbitrary compile-time execution or permission to use complete const records at runtime or in collections/maps.

A runtime non-empty record remains a deferred feature through the existing runtime-record reason. Empty runtime records are rejected as outside the initial runtime shape. Ordinary `=` never silently becomes `#=`. Ordinary `f()` remains valid, and directive empty-call policy remains owned at the directive entry boundary.

### Retained syntax, folding and diagnostics

Preserve source-owned tokens and retained ranges. Parse each initializer once without cloned token windows, source reparsing or rescanning every enclosing list. Ordered entry storage and transient duplicate/slot lookup are sufficient. Avoid a new generic syntax tree or a second record IR.

Preparation still owns structural traversal and retained dependency facts. Field labels contribute no value dependency or declaration shell. References in nested initializers do contribute the existing constant-ordering and structural file-reference facts. Test same-file source order, cross-file ordering and imported folded values rather than assuming recursive parsing fixes those handoffs.

Keep anonymous const-record marker identity, folded field storage and recursive public-value projection. Preserve resource/site-root pieces, provenance and exact use sites through existing value walkers. Ordinary named constructors keep their own type identities. Runtime implementation later uses ordinary hidden struct types, never the const marker.

Carry diagnostic context through the existing source/diagnostic owners: receiving kind, available callee/directive identity, opening delimiter, label span, expression span and authored access marker. Duplicate diagnostics label both occurrences. Nested failures identify the failing inner entry. Retain context only where useful and render prose at the diagnostic boundary, not on successful parse paths. Diagnostic attribution must not select another parser loop.

### Deletion and simplification

Remove record-specific pipe classification, nested-record bans, expression terminators, qualifier parsing and scope flags. Search callers before deleting `pipe_opens_value_record`, `inside_anonymous_const_record`, `NestedAnonymousConstRecord` and equivalent cursor/slice variants. Delete redundant argument data conversions and wrappers uncovered by consolidation.

Keep valid pipe handling for function signatures, named structs, choice payloads, loops and captures. Their flat declaration grammar remains. A narrow semantic helper may still construct a const-record expression, but it must not retain a private list parser. User mistakes receive current-language diagnostics only, with no migration recogniser, fallback, automatic rewrite or compatibility fixture family.

## Required config work before directives

The reviewed config resolver rejects a top-level qualified declaration in `resolve_direct_project_config_qualifiers` before inspecting direct project fields. Relocating source alone is insufficient. Implement declaration-boundary bootstrap resolution in the existing config compiler service and AST config owner, using the delivered declaration parser and primitive resolver.

The required post-cutover form, while general directives remain deferred, is:

```moth
version #Config of String = "0.1.0"

project #= (
    name = "moth_docs",
    version = version,
    entry_root = "src",
)

html #= (
    dev_output = "dev",
    release_output = "release",
)
```

This specimen is an implementation target, not an assertion that it works before this cutover. Keep current grouped project/HTML application and the existing declaration qualifier spelling. Do not implement general directives early.

Required behaviour:

- Move embedded contracts to earlier explicitly typed top-level declarations, preserving each input name, type, fallback and required/optional state. A field then contains only `name = expression`.
- Resolve explicit input, compatible builder global and validated default/optional absence through the existing primitive resolver before dependent constants fold. Validate defaults even when an explicit input exists. Preserve ordinary same-file source order.
- Preserve the existing supported primitive/optional domain, source-module literal-default restriction and unknown-input policy. Do not add general compile-time evaluation or broaden source contracts to match bootstrap helpers.
- Preserve input dependence through names, projections, nested folded values and config-service handoff. Apply fixed-only project-field checks to that dependence, including when the input declaration has a different name from the receiving field.
- Keep build-input names and project-field identities distinct. Do not substitute a declaration name for a projected `@project` field or publish private helpers/input declarations as project metadata. Retain the current provider precedence and fixed-field override policy until their separately queued directive change.
- Review `ConfigResolutionRecord`, config projection and source-input provider consumers together. A configured project field must not be relabelled as an override-blocking fixed literal merely because field-embedded metadata has disappeared.
- Keep required project/active builder sections, existing schema domains, allowed private helpers, input type errors, source spans, output validation and config reload cleanup intact. Config still rejects dependencies, authored file values, functions and named support types.

Delete field-specific qualifier extraction, missing-value sentinels and placement checks. Preserve legitimate declaration-owned required/optional metadata. Reuse one resolver and named compiler service. The build owner consumes folded results and does not gain an AST walk or bootstrap evaluator.

## Implementation phases

Commit accepted phases separately. Each code-bearing phase includes focused tests, displaced-code deletion, the current `just validate` gate and the `AGENTS.md` Slice review. Combine tightly coupled cutover edits so an accepted commit can read its live configs and fixtures.

### Phase 0: Refresh and inventory

- [x] Satisfy the activation gate and record the accepted tree and baseline locally.
- [x] Read the published contracts. Treat the documentation work above as done, while checking for changes introduced by either cleanup.
- [x] Trace current parser consumers, declaration/header classification, nested folded values and config resolution. Revalidate the locators below against actual ownership.
- [x] Produce one local checklist of exact paths with dispositions: prepared, change with parser/config, change at closeout, unaffected with reason or external handoff. Include all queued plans and embedded snippets.
- [x] Identify primary test owners and the smallest shared context change. Fix ownership conflicts directly rather than retaining duplicate paths.

Exit: a current owner map and complete cutover checklist, not another design interview or speculative parser framework.

### Phase 1: Consolidate argument parsing

- [x] Extend the shared argument owner with named-only and const-required policies and diagnostic receiving context.
- [x] Route implemented consumers through one list loop, preserving specialised template payload handling and all signature/access/default semantics.
- [x] Consolidate grammar coverage under existing owners. Add context-specific coverage only for distinct contracts.

Exit: existing supported consumers validate on one path. Published syntax contracts remain unchanged. Record construction is not reported as implemented yet.

### Phase 2: Cut over syntax, nested values and config

- [ ] Integrate grouping/record classification, empty and single-field const records and recursive construction through the shared owner.
- [ ] Preserve visibility, ordering, folded projection, provenance and nominal identity. Retarget runtime deferral to the final syntax.
- [ ] Complete the required declaration-boundary config resolution and update live configs, scaffolds, executable fixtures, embedded Rust snippets and benchmark inputs atomically.
- [ ] Delete the displaced parser, scans, state, diagnostics and tests. Simplify the surviving declaration/parameter owners.
- [ ] Prove the contracts in the test table below before accepting the cutover.

Exit: only final value syntax is accepted and all executable inputs use it. Named nominal construction remains explicit. Runtime records and the MON format remain deferred.

### Phase 3: Complete documentation and future-plan cutover

- [ ] Apply the concrete inventory below, including remaining cheatsheet, architecture, config, teaching and future-plan edits. Published references need a support-notice update, not another rewrite.
- [ ] Remove stale code comments and all instructions that would restore displaced parsing or nesting bans. Preserve legitimate declaration examples.
- [ ] Rebuild documentation and inspect the new MON and runtime-record sections and their links. Keep runtime/format status truthful.

Exit: the maintained tree and future instructions describe one value syntax. Historical evidence is not rewritten to claim a different program ran.

### Phase 4: Verify and retire

- [ ] Repeat the tracked-file sweep. Review every residual candidate, not just a regular-expression count.
- [ ] Review shared parser ownership, diagnostic attribution, config/provider continuity and recursive folded values. Compare bounded non-recording performance evidence against the activation baseline.
- [ ] Run final `just validate` and the documentation release build. Record actual results and any inherited failure separately. Update support matrices and source/audit indexes under their policies.
- [ ] Perform the final Slice review. Delete this completed plan and its roadmap entry in the completion commit. Preserve the runtime and format follow-ups.

## Concrete cutover inventory

These are current edit/search anchors, not frozen APIs or an exhaustive allowlist. Follow renamed owners after cleanup. Plan filenames below are files to reconcile, not semantic authorities or linked prerequisites.

| Area | Exact starting targets and required disposition |
|---|---|
| Shared argument parsing | `src/compiler_frontend/ast/expressions/call_arguments.rs`, `call_argument.rs`, `call_validation.rs` and all callers, including struct/choice construction, receiver/builtin arguments and `statements/asserts.rs`. Consolidate list syntax and retained metadata, not semantic consumers. |
| Template argument consolidation | `src/compiler_frontend/ast/templates/template_head_parser/directive_args.rs`, `children_directive.rs`, `core_directives.rs`, `handler_directives.rs` and their callers. Replace single-expression parentheses helpers with the shared list owner. Preserve slot/helper/literal categories, use common separators and enforce directive arity separately. |
| Record/grouping deletion | `ast/expressions/anonymous_const_record.rs`, `parse_expression_dispatch.rs`, expression input/boundary helpers, AST scope context, `declaration_syntax/`, headers and token-scan utilities. Remove value-pipe classification and nesting flags from every producer/consumer. |
| Folded values | `ast/const_values/`, `ast/module_ast/environment/constant_resolution.rs`, `folded_value.rs`, `public_interface/`, imported-value projection and HIR constant projection. Preserve recursive field access, resource pieces and provenance through current shared walkers. |
| Config | `ast/module_ast/environment/config_resolution.rs`, `declaration_syntax/build_config_contract.rs`, `single_source_compilation/config.rs`, `build_config/`, `src/build_system/project_config.rs`, `project_config/validation.rs`, source-contract/provider projection, `src/projects/settings.rs` and builder config schema. Implement the config contract above and delete only obsolete field plumbing. |
| Executable inputs | Every tracked `config.moth`, scaffold strings under `src/projects/`, `packages/`, examples, `tests/cases/`, subsystem tests, `src/compiler_tests/`, `benchmarks/` and `xtask/`. Convert multiline/single-line source and generator templates, then rerun generated-project coverage. |
| Diagnostics and comments | `compiler_messages/` descriptors, reasons, rendering, suggestions and assertions, tokenizer/header/declaration comments, Rust module docs and test/fixture names. Remove displaced vocabulary and preserve codes/reasons for unchanged contracts. |
| Prepared language pages | `language-overview/mon-syntax*.mtf`, `constants/const-records*.mtf`, affected `functions/` and `structs/` pages, their `@page.moth` routes and developer language index. Remove only delivered-feature notices after acceptance. |
| Remaining language/teaching sweep | Cheatsheet sections `Functions and calls`, `Structs and receiver methods` > `Runtime anonymous records`, `Compile-time records and const templates` > `Anonymous const records`, `Project configuration`, `No stable syntax yet` and the opening support text. Recheck headings after cleanup. Review paired Basic pages, page introductions, `language-overview/`, `choices/`, `collections/`, `constants/`, `templates/`, packages and Design Scope for member-list equivalence or nesting bans. |
| Config documentation | `project-structure/project-config.mtf` > `Current grouped config`, `build-inputs.mtf` > current qualifier/provider text, paired Basic pages, `getting-started/`, `docs/config.moth` and any scaffold examples. Use the config specimen above, preserving the distinction from later general directives. |
| Compiler architecture | `docs/compiler-design-overview.md` > `Architectural invariants`, Stage 2 header syntax/config service, Stage 3 ordering, Stage 4 call syntax/constants and public-surface validation. Explicitly include anonymous entries in the shared owner, distinguish signature slots from fields and describe nested folded-value/config handoffs. Update relevant educational compiler pages. |
| Build architecture | `docs/build-system-design.md` > `Project bootstrap`, config service handoffs and source-input/project-global boundaries. Distinguish the delivered MON bootstrap contract from queued general directives. Keep target, graph, package and output policy unchanged. |
| Other architecture/status | `docs/compiler-data-layout-design.md` retained-syntax examples, both progress matrices, `index.md` and affected `audit-log.md` rows. Update only current representation/status facts and mark affected audits stale, not newly audited. |
| General/page directive work items | `general-directives-and-project-config-plan.md` and `html-page-directives-and-runtime-title-plan.md`. Require already delivered MON list parsing and template-list consolidation. Remove duplicated parser-delivery steps and refresh config starting-state assumptions. Keep registry, signature, placement, root-purpose and directive cutovers in their owners. |
| Wiring/runtime work items | `wiring-v1-cleanup-and-foundations.md` shared-syntax teaching and activation text, plus `runtime-anonymous-records-plan.md`. Consume published MON/deferred runtime contracts. Replace record/parameter equivalence examples and mandatory child hoisting. Keep capability storage restrictions. |
| Other queued work | Inventory every file under `docs/roadmap/plans/`, including native result slots, numeric semantics, token/data-layout reactivation, packages, Wasm and memory plans. Refresh record examples, nesting assumptions, config fixtures, parser locators and prerequisite wording. Do not copy grammar from this temporary plan. |
| Tooling and generated docs | In-repository code highlighting, formatting and code-example assets. Rebuild `docs/release/**` through the compiler. External highlighting repositories receive an explicit handoff rather than an unrequested cross-repository patch. |

### Sweep procedure

Use tracked files in the implementation worktree. GitHub default-branch search is a locator, not evidence that this branch is clean.

```bash
git status --short
git rev-parse HEAD
git ls-files docs src tests packages benchmarks xtask
git grep -n -I -E 'pipe_opens_value_record|inside_anonymous_const_record|NestedAnonymousConstRecord|parse_anonymous_const_record_expression' -- src docs tests packages benchmarks xtask
git grep -n -I -E 'AnonymousConstRecord|RuntimeAnonymousRecord|anonymous.{0,30}record|nested.{0,40}(record|list)|parameter.?list' -- src docs tests packages benchmarks xtask README.md AGENTS.md index.md
git grep -n -I -E '#=[[:space:]]*\||=[[:space:]]*\|' -- '*.moth' '*.mtf' '*.md' '*.rs'
git grep -n -I '#Config' -- src docs tests packages benchmarks xtask
git ls-files 'docs/roadmap/plans/*' 'docs/roadmap/plans/**/*'
```

An empty grep result exits with status 1 and is not a command failure. These searches produce candidates, not syntax classifications. Inspect whole literals, next-line delimiters and surrounding grammar. Also inspect escaped/raw Rust strings, template/scaffold generators and files named by the owner inventory. Do not mechanically replace every pipe or delete valid declaration tests.

Record a disposition for every candidate in local working notes. Convert maintained examples and future instructions. Preserve immutable historical audit/measurement evidence as evidence, without copying it into current teaching or adding migration notes. The final tree contains no live displaced value syntax, compatibility fixture family or instructions to restore it.

## Test and validation contract

Use the existing fixture runner and subsystem test owners. One primary case owns each observable grammar contract. Consumer-specific cases protect distinct typing, phase, diagnostic or backend boundaries, not copies of the same separator matrix.

| Contract | Acceptance evidence |
|---|---|
| Lists | Named-only rejection, duplicate first/repeated labels, positional/named ordering, unknown names for known signatures, missing values, malformed termination, comments/newlines and trailing commas. |
| Grouping | `(value)`, singleton named records, empty const/runtime cases, invalid unnamed lists, a group around a named call, chained postfix projections, places and unchanged immediate cast-target rules. |
| Recursive values | Multiple depths, repeated labels in different children, earlier bound children, explicit nominal children, surrounding-name lookup and a non-folding inner value with its exact source location. |
| Ordering/projection | Labels create no dependency edges, initializer references do, same-file forward references stay invalid, cross-file/imported nested constants project correctly and whole records remain runtime-ineligible. |
| Context boundaries | Ordinary calls, positional-only host/builtin calls, typed options/numerics/casts, assertion-message laziness and template slot/helper categories. Non-empty runtime records remain deferred. |
| Config inputs | Generated/live project parity, required/default/optional declarations, explicit and builder inputs, invalid defaults despite overrides, unknown input, source-contract matching, fixed-only field rejection through an alias/nested helper, configured versus fixed provider behaviour and private-helper exclusion from `@project`. |
| Structure/scaling | Existing compact token/span invariants, retained ranges, one initializer parse and bounded width/depth behaviour. Use current profiling/scaling facilities, not a new parser benchmark framework. |

Use folded/rendered output and narrow artefact assertions for success. Use stable diagnostic codes/reasons and relevant spans for failures. Internal slot, type or folded-store relationships justify focused unit tests. Benchmarks are not correctness coverage.

Run `cargo fmt` and `just validate` for accepted code-bearing phases. Run `moth build docs --release` or `cargo run --quiet -- build docs --release` for documentation publication and closeout. Use `just bench-check` for bounded non-recording evidence after correctness checks. Follow the activated validation guide if commands change. Report executed, failed and unavailable gates separately. An inherited exception is not a passing gate or permission for a new regression.

Final acceptance requires the shared parser, recursive const values, explicit nominal construction, working config/scaffolds and completed tracked-file sweep. The MON format still needs its own supported values, root framing, numeric/optional and collection forms, tags, escaping, ordering, resource handling, round trips, input limits and read/write contract before implementation.
