# MON syntax and nested const records

## Status

- Status: queued, with the maintainer's syntax direction accepted.
- Current slice: not started.
- Blockers: the accepted source/token-layout Phase 3 checkpoint and the separate post-Phase-3 compiler cleanup must be merged.
- Next action: refresh the activation tree, record the baseline in working notes and start Phase 0.

## Purpose

Use parenthesised MON syntax for value construction and supplied arguments. Enable inline nested anonymous const records through the existing call/constructor argument parser. Keep pipe-delimited parameter lists and nominal type declarations on their declaration owners.

This is one breaking syntax cutover with deletion and simplification of the displaced parsing paths. It delivers nested compile-time values and a shared argument-list owner. Runtime anonymous records remain a later implementation using that owner. The MON serialisation format and compiler-native project builder require a separate design and hardening pass.

The maintainer's accepted direction changes the current record contract. Publish it in permanent language and architecture references during this work. After publication, those references own semantics and this plan owns implementation order. Describe the resulting language directly, without compatibility guidance or historical syntax notes.

## Sequencing and activation

Run after the accepted source/token-layout Phase 3 merge and the separate compiler-cleanup follow-up. The follow-up is a required checkpoint, not work absorbed by this plan. Its exact scope and filename belong to its own authoring task.

Complete this cutover before the subsequent Wiring and native result-slot/Core const-evaluation checkpoints and before data-layout Phase 4 resumes. General directives and runtime anonymous records consume the resulting shared syntax later. Runtime records retain their separate numeric-semantics prerequisite. The main roadmap owns the full serial order and existing package pauses remain in force.

`diagnostic-data-layout-changes` is the preparation reference while Phase 3 is active. Implementation starts from the accepted merged tree, including the cleanup checkpoint, rather than an earlier main or an assumed branch state. Establish the full revision, worktree status and validation baseline at activation in local working notes. Keep this queued plan free of a speculative baseline SHA.

Preserve delivered source-owned token storage, retained ranges, exact spans and parser cursors. Use the diagnostic API present at activation. This work neither starts the deferred diagnostic-layout phases nor restores adapters removed by Phase 3 or its follow-up.

## Required reading

Read `AGENTS.md`, the complete style guide and the language cheatsheet. Read both compiler and build design authorities in full for the config and compiler-service boundary. Also read:

- `docs/compiler-data-layout-design.md`
- `docs/src/developer-docs/language/overview.mtf` and its canonical references for constants, calls, parameters, returns, structs, choices, collections, directives, templates and project configuration
- `docs/src/developer-docs/memory-management/overview.mtf` and routed leaves when validating retained values or revising runtime-record contracts
- the complete testing guide and the applicable validation-guide sections
- both progress matrices and `docs/roadmap/roadmap.md`

Use `index.md` only as a locator. Recheck moved files and active owners instead of restoring names from this document.

## Accepted language contract

### Terminology and punctuation

**MON** means **Moth Object Notation**. **MON syntax** is the documentation name for the shared parenthesised argument/value-construction notation. The **MON format** is the future const-only, named-field data profile with a separately defined serialisation contract. A function call using MON syntax is still a function call, not a serialised document.

Pipes declare parameters and typed nominal fields. Parentheses supply arguments, construct values or group expressions. Nominal constructors remain explicit:

```moth
Size = |
    width Int,
    height Int,
|

settings #= (
    name = "Moth Object Notation",
    window = (
        size = Size(width = 1280, height = 720),
        title = "Strategy",
    ),
)

window_width #= settings.window.size.width
```

This change introduces no nested anonymous type declarations. Existing pipe-delimited declaration lists remain flat. Their value defaults may use legal parenthesised expressions without introducing another declaration list.

### Grouping and anonymous construction

| Source form | Meaning |
|---|---|
| `(value)` | A grouped expression |
| `(name = value)` | A one-field anonymous record |
| `(name = value,)` | The same record with a trailing comma |
| `(first = value, second = other)` | A two-field anonymous record |
| `(outer = (inner = value))` | An inline nested record |
| `()` in a compile-time record receiving context | An empty const record |
| `(value,)`, `(value, other)` or `(,)` | Invalid value construction |

