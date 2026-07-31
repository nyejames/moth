# Docs language migration parity ledger

Audit evidence, not a prose diary. One row per monolith section or delegated formal authority.

| Source heading or authority | Advanced owner | Basic owner | Public route | Related formal owner | Examples preserved | Current implementation status | Remaining discrepancy | Completion state |
|---|---|---|---|---|---|---|---|---|
| Blocks and statements | language-overview/blocks-and-statements.mtf | blocks-and-statements-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Comments and naming | language-overview/comments-and-naming.mtf | comments-and-naming-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Core values | language-overview/core-values.mtf | core-values-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Strings and characters | language-overview/strings-and-characters.mtf | strings-and-characters-basic.mtf | /docs/language-overview/ | compiler-design-overview.md | Yes | Supported | String + String accepted by compiler, Stage B removes | Incomplete (compiler drift) |
| Values and bindings | bindings/bindings.mtf | bindings-basic.mtf | /docs/bindings/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Mutable bindings | bindings/mutable-bindings.mtf | mutable-bindings-basic.mtf | /docs/bindings/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Shared access | bindings/shared-access.mtf | shared-access-basic.mtf | /docs/bindings/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Explicit copies | bindings/explicit-copies.mtf | explicit-copies-basic.mtf | /docs/bindings/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Shadowing | bindings/shadowing.mtf | shadowing-basic.mtf | /docs/bindings/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Numeric types | numbers/numeric-types.mtf | numeric-types-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Numeric literals | numbers/numeric-literals.mtf | numeric-literals-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Operators | numbers/operators.mtf | operators-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Checked arithmetic | numbers/checked-arithmetic.mtf | checked-arithmetic-basic.mtf | /docs/numbers/ | compiler-design-overview.md | Yes | Partial | Math non-finite gap, Stage B or dedicated plan | Incomplete (math gap) |
| Cast syntax | casts/cast-syntax.mtf | cast-syntax-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Cast targets | casts/cast-targets.mtf | cast-targets-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Fallible casts | casts/fallible-casts.mtf | fallible-casts-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Cast evidence | casts/cast-evidence.mtf | cast-evidence-basic.mtf | /docs/casts/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Function declarations | functions/function-declarations.mtf | function-declarations-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Parameters and defaults | functions/parameters-and-defaults.mtf | parameters-and-defaults-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Partial | Named calls not yet supported for external builtins | Incomplete (external named calls) |
| Calls and access | functions/calls-and-access.mtf | calls-and-access-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Returns and multiple values | functions/returns-and-multiple-values.mtf | returns-and-multiple-values-basic.mtf | /docs/functions/ | compiler-design-overview.md | Yes | Partial | Source-authored return aliases still parsed, Stage B removes | Incomplete (compiler drift) |
| Statement if | branching/statement-if.mtf | statement-if-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Value-producing if | branching/value-producing-if.mtf | value-producing-if-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Partial | Block value-producing if with then deferred | Incomplete (deferred) |
| Pattern matching | branching/pattern-matching.mtf | pattern-matching-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Patterns and exhaustiveness | branching/patterns-and-exhaustiveness.mtf | patterns-and-exhaustiveness-basic.mtf | /docs/branching/ | compiler-design-overview.md | Yes | Partial | Bare-name capture and String relational still accepted, Stage B removes | Incomplete (compiler drift) |
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
| Payload patterns | choices/payload-patterns.mtf | payload-patterns-basic.mtf | /docs/choices/ | compiler-design-overview.md | Yes | Partial | Nested payload patterns deferred | Incomplete (deferred) |
| Choice equality | choices/choice-equality.mtf | choice-equality-basic.mtf | /docs/choices/ | compiler-design-overview.md | Yes | Partial | Option payload equality inside choices not yet validated, Stage B | Incomplete (compiler gap) |
| Error values | errors/error-values.mtf | error-values-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Error returns | errors/error-returns.mtf | error-returns-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Partial | Nested-block return! in error-only functions not supported | Incomplete (compiler gap) |
| Propagation | errors/propagation.mtf | propagation-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Catch and recovery | errors/catch-and-recovery.mtf | catch-and-recovery-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Options | errors/options.mtf | options-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Assertions | errors/assertions.mtf | assertions-basic.mtf | /docs/errors/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Collection literals | collections/collection-literals.mtf | collection-literals-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Growable collections | collections/growable-collections.mtf | growable-collections-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Fixed collections | collections/fixed-collections.mtf | fixed-collections-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Collection operations | collections/collection-operations.mtf | collection-operations-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Hash maps | collections/hash-maps.mtf | hash-maps-basic.mtf | /docs/collections/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Template basics | templates/template-basics.mtf | template-basics-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Template directives | templates/template-directives.mtf | template-directives-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
| Template slots | templates/template-slots.mtf | template-slots-basic.mtf | /docs/templates/ | compiler-design-overview.md | Yes | Supported | None | Complete |
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
| Lifetimes and result shapes | memory/lifetimes-and-result-shapes.mtf | lifetimes-and-result-shapes-basic.mtf | /docs/memory/ | memory-management/overview.mtf | Yes | Supported | None | Complete |
| Declared memory groups | memory/declared-memory-groups.mtf | declared-memory-groups-basic.mtf | /docs/memory/ | memory-management/overview.mtf | Yes | Deferred | Accepted deferred syntax, not implemented | Incomplete (deferred) |
| Project layout | project-structure/project-layout.mtf | project-layout-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Project config | project-structure/project-config.mtf | project-config-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Module roots | project-structure/module-roots.mtf | module-roots-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Public API | project-structure/public-api.mtf | public-api-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Entry runtime and fragments | project-structure/entry-runtime-and-fragments.mtf | entry-runtime-and-fragments-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| HTML routing and artifacts | project-structure/html-routing-and-artifacts.mtf | html-routing-and-artifacts-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Supported | None | Complete |
| Build inputs | project-structure/build-inputs.mtf | build-inputs-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Deferred | Accepted deferred syntax, not implemented | Incomplete (deferred) |
| Entry config | project-structure/entry-config.mtf | entry-config-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Deferred | @project and config: not implemented | Incomplete (deferred) |
| Project package facade | project-structure/project-package-facade.mtf | project-package-facade-basic.mtf | /docs/project-structure/ | build-system-design.md | Yes | Partial | Source-backed package discovery works, full facade contract partial | Incomplete (partial) |
| Import paths | packages/import-paths.mtf | import-paths-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Grouped and namespace imports | packages/grouped-and-namespace-imports.mtf | grouped-and-namespace-imports-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Module visibility | packages/module-visibility.mtf | module-visibility-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Public re-exports | packages/public-reexports.mtf | public-reexports-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Project-local packages | packages/project-local-packages.mtf | project-local-packages-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Partial | Partial discovery, full facade partial | Incomplete (partial) |
| Package origins and backing | packages/package-origins-and-backing.mtf | package-origins-and-backing-basic.mtf | /docs/packages/ | build-system-design.md | Yes | Supported | None | Complete |
| Moth template files | moth-templates/moth-template-files.mtf | moth-template-files-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Implicit markdown | moth-templates/implicit-markdown.mtf | implicit-markdown-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Content imports | moth-templates/content-imports.mtf | content-imports-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Template scope | moth-templates/template-scope.mtf | template-scope-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Partial | Collision model uses precedence not collision diagnostic, Stage B | Incomplete (compiler drift) |
| Moth template limits | moth-templates/moth-template-limits.mtf | moth-template-limits-basic.mtf | /docs/moth-templates/ | build-system-design.md | Yes | Supported | None | Complete |
| Plain Markdown files | markdown/markdown-files.mtf | markdown-files-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Markdown imports | markdown/markdown-imports.mtf | markdown-imports-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Rendering contract | markdown/rendering-contract.mtf | rendering-contract-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Choosing a content format | markdown/choosing-a-content-format.mtf | choosing-a-content-format-basic.mtf | /docs/markdown/ | build-system-design.md | Yes | Supported | None | Complete |
| Design principles | design-scope/design-principles.mtf | design-principles-basic.mtf | /docs/design-scope/ | compiler-design-overview.md | Yes | N/A | None | Complete |
| Deferred and outside scope | design-scope/deferred-and-outside-scope.mtf | deferred-and-outside-scope-basic.mtf | /docs/design-scope/ | compiler-design-overview.md | Yes | N/A | None | Complete |
| Excluded language families | design-scope/excluded-language-families.mtf | excluded-language-families-basic.mtf | /docs/design-scope/ | compiler-design-overview.md | Yes | N/A | None | Complete |
| Core IO | packages/core/io/io.mtf | io-basic.mtf | /docs/packages/core/io/ | build-system-design.md | Yes | Supported | None | Complete |
| Core math | packages/core/math/math.mtf | math-basic.mtf | /docs/packages/core/math/ | build-system-design.md | Yes | Supported | Non-finite Float gap, Stage B or dedicated plan | Incomplete (math gap) |
| Core text | packages/core/text/text.mtf | text-basic.mtf | /docs/packages/core/text/ | build-system-design.md | Yes | Partial | text.length uses UTF-16 code units, should use Unicode scalar values, Stage B | Incomplete (text length gap) |
| Core random | packages/core/random/random.mtf | random-basic.mtf | /docs/packages/core/random/ | build-system-design.md | Yes | Partial | Seeded random deferred | Incomplete (deferred) |
| Core time | packages/core/time/time.mtf | time-basic.mtf | /docs/packages/core/time/ | build-system-design.md | Yes | Partial | Non-JS lowerings deferred | Incomplete (deferred) |
| Core collections | packages/core/collections/collections.mtf | collections-basic.mtf | /docs/packages/core/collections/ | build-system-design.md | Yes | Partial | Wasm lowering deferred | Incomplete (deferred) |
| Prelude | packages/core/prelude/prelude.mtf | prelude-basic.mtf | /docs/packages/core/prelude/ | build-system-design.md | Yes | Supported | None | Complete |
| @html package | packages/builder/html/html-helpers.mtf | html-helpers-basic.mtf | /docs/packages/builder/html/ | build-system-design.md | Yes | Partial | Skeleton surface, will grow as standard library matures | Complete |
| @web/canvas package | packages/builder/canvas/canvas-drawing.mtf | canvas-drawing-basic.mtf | /docs/packages/builder/canvas/ | build-system-design.md | Yes | Partial | Skeleton surface, JS-only, deferred surfaces documented | Complete |
| WIT value-only imports | N/A | N/A | N/A | memory-management/overview.mtf | N/A | Deferred | Accepted design, no implementation | Incomplete (deferred) |
| Progress matrix | progress/@page.moth | N/A | /docs/progress/ | N/A | N/A | N/A | Updated for all Stage A changes | Complete |