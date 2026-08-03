# Docs language migration parity ledger

Audit evidence, not a prose diary. One row per monolith section or delegated formal authority.

Stage C rechecked every row against the former monolith at commit `1548b09be`
and the delegated compiler, build-system and memory authorities. The review
restored omitted compound-assignment, option-equality, collection-access and
String-map-key rules, corrected current/deferred status drift and rebuilt every
public route before the authority switch.

The corrected Stage C final audit confirmed the focused owner map preserves the
former monolith contract. A focused follow-up audit cleared the final Prelude
ownership wording correction. No incomplete parity row or unresolved design
ambiguity remains.

The ledger distinguishes two separate states:
- **Doc parity**: whether the focused documentation owner exists and covers the source contract. `Complete` or `Incomplete`.
- **Implementation status**: what the compiler currently supports. `Supported`, `Partial`, `Deferred`, `Rejected` or `Compiler drift`.

A documentation row can be `Complete` while implementation is `Deferred` or `Partial`.

| Source heading or authority | Advanced owner | Basic owner | Public route | Related formal owner | Examples preserved | Implementation status | Remaining discrepancy | Doc parity |
|---|---|---|---|---|---|---|---|---|
| Blocks and statements | language-overview/blocks-and-statements.mtf | blocks-and-statements-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Comments and naming | language-overview/comments-and-naming.mtf | comments-and-naming-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Core values | language-overview/core-values.mtf | core-values-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Strings and characters | language-overview/strings-and-characters.mtf | strings-and-characters-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Partial | Source addition is rejected and equality is content-based across String origins. HTML-JS map keys and HTML-Wasm equality are aligned. Non-JS map lowering remains deferred. | Complete |
| Values and bindings | bindings/bindings.mtf | bindings-basic.mtf | /docs/bindings/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Mutable bindings | bindings/mutable-bindings.mtf | mutable-bindings-basic.mtf | /docs/bindings/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Shared access | bindings/shared-access.mtf | shared-access-basic.mtf | /docs/bindings/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Explicit copies | bindings/explicit-copies.mtf | explicit-copies-basic.mtf | /docs/bindings/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Shadowing | bindings/shadowing.mtf | shadowing-basic.mtf | /docs/bindings/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Numeric types | numbers/numeric-types.mtf | numeric-types-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Numeric literals | numbers/numeric-literals.mtf | numeric-literals-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Operators | numbers/operators.mtf | operators-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Checked arithmetic | numbers/checked-arithmetic.mtf | checked-arithmetic-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | Runtime arithmetic and the HTML-JS Core Math external Float boundary reject non-finite results. Other Core Math target lowerings remain tracked by the Core math row. | Complete |
| Cast syntax | casts/cast-syntax.mtf | cast-syntax-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Cast targets | casts/cast-targets.mtf | cast-targets-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Fallible casts | casts/fallible-casts.mtf | fallible-casts-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Cast evidence | casts/cast-evidence.mtf | cast-evidence-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Function declarations | functions/function-declarations.mtf | function-declarations-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Parameters and defaults | functions/parameters-and-defaults.mtf | parameters-and-defaults-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Partial | Named calls not yet supported for external builtins | Complete |
| Calls and access | functions/calls-and-access.mtf | calls-and-access-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Returns and multiple values | functions/returns-and-multiple-values.mtf | returns-and-multiple-values-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Supported | Source return slots contain the declared types and channels; inferred aliases remain compiler metadata rather than source syntax. | Complete |
| Statement if | branching/statement-if.mtf | statement-if-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Value-producing if | branching/value-producing-if.mtf | value-producing-if-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Supported | Block `if ...: then ...` is supported at closed receiving sites. | Complete |
| Pattern matching | branching/pattern-matching.mtf | pattern-matching-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Patterns and exhaustiveness | branching/patterns-and-exhaustiveness.mtf | patterns-and-exhaustiveness-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Partial | General full-match capture and String relational subjects are rejected. Full relational overlap analysis and nested choice payload patterns remain deferred. | Complete |
| Conditional loops | loops/conditional-loops.mtf | conditional-loops-basic.mtf | /docs/loops/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Collection loops | loops/collection-loops.mtf | collection-loops-basic.mtf | /docs/loops/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Range loops | loops/range-loops.mtf | range-loops-basic.mtf | /docs/loops/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Loop control | loops/loop-control.mtf | loop-control-basic.mtf | /docs/loops/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Struct declarations | structs/struct-declarations.mtf | struct-declarations-basic.mtf | /docs/structs/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Construction and fields | structs/construction-and-fields.mtf | construction-and-fields-basic.mtf | /docs/structs/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Receiver methods | structs/receiver-methods.mtf | receiver-methods-basic.mtf | /docs/structs/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Mutable receivers | structs/mutable-receivers.mtf | mutable-receivers-basic.mtf | /docs/structs/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Choice declarations | choices/choice-declarations.mtf | choice-declarations-basic.mtf | /docs/choices/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Variant construction | choices/variant-construction.mtf | variant-construction-basic.mtf | /docs/choices/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Payload patterns | choices/payload-patterns.mtf | payload-patterns-basic.mtf | /docs/choices/ | compiler-design-overview.md | Yes | Partial | Nested payload patterns deferred | Complete |
| Choice equality | choices/choice-equality.mtf | choice-equality-basic.mtf | /docs/choices/ | compiler-design-overview.md | Yes | Partial | Option payload equality now recurses through supported inner types. Struct, collection, map, fallible and external opaque payloads remain unsupported. | Complete |
| Error values | errors/error-values.mtf | error-values-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Error returns | errors/error-returns.mtf | error-returns-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | Nested-block `return!` is supported in error-only functions. | Complete |
| Propagation | errors/propagation.mtf | propagation-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Catch and recovery | errors/catch-and-recovery.mtf | catch-and-recovery-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Options | errors/options.mtf | options-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Assertions | errors/assertions.mtf | assertions-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Collection literals | collections/collection-literals.mtf | collection-literals-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Growable collections | collections/growable-collections.mtf | growable-collections-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Fixed collections | collections/fixed-collections.mtf | fixed-collections-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Collection operations | collections/collection-operations.mtf | collection-operations-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Hash maps | collections/hash-maps.mtf | hash-maps-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Partial | HTML-JS normalizes all String-like keys by content. Non-JS map lowering and broader key families remain deferred or outside scope. | Complete |
| Template basics | templates/template-basics.mtf | template-basics-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Template directives | templates/template-directives.mtf | template-directives-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Template slots | templates/template-slots.mtf | template-slots-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | Stored named insert carriers flatten through the immediate parent. Orphan inserts remain invalid. | Complete |
| Child wrappers | templates/child-wrappers.mtf | child-wrappers-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Template control flow | templates/template-control-flow.mtf | template-control-flow-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Markdown formatting | templates/markdown-formatting.mtf | markdown-formatting-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Constant bindings | constants/constant-bindings.mtf | constant-bindings-basic.mtf | /docs/constants/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Constant folding | constants/constant-folding.mtf | constant-folding-basic.mtf | /docs/constants/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Const records | constants/const-records.mtf | const-records-basic.mtf | /docs/constants/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Const templates | constants/const-templates.mtf | const-templates-basic.mtf | /docs/constants/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Type aliases | aliases/type-aliases.mtf | type-aliases-basic.mtf | /docs/aliases/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Import aliases | aliases/import-aliases.mtf | import-aliases-basic.mtf | /docs/aliases/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Payload capture aliases | aliases/payload-capture-aliases.mtf | payload-capture-aliases-basic.mtf | /docs/aliases/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Generic declarations | generics/generic-declarations.mtf | generic-declarations-basic.mtf | /docs/generics/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Type application | generics/type-application.mtf | type-application-basic.mtf | /docs/generics/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Generic inference | generics/generic-inference.mtf | generic-inference-basic.mtf | /docs/generics/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Generic instances | generics/generic-instances.mtf | generic-instances-basic.mtf | /docs/generics/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Generic limits | generics/generic-limits.mtf | generic-limits-basic.mtf | /docs/generics/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Trait declarations | traits/trait-declarations.mtf | trait-declarations-basic.mtf | /docs/traits/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Trait requirements | traits/trait-requirements.mtf | trait-requirements-basic.mtf | /docs/traits/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Conformance | traits/conformance.mtf | conformance-basic.mtf | /docs/traits/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Generic trait bounds | traits/generic-trait-bounds.mtf | generic-trait-bounds-basic.mtf | /docs/traits/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Trait incompatibility | traits/trait-incompatibility.mtf | trait-incompatibility-basic.mtf | /docs/traits/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Core cast traits | traits/core-cast-traits.mtf | core-cast-traits-basic.mtf | /docs/traits/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Trait design scope | traits/trait-design-scope.mtf | trait-design-scope-basic.mtf | /docs/traits/ | design-scope/overview.mtf | Yes | N/A | None | Complete |
| Reactive sources | reactivity/reactive-sources.mtf | reactive-sources-basic.mtf | /docs/reactivity/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Subscriptions | reactivity/subscriptions.mtf | subscriptions-basic.mtf | /docs/reactivity/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Reactive parameters | reactivity/reactive-parameters.mtf | reactive-parameters-basic.mtf | /docs/reactivity/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Mutation and invalidation | reactivity/mutation-and-invalidation.mtf | mutation-and-invalidation-basic.mtf | /docs/reactivity/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Runtime sinks | reactivity/runtime-sinks.mtf | runtime-sinks-basic.mtf | /docs/reactivity/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Reactivity scope | reactivity/reactivity-scope.mtf | reactivity-scope-basic.mtf | /docs/reactivity/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Reference semantics | memory/reference-semantics.mtf | reference-semantics-basic.mtf | /docs/memory/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Copy and exclusive access | memory/copy-and-exclusive-access.mtf | copy-and-exclusive-access-basic.mtf | /docs/memory/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Lifetimes and result shapes | memory/lifetimes-and-result-shapes.mtf | lifetimes-and-result-shapes-basic.mtf | /docs/memory/ | memory-management/overview.mtf | Yes | Partial | Mandatory lifetime-region and escape validation deferred | Complete |
| Declared memory groups | memory/declared-memory-groups.mtf | declared-memory-groups-basic.mtf | /docs/memory/ | memory-management/overview.mtf | Yes | Deferred | Accepted deferred syntax, not implemented | Complete |
| Project layout | project-structure/project-layout.mtf | project-layout-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Partial | Implementation partial while legacy package_folders and scaffold lib/ remain | Complete |
| Project config | project-structure/project-config.mtf | project-config-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Partial | Grouped project/builder records and #Import/@project/entry-local config deferred. Current compiler uses transitional flat config with package_folders drift | Complete |
| Module roots | project-structure/module-roots.mtf | module-roots-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Public API | project-structure/public-api.mtf | public-api-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Entry runtime and fragments | project-structure/entry-runtime-and-fragments.mtf | entry-runtime-and-fragments-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| HTML routing and artifacts | project-structure/html-routing-and-artifacts.mtf | html-routing-and-artifacts-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Build inputs | project-structure/build-inputs.mtf | build-inputs-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Deferred | Accepted deferred syntax, not implemented | Complete |
| Entry config | project-structure/entry-config.mtf | entry-config-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Deferred | @project and config: not implemented | Complete |
| Project package facade | project-structure/project-package-facade.mtf | project-package-facade-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Partial | Source-backed package discovery works, full facade contract partial | Complete |
| Import paths | packages/import-paths.mtf | import-paths-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Grouped and namespace imports | packages/grouped-and-namespace-imports.mtf | grouped-and-namespace-imports-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Module visibility | packages/module-visibility.mtf | module-visibility-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Public re-exports | packages/public-reexports.mtf | public-reexports-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Project-local packages | packages/project-local-packages.mtf | project-local-packages-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Partial | Implementation partial while both structural and legacy package_folders discovery paths exist | Complete |
| Legacy package_folders and scaffold lib directory | project-structure/project-layout.mtf and packages/project-local-packages.mtf | project-layout-basic.mtf | /docs/project-structure/ and /docs/packages/ | build-system-design.md | Yes | Compiler drift | Current compiler/scaffold retain package_folders and empty lib directory. Accepted design uses structural +*.moth packages | Complete |
| Package origins and backing | packages/package-origins-and-backing.mtf | package-origins-and-backing-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| External binding contracts | packages/external-binding-contracts.mtf | external-binding-contracts-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Partial | Annotated JS supported, WIT value-only deferred | Complete |
| Moth template files | moth-templates/moth-template-files.mtf | moth-template-files-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Implicit markdown | moth-templates/implicit-markdown.mtf | implicit-markdown-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Content imports | moth-templates/content-imports.mtf | content-imports-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Template scope | moth-templates/template-scope.mtf | template-scope-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | Builder-declared implicit providers are gated by semantic `.mtf` sources, and same-directory and builder constants use one collision-reporting registry. | Complete |
| Moth template limits | moth-templates/moth-template-limits.mtf | moth-template-limits-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Plain Markdown files | markdown/markdown-files.mtf | markdown-files-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Markdown imports | markdown/markdown-imports.mtf | markdown-imports-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Rendering contract | markdown/rendering-contract.mtf | rendering-contract-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Choosing a content format | markdown/choosing-a-content-format.mtf | choosing-a-content-format-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Design principles | design-scope/design-principles.mtf | design-principles-basic.mtf | /docs/design-scope/ | compiler-design-overview.md | Yes | N/A | None | Complete |
| Deferred and outside scope | design-scope/deferred-and-outside-scope.mtf | deferred-and-outside-scope-basic.mtf | /docs/design-scope/ | compiler-design-overview.md | Yes | N/A | None | Complete |
| Excluded language families | design-scope/excluded-language-families.mtf | excluded-language-families-basic.mtf | /docs/design-scope/ | compiler-design-overview.md | Yes | N/A | None | Complete |
| Core IO | packages/core/io/io.mtf | io-basic.mtf | /docs/packages/core/io/ | build-system-design.md | Yes | Partial | HTML-JS supports Core IO. HTML-Wasm and other non-JS lowerings remain deferred. | Complete |
| Core math | packages/core/math/math.mtf | math-basic.mtf | /docs/packages/core/math/ | build-system-design.md | Yes | Partial | HTML-JS uses the shared finite-Float external boundary. HTML-Wasm package lowering remains deferred. | Complete |
| Core text | packages/core/text/text.mtf | text-basic.mtf | /docs/packages/core/text/ | build-system-design.md | Yes | Partial | HTML-JS `text.length` counts Unicode scalar values. HTML-Wasm package lowering and receiver methods remain deferred. | Complete |
| Core random | packages/core/random/random.mtf | random-basic.mtf | /docs/packages/core/random/ | build-system-design.md | Yes | Partial | Seeded random deferred | Complete |
| Core time | packages/core/time/time.mtf | time-basic.mtf | /docs/packages/core/time/ | build-system-design.md | Yes | Partial | Non-JS lowerings deferred | Complete |
| Core collections | packages/core/collections/collections.mtf | collections-basic.mtf | /docs/packages/core/collections/ | build-system-design.md | Yes | Partial | Wasm lowering deferred | Complete |
| Prelude | packages/core/prelude/prelude.mtf | prelude-basic.mtf | /docs/packages/core/prelude/ | build-system-design.md | Yes | Supported | None | Complete |
| @html package | packages/builder/html/html-helpers.mtf | html-helpers-basic.mtf | /docs/packages/builder/html/ | build-system-design.md | Yes | Partial | Skeleton surface, will grow as standard library matures | Complete |
| @web/canvas package | packages/builder/canvas/canvas-drawing.mtf | canvas-drawing-basic.mtf | /docs/packages/builder/canvas/ | build-system-design.md | Yes | Partial | Skeleton surface, JS-only, deferred surfaces documented | Complete |
| WIT value-only imports | packages/external-binding-contracts.mtf | external-binding-contracts-basic.mtf | /docs/packages/ | memory-management/overview.mtf | Yes | Deferred | Accepted design, no implementation | Complete |
| Progress matrix | progress/@page.moth | N/A | /docs/progress/ | N/A | N/A | N/A | Updated for current Stage A and Stage B implementation status | Complete |