Every anonymous field has a unique identifier and an explicit `= expression` initializer. Field types are inferred. Field annotations, declaration qualifiers, omitted initializers, positional entries, implicit name shorthand and field-level mutable declarations are outside this grammar.

A field label is not a local declaration. Initializers resolve names in the surrounding scope, including `name = name`. Earlier fields do not introduce bare names for later fields. Nesting creates a value tree, not a new statement block or a second name-resolution model.

Commas separate entries. A named initializer identifies record construction. At a parenthesised expression start, use bounded token lookahead for an immediate named entry or an empty record. Otherwise parse an ordinary group. Nested calls' commas and names do not classify their enclosing parentheses. Reuse the shared named-entry recogniser and trivia rules rather than maintaining a competing probe grammar.

Each field receives one value. Existing multi-return arity rules remain unchanged and cannot create tuple fields or expand values into several fields. Grouping retains existing precedence, place behaviour, postfix handling and immediate cast-target rules.

`f()` remains an ordinary empty call. A directive's accepted bare no-argument spelling remains separate from expression-position `()`. Neither form introduces a unit or tuple type.

### Compile-time and nominal boundaries

`#=` selects the compile-time receiving requirement. A const-record initializer and every nested field must satisfy the existing constant-expression and folding rules. Calls inside such expressions still need existing compile-time support. Nesting does not add arbitrary compile-time execution.

Preserve anonymous const-record identity, folded field storage, field projection, export and provenance through their existing owners. Preserve folded nominal constructor values inside records and references to already declared const records. A complete const record remains unavailable as an ordinary runtime value. Existing collection/map and public-value restrictions stay in force.

Field labels supply no expected nominal type. An anonymous record does not become `Size` because its fields match. Continue to write `Size(...)` at a nominal boundary. Preserve ordinary contextual checking for explicitly named constructors and calls, including options, numeric literals and casts. Untyped anonymous fields receive no invented type context for `none`, empty containers or `cast`.

A non-empty record in a runtime receiving context remains the accepted deferred runtime-record feature until its own implementation. Empty runtime records remain outside that initial runtime surface. Constant-looking fields do not silently turn an ordinary `=` binding into a compile-time record.

## One argument-list owner

Extend and simplify the existing call/constructor parser. It owns delimiters, separators, trivia, named-entry recognition, access markers, expression boundaries and retained argument metadata for all relevant consumers. Existing expression parsing and constant evaluation remain their own shared owners.

| Consumer | Entry policy | Evaluation policy |
|---|---|---|
| User calls, named constructors and source receiver calls | Existing positional-then-named routing | Existing receiving context |
| Host calls and builtin members | Existing positional-only contract | Existing receiving context |
| Anonymous const records | Named-only fields, including empty records | Const-required |
| Implemented template directive argument lists | Existing directive categories and signature rules | Const-required |
| Later general directives | Their registered argument contract | Const-required |
| Later runtime anonymous records | Named-only, at least one field | Ordinary runtime expressions |

The future MON data reader will consume the same owner with named-only and const-required rules plus its validated value-domain restrictions. It does not justify a second parser or an unused reader implementation now.

### Policy and diagnostic context

Use small typed context data to express the actual receiving construct and its supported entry/evaluation policies. Prefer extending the existing context and expectation types over a new framework, callback registry or stack of forwarding wrappers. Avoid unrelated booleans and duplicated policy facts that can disagree.

Keep one list traversal and one entry-expression path. Per-context restrictions select checks and diagnostic attribution, not separate delimiter loops. Constant-required parsing uses the existing compile-time expression context and final folding validation, including nested values, rather than a token whitelist or second evaluator.

Known signatures route each supplied argument once before expression parsing, retaining the selected parameter slot for contextual checking and final validation. Anonymous fields have no predeclared signature. Record their unique names and source order without fabricating a signature, running unknown-parameter checks or treating field positions as callee slots. Adapt the existing returned argument data only where the distinction needs representation.

Carry the receiving kind and available callee/directive identity, opening location, field/argument label span, expression span and access-marker span through the existing source and diagnostic owners. Duplicate diagnostics retain the first and repeated occurrence. A nested failure points at the actual inner entry, with containing context when useful. Render prose at the diagnostic boundary, not while parsing successful input.

Preserve defaults, access validation, generic inference, evaluation order, assertion-message laziness and effect restrictions. Sharing grammar does not merge those semantic owners. Template slot labels, indices, wrappers and literal-body modes keep their specialised meaning while list structure is shared. Future Wiring receiver contexts remain immediate and cannot leak through anonymous field storage.

### Efficient parsing and deletion

Use canonical cursors and retained token ranges without cloned token windows, source rescans or token materialisation. Nested construction should recurse through ordinary expression parsing without rescanning each enclosing list. Parse each initializer once. Retain ordered entries and only the transient lookup state needed for uniqueness or signature routing.

Delete record-specific pipe disambiguation, nested-record prohibition, special expression terminators and state used only to support them. Preparation and declaration ordering still need their normal structural traversal, but field labels must not become dependency names or declaration shells. Preserve references inside nested values, including existing structural file-reference and constant-ordering facts.

Known preparation-time locators include `call_arguments.rs`, `call_argument.rs`, `call_validation.rs`, `anonymous_const_record.rs`, expression dispatch, struct/choice construction, `declaration_syntax`, headers and token scanning. Names such as `pipe_opens_value_record`, `inside_anonymous_const_record` and `NestedAnonymousConstRecord` identify deletion candidates, not APIs to preserve. Search all callers and representation-specific variants at activation.

Retain valid pipe handling for functions, nominal declarations, choices, captures and loops. Remove only the value-record responsibilities. Shared semantic const-record construction may remain where useful, but no anonymous-record module may retain its own list grammar.

## Config continuity before directives

Keep the working config compiler service operational while general directives remain queued. Convert live grouped config values, private helper records, scaffold templates and test inputs to MON syntax without implementing `$project`, `$html_builder` or `$config` early.

MON fields remain `name = expression` only. Bootstrap input contracts belong on declarations, not inside record entries. Move authored and generated field-embedded contracts to earlier top-level declarations through the existing configuration qualifier/declaration owner, then reference their resolved values from the grouped config. Keep the existing declaration spelling until the directive cutover owns its replacement.

At activation, verify which top-level bootstrap contract path is already supported. Where necessary, make the smallest config-service adjustment to resolve these declarations before consuming grouped fields. Preserve input names, supported primitive/optional types, defaults, required-input errors, explicit overrides, builder globals, validation and `@project` results. Preserve configured-value provenance so a projected field is not misclassified as a fixed override-blocking literal. Keep other config policy unchanged.

Delete embedded-field qualifier parsing, omitted-value sentinels and extraction branches made obsolete by this relocation. Share the declaration parser and input resolver already used for configuration contracts. A MON-specific config entry parser or a parallel bootstrap evaluator is not an acceptable bridge.

## Implementation phases

Each code-bearing phase includes focused tests, deletion of displaced code and the required final gate. Commit accepted phases separately. Run the `AGENTS.md` Slice review after each non-trivial phase. Combine tightly coupled cutover steps rather than committing a compiler that cannot read its own live config and fixtures.

### Phase 0: Activation inventory

1. Verify both prerequisite checkpoints and refresh current authorities, source/token owners and baseline validation.
2. Trace every call-shaped consumer, anonymous-record producer, grouping boundary, retained-header scan and folded-value consumer. Inventory parallel parser paths and relevant diagnostics.
3. Inventory source, docs, fixtures, scaffolds, benchmark inputs, comments, diagnostic prose and every queued plan. Record exact paths and their disposition in local working notes.
4. Trace config contract resolution and nested folded-value projection, including source provenance and export. Confirm the implementation work needed for the contracts above.
5. Identify the smallest shared parser/context change and primary test owners. Resolve ownership conflicts before implementation rather than preserving competing paths.

Exit: a bounded owner map and cutover checklist based on the activation tree, with no speculative parser framework or borrowed token-layout compatibility work.

### Phase 1: Publish the contract and consolidate argument parsing

1. Publish a canonical unsuffixed MON syntax reference under the language overview and route it from the compiler-facing language index. Publish const/runtime distinctions and accepted deferred boundaries in their existing authorities.
2. Extend the existing argument owner with the required named-only and const-required policies and structured receiving context. Retain ordinary call behaviour and semantic owner boundaries.
3. Route implemented consumers through that owner, including template directive list structure where it is still separate. Preserve their specialised argument categories and diagnostics.
4. Consolidate grammar tests under the existing primary owners. Add consumer tests only for distinct context or semantic boundaries.

Exit: one shared argument path ready for anonymous const records, with existing supported consumers validated. General directives and runtime records remain unimplemented.

### Phase 2: Complete the syntax and nested-value cutover

1. Integrate local grouping/record classification and parenthesised anonymous const-record construction through the shared owner.
2. Accept empty, single-field and recursive const records. Preserve projection, nested nominal constructors, constant visibility and folded public values. Keep runtime records explicitly deferred.
3. Complete config-contract relocation and update working configs, generated scaffolds, source fixtures, embedded snippets and benchmark inputs in the same accepted cutover.
4. Remove the displaced value-record parser, pipe classification, nesting bans, dead context state, duplicated scans and obsolete diagnostics. Simplify surviving parameter/declaration paths.
5. Add end-to-end nested-record cases and focused boundary/invariant coverage. Replace nesting-rejection cases with the intended positive behaviour rather than retaining obsolete contracts.

Exit: only the final value syntax is accepted. Live builds and tests use it. No compatibility parser, migration diagnostic, fallback or automatic rewrite exists.

### Phase 3: Finish the repository-wide documentation and plan sweep

Apply the inventory below systematically, including examples embedded in prose and Rust strings. Update permanent authorities first, then teaching material and queued consumers. Recheck code comments after deletion. Rebuild generated documentation through the compiler.

Revise the runtime-record follow-up to require recursive local composition through the shared parser and ordinary hidden nominal structs. Remove its contradictory aggregate-storage ban for anonymous parent fields. Preserve explicit nominal construction and transitive escape checks.

Exit: current source, comments, tests, documentation and future implementation instructions describe one syntax. Runtime and format work are clearly deferred rather than presented as delivered.

### Phase 4: Final validation and closeout

1. Repeat the repository-wide inventory and inspect every residual match. Distinguish valid declaration pipes and unrelated text from displaced value syntax.
2. Review shared-parser ownership, contextual diagnostics, nested folded-value preservation, config continuity and the revised runtime contract.
3. Run final validation, documentation generation and bounded non-recording performance checks. Investigate regressions instead of attributing them to the earlier token work without evidence.
4. Update implementation status, affected source locations and stale audit-log entries under their existing policies. Preserve the separate runtime and MON format follow-ups.
5. Perform the final Slice review. Delete this completed plan and its roadmap entry in the completion commit.

## Documentation and source inventory

The activation inventory is mandatory. This table names starting areas, not a finite allowlist.

| Area | Required update |
|---|---|
| Language authorities | MON syntax, constants/const records, struct construction, function calls, parameter/default rules, grouping, returns, choices, collections and directives |
| Teaching and routing | Paired Basic pages, page introductions, language overview, cheatsheet, compiler-facing index and Design Scope |
| Architecture | Shared argument ownership, const-record handoffs, retained syntax and config service boundaries in compiler/build references and relevant educational pages |
| Templates and project docs | Template directive arguments, helper examples, config/build-input pages, package/project examples and metadata examples |
| Runtime follow-up | Nested anonymous parents/children, mixed explicit nominal children, hidden identity, empty-case policy, transitive escape and shared tests |
| Other queued work | General directives, page directives, Wiring shared-syntax teaching, numeric/result-slot work, data-layout reactivation notes, package/config and Wasm plans wherever affected |
| Executable inputs | Live configs, scaffold generation, examples, packages, tests, benchmark inputs, Rust raw-string fixtures and helper generators |
| Diagnostics and comments | Descriptors, reasons, render text, suggestions, module docs, invariants, tokenizer/declaration commentary and test names |
| Tooling and generated material | In-repository highlighting/formatting recognition, syntax examples, documentation generation and source index entries |

Use MON terminology consistently without calling every MON-syntax value serialisable. Keep declarations and definitions described as declarations, not MON arguments. Update future plans to consume delivered capabilities rather than reproduce parsing work or reference this temporary plan.

Search by semantic names and prose as well as punctuation. Include single-line and multiline records, nesting prohibitions, record/parameter equivalence claims and embedded code. Review matches manually so valid pipes remain intact. Remove obsolete fixtures and coverage instead of leaving a historical compatibility suite.

Generated `docs/release/**` files are rebuilt, never hand-edited. Retire superseded tracked explanatory snippets through their owning document policy rather than rewriting historical validation evidence as if another program had run. Git history needs no migration appendix. External tooling repositories require an explicit handoff and are not silently included in this repository change.

## Test requirements

Use one primary owner per contract. Centralise shared grammar coverage and keep consumer-specific cases for real policy differences. Use the existing fixture runner and subsystem test locations, not a new MON harness.

| Contract | Required evidence |
|---|---|
| Shared lists | Positional/named ordering, required names, duplicates, unknown names for known signatures, separators, trailing commas, trivia, empty forms and malformed termination |
| Grouping | Nested calls do not classify an outer group, named singletons need no comma, unnamed lists remain invalid, precedence/postfix/places and cast-target boundaries remain correct |
| Nested const values | Multiple depths, repeated field names in different children, earlier constants, explicit nominal children, grouped fields, chained projection and a nested non-folding failure |
| Visibility and handoff | Field labels do not create local names or ordering edges, initializer references do, exported nested folded values retain identity/provenance and complete records stay runtime-ineligible |
| Context differences | Runtime record deferral, empty runtime rejection, positional-only builtins, typed options/casts, assertions and implemented template directive categories |
| Config | Live and generated projects, declaration-owned inputs, required/default/optional values, override/provenance behaviour and unchanged applied settings |
| Diagnostics | Stable codes/reasons, exact inner source locations, duplicate first-use labels and correctly attributed call/constructor/directive/record errors |
| Structure and scaling | Existing fixed token/span invariants, retained-range ownership, one initializer parse and bounded width/depth behaviour without a new benchmark subsystem |

Use observable folded output or narrow artefact assertions for success and structured diagnostic expectations for failure. Internal identity/routing facts justify focused unit coverage. Benchmark fixtures are performance evidence only. A final-grammar rejection may cover a pipe in value position, but it carries no migration message or preserved obsolete fixture family.

## MON format and builder follow-up

Record this as accepted future direction, not implemented syntax capability:

- A compiler-native MON project builder and reusable compiler-library service use the same core lexer, argument parser and constant-value owners.
- The MON data entry policy is const-only and named-only. The format explicitly defines its recursive supported value domain and accepted authoring surface before implementation.
- The builder owns serialisation validation and output rules. Compiler services own language parsing and folding. Clients do not assemble raw compiler stages.
- Engine/application schemas validate the resulting data separately. Templates continue to produce ordinary strings and carry no implicit live bindings or executable routing state.

The separate design pass must settle root framing, scalar/numeric representation, optional/absence values, collections/maps, nominal or choice tags, string escaping, key/order rules, resource-bearing strings, round trips, depth/size limits and the reusable read/write API. It must also distinguish permitted compile-time authoring expressions from the self-contained serialised representation.

This plan adds no MON builder, file extension, CLI command, serialisation IR, reflection system, runtime decoder or speculative extension API. No const value becomes serialisable merely because it uses MON syntax.

## Validation and completion

Run `cargo fmt` and `just validate` for each accepted code-bearing phase. Run `moth build docs --release`, or `cargo run --quiet -- build docs --release`, when documentation changes and at final closeout. Use `just bench-check` for non-recording performance evidence after correctness checks. Follow the current validation guide if commands change.

Record executed commands and outcomes accurately. Separate inherited failures from new ones and from checks that could not run. Existing exceptions do not authorise weakening tests or declaring an incomplete gate green.

Final acceptance requires one shared argument parser, inline nested const values, explicit nominal construction, working config/scaffolds, the complete repository sweep and a coherent deferred runtime/format boundary. The completed tree contains no compatibility path or migration notes for the displaced value syntax.
