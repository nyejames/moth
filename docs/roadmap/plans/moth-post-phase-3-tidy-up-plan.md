# Post-Phase-3 compiler tidy-up

Suggested repository path: `docs/roadmap/plans/post-phase-3-compiler-tidy-up-plan.md`

## Status

The final Phase 3 closeout is accepted at source checkpoint `4cfd9d492`, derived from the
implementation checkpoint `6309adf6d`. Start this plan from a fresh activation worktree after that
checkpoint; this plan changes implementation shape, not language semantics or compiler-stage
ownership.

Proposed placement: the post-Phase-3 checkpoint, before overlapping Wiring or native result-slot work starts. This does not reactivate diagnostic Phase 4 or change the relative order of Wiring, native result slots and later diagnostics. If overlapping work has already landed, preserve it and revalidate the affected items against that state.

Implementation baseline: establish at activation. The review revision below is historical evidence, not a preselected implementation base.

## Purpose and review basis

Remove unnecessary stored copies, forwarding layers and repeated local operations while preserving explicit compiler contracts. Prefer direct data flow and existing owners. A smaller diff or lower line count alone is not an acceptance criterion.

The eight supplied CER reports contain 30 numbered findings. This review retains 26 as bounded changes and two as separately gated representation changes. Two proposals are not scheduled. Several retained findings have a narrower correction than their reports recommend. The complete disposition table is at the end.

The reports and independent source checks refer to:

```text
Repository: nyejames/moth
Branch: diagnostic-data-layout-changes
Review revision: 073308298898ed1e9902cfa5e8641b2e6acea10b
```

The available remote branch read still resolved to that revision during review. Commit-qualified reads confirmed the principal implementations and relevant producer/consumer paths described below. The local implementation worktree and uncommitted Phase 3 changes were unavailable. GitHub search was useful for locating uses, but indexes the default branch and is not an exhaustive use proof for this branch. Activation therefore includes a committed-tree use inventory.

This is source-validated planning. No proposed patch was compiled and no compiler tests, benchmarks or runtime equivalence checks were run during this review. Performance statements below identify source-visible work or acceptance requirements, not measured improvements.

### Governing material

Follow the activated revision's `AGENTS.md` and these current authorities:

- `docs/compiler-design-overview.md` and `docs/build-system-design.md`
- `docs/compiler-data-layout-design.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- The relevant canonical language and memory references selected by the repository's reading routes

Use the supplied Moth code-review guide and the existing Style and Redundancy criteria as review aids. Its older path names do not override current repository routing. Use the roadmap and detailed migration slice records to identify overlapping work. A stale top-level status capsule does not establish migration completion.

## Decisions that correct or narrow the reports

**Canonical traversal sharing stops at equivalent semantics.** CER-03-F01 overstates equivalence. The public-interface closure walker descends into the arguments of `ModulePrivateGenericInstance`, as does `CanonicalTypeIdentity::visit`. The imported-nominal walker deliberately treats that entire variant as a leaf. The folded-value walker used by import projection inherits that pruning. Share the closure traversal only. Keep the import walkers local rather than adding a pruning framework or changing malformed/private input handling.

**Import projection remains streaming over declaration payloads.** CER-03-F02 correctly identifies cloning before category filtering. Its proposed vector of owned category payloads could retain every matching declaration at once, where the current loop retains one full record at a time. Filter before cloning and process one matching payload at a time. The existing binding-map snapshot may remain where it is needed to end a borrow.

**A cheap path iterator remains useful.** CER-05-F02 correctly identifies callers that extract a path and immediately look up the same row again. Those callers should use `ConstValueRowView`. However, `iter_module_constant_views` also indexes the value table for metadata. Keep `module_constant_paths` for genuine path-only consumers rather than forcing an unnecessary lookup or relying on optimisation to erase it.

**Storage consolidation and wrapper flattening require separate acceptance evidence.** CER-01-F01 removes real duplicate nominal payload ownership, but changes lookup locality and inherited lookup paths. CER-02-F01 removes plain enum-payload wrappers, but changes representation and field access. Neither is approved solely because fewer types or allocations appear in the source. Phase 7 contains bounded trials with explicit stop conditions.

**Retaining a small catalogue or test is not a failure to simplify.** Removing `CoreCastTrait` and broadly pruning equality/hash tests are not prerequisites for the supported production cleanup. They remain outside this plan. Obsolete API assertions may be removed within their owning change, while semantic equality, projection and registration coverage stays.

## Fixed boundaries

Keep the following distinctions intact throughout every phase:

1. `TypeId`, `NominalTypeId`, local identity bridges and stable cross-module identities remain distinct. Generated environments keep immutable inherited snapshots and local overlays. Borrowed member views and useful derived caches/indexes remain.
2. Public-interface draft, post-borrow local interface and closed provider interface remain phase-distinct. Stage 0 schedules compiler services and consumes results. It does not gain semantic orchestration or mutation APIs.
3. AST owns coercion, constant resolution and TIR. HIR receives completed semantic results, not TIR or retained AST expressions. Backends consume validated HIR and explicit selection inputs, never earlier syntax or AST.
4. Module-local folded values, stable owned public values and advisory const summaries retain their separate identity and lifetime contracts. Resource and SiteRoot pieces remain structural until the existing builder boundary renders them.
5. TIR exact-view identity, overlay authority, preparation modes, lazy execution and phase transitions stay unchanged. Formatter input and output keep their real ownership distinction. CSS and HTML implementation stays under its existing owner rather than becoming a frontend dependency.
6. Diagnostics retain their lane, code, reason, source context and ordering. Preserve short-circuit validation order and malformed-state handling. An incidental bug belongs in a separate correctness change.

Excluded: token/source ownership redesign, diagnostic storage/schema work, generic-body retention changes, Wiring/reactivity replacement, result carriers or ABI redesign, new constant-evaluation capabilities, broad interner/cache redesign, shared AST visitor frameworks and a general output-writer programme.

## Execution and acceptance rules

Implement one coherent owner-local slice at a time. The phase groups are a sequence, not a reason to combine unrelated edits into one patch. Commit accepted slices independently. Reserve `environment.rs` and canonical identity reshaping for their assigned slices rather than having concurrent writers there.

Before each slice, re-read the actual owner and affected callers. Remove superseded paths in the same change. Use one short before/after data-flow description in the completion record. Avoid new generic traits, broad contexts, registries or compatibility wrappers. A new private helper is justified only where it removes actual shared mechanics without hiding policy.

For every non-trivial code-bearing slice, run focused coverage followed by the current `just validate` gate. Follow the validation guide for formatting, feature lanes and platform checks. Update existing module comments with the change. Do not rewrite the style guide, add source-text tests for deleted symbols or rebuild the audit framework.

### Performance and output safety

Classify the impact before changing code. Moving a terminal result or removing a no-op is different from changing an access path or retaining a larger work list. Review peak live storage, allocations, traversal count, recursion depth and stack/result size, not just the number of explicit `clone` calls.

Use the existing non-recording benchmark and profiling owners. Capture the toolchain, features, build profile, thread count, machine and fixture identity. Compare the immediate accepted parent against the candidate to isolate attribution. Compare the final result against the activation baseline for accumulated regressions.

For representation trials and any suspicious timing movement, use warmed, interleaved before/after runs with at least five samples per side and report the distribution as well as the median. Use the same inputs and validate successful execution before treating lower timings as an improvement. Include a representative workload and an owner-specific workload. A neutral docs build alone cannot clear a nominal or generic lookup change.

Reject a reproducible slowdown outside measurement noise, increased peak memory caused by the new shape, additional routine copying or a readability regression. This plan grants no blanket regression allowance. When evidence is inconclusive, retain the simpler existing shape for the affected trial rather than adding machinery to rescue it. Unrelated accepted cleanups can still proceed.

Keep existing complexity budgets unchanged. The review snapshot recorded an inherited generic-scaling exception, but activation must establish whether it still exists. An explicitly accepted baseline failure is not a passing gate and is not permission to worsen it. Report failed or unavailable checks precisely.

Generated JavaScript and formatter output should be byte-identical on parity fixtures. Compare public semantic records and compiler diagnostics structurally where internal IDs legitimately vary. Do not update golden output merely to make a refactor pass. Any unavoidable internal Debug spelling change needs explicit review and must not alter public identity or source-diagnostic contracts.

## Phase 0 - Activate after final Phase 3 closeout

- [ ] Verify the final Phase 3 closeout and capture a full committed baseline SHA, branch and
  worktree status. Keep the active implementation worktree untouched until its work is complete.
  Use a dedicated tidy-up worktree from the closed checkpoint.
- [ ] Read current authorities, detailed migration status, roadmap order and any accepted cleanup
  decisions that supersede these reports.
- [ ] Revalidate each scheduled finding by symbol, including callers, consumers, tests, feature
  gates, registrations and relevant history. Use commit-qualified searches while another worktree
  is changing. Mark superseded findings as retired rather than recreating deleted code.
- [ ] Establish the baseline validation result and existing non-recording performance results.
  Record any explicitly accepted exception separately. Identify the focused fixtures needed by
  Phases 1 through 7.
- [ ] Establish a small completion record under `tmp/` containing accepted slice SHAs, dispositions
  and validation evidence. Keep raw measurements local. This is not a new audit registry.

Exit: every scheduled item still has a current owner and a specific local correction. No unfinished Phase 3 adapter is included as independent cleanup. If one item cannot be validated, exclude that item and continue with the verified remainder.

## Phase 1 - Local type and generic APIs

Sources: CER-01-F02 through F06. Evidence E1 and E2.

### 1A - Remove redundant generic-instance and unused API state

- [ ] Remove `GenericInstanceDefinition::source_key` and `TypeEnvironment::generic_instance_key` after the fresh use inventory confirms callers need only generic-instance classification. Replace those boolean callers with an existing semantic kind query or a direct `TypeDefinition::GenericInstance` match.
- [ ] Keep `GenericInstanceKey`, the interning map and the definition's ordered `base`/`arguments`. Publish the interning entry before eager substituted-member population, as today. Avoid changing recursive-instantiation order while deleting the redundant copy.
- [ ] Remove the unused `TypeKey` enum and its dead-code suppression if the exact-symbol inventory still establishes no consumer. Keep the live per-shape interning keys and both local/stable identity vocabularies.
- [ ] Remove `_compilation_stage` from `GenericParameterScope::from_parameter_list` and update current declaration-syntax, AST and test callers. Add no replacement stage enum or context.

Preserve generic classification behavior in coercion and field access, exact interning, parameter naming/collision diagnostics and separate parse-local versus canonical parameter IDs.

Verify with datatype/generic tests and the affected field-access and coercion tests. Include nested generic substitution and generated-local environments. A deleted API needs no replacement smoke test.

### 1B - Reuse inference results and postpone owned substitution snapshots

- [ ] Delete the allocating `GenericTypeBindings::is_complete_for`. In `infer_generic_function_call`, retain the first successful `concrete_arguments_for` result. Consult expected-result evidence only when arguments are incomplete and rebuild the argument list only after that additional inference is attempted.
- [ ] In `substitute_type_id`, classify the borrowed definition first, check the existing cache and construct the owned `TypeSubstitutionSource` only on a miss. Preserve empty-mapping, direct-parameter and non-substitutable fast paths.
- [ ] Reuse the borrowed definition through immutable classification/cache lookup where practical. Avoid adding a second inherited-environment traversal on every miss merely to preserve the old helper signature. Keep the narrow owned snapshot needed before recursive interning.

Preserve parameter order, argument-first inference, conflict staging, missing-inference diagnostics, deterministic cache keys and inherited cache hits. Retain the cache and its invalidation policy. This is not a substitution-key canonicalisation project.

Verify argument-complete, expected-result-assisted, conflict and unresolved inference, plus cache hit/miss behavior for constructed, function and generic-instance types. Check both hits and misses in a representative generic workload. Reject a change that optimises hits by materially slowing the common miss path.

## Phase 2 - Folded values and advisory facts

Sources: CER-05-F01 through F04, with F02 narrowed. Evidence E3 and E4.

### 2A - Retain metadata, not resolved AST values, in advisory facts

- [ ] Make `AstConstDeclarationFact` retain declaration path, scope, source, value kind and `Option<SourceSpan>`. Remove `AstConstFactValue` and its stored-ID/deep-expression alternatives.
- [ ] Return the resolved expression and its classification as a transient resolver result. Capture its span, move it into the lexical const environment and store only metadata in the completed fact table. Keep this result local to AST resolution. Avoid replacing the old return shape with a large, repeatedly copied result merely to remove a box.
- [ ] Preserve `None` for explicit top-level advisory spans and the resolved expression span for inferred facts. Keep the existing AST-to-HIR projection, reduced to metadata transfer. Do not merge the AST and HIR structs or publish an expression/store handle into HIR.

The authoritative `ConstValueStore`, lexical substitution environment and advisory summaries remain separate. Preserve mutable/non-constant omission, source-order visibility and collection after static selection. Keep both existing const walkers with their distinct timing and responsibilities.

Verify explicit, private and body-local classifications, references to earlier facts/constants, nested scope isolation, inactive-branch exclusion and AST-to-HIR span preservation. Update resolver tests to inspect the transient resolution result instead of demanding a durable expression payload.

### 2B - Use the appropriate existing value view

- [ ] Change `declaration_table_without_module_values` to consume `iter_module_constant_views` directly. Remove its same-store path-to-ID and ID-to-metadata round trips.
- [ ] Change the all-module-constant loop in generated `build_environment` to use the row's path and ID. Keep real joins and invariant failures against the separate declaration table and materialisation owner.
- [ ] Keep `module_constant_paths` for path-only search/error-list callers. Keep `value_for_path` for arbitrary lookups and `path_value_bindings` for its distinct inclusion of body-local const records. Add no replacement iterator family.
- [ ] Extract one private borrowed-piece projector from `owned_folded_string_from_const_string`. Let both its `Pieces` arm and direct `ExpressionKind::StructuralString` projection call it. Remove only the cloned temporary `ConstStringValue`.
- [ ] Remove `insert_from_place` and `walk_place_expression_for_body_local` if places still contain only Local/Field structure. Treat `Copy` as a leaf in these const walks. Leave a brief reason where useful so a future expression-bearing place form prompts deliberate review.

Preserve row order, declaration-table errors, synthetic content selection, Resource/SiteRoot ordering and stable-resource conversion. Keep the required owned public-value boundary and the expression bridges required by current resolution/materialisation APIs.

Verify store views, generated constants and structural resource defaults, including materialisation into a distinct local resource table. Keep const-fact/body-local suites. No test should assert that a helper or clone disappeared.

## Phase 3 - Public-interface closure and import plumbing

This phase number belongs to this tidy-up plan, not the completed diagnostic migration.

Sources: CER-03-F01 through F03. Evidence E5 and E6.

### 3A - Give closure recursion one owner

- [ ] Split `ClosureWork`'s recursive type traversal from its non-recursive origin-selection operation. `enqueue_type` calls `CanonicalTypeIdentity::visit`. `enqueue_folded_value` calls `PublicFoldedValue::visit_type_identities`. Both callbacks perform only the non-recursive selection/enqueue operation.
- [ ] Replace the closure module's `collect_type_origins` recursive implementation with the existing canonical visitor and a local origin collector. Preserve the evidence index's sort/dedup step and queue ordering.
- [ ] Leave `import_projection/nominal.rs`'s canonical and folded walkers local. Their private-generic pruning differs. Add a short ownership comment if needed, not a configurable visitor or an extra filtering traversal.

Preserve source nominal/base selection, repeated-origin deduplication, private identity handling and public-interface closure membership. No new trait requirement reachability rule is added here.

Verify nested folded records/choices containing option/map/generic identities, repeated re-exports, evidence dependencies and exclusion of unrelated provider records. Include the private-generic subtree distinction in focused invariant coverage. Compare the selected origin sets and published order, not only counts. A bounded local diagnostic probe may confirm eliminated repeated walking without becoming permanent test scaffolding.

### 3B - Filter before cloning, without increasing peak live payloads

- [ ] In alias, constant and free-function import projection, borrow the record and identify its semantic category before cloning. Clone only the matching payload and required provenance/origin fields, then perform the mutable projection for that one entry.
- [ ] Initially retain the existing local-path binding snapshot where Rust borrowing requires it. Remove a snapshot only when existing ownership allows a simpler borrow. Keep the established pass/iteration order and error precedence.
- [ ] Avoid collecting all owned declaration payloads into a category work vector. Keep at most one cloned declaration payload live per iteration. Leave specialised receiver/evidence-only projection outside this change.

Preserve the header-owned binding facts, stable origins, imported generics, constant provenance and consumer-local interning. No change moves projection into headers or the build system.

Verify a mixed provider surface with aliases, constants, concrete functions and generics through the existing import tests. Include structural defaults/provenance. Check allocation and peak-retention behavior on an import-heavy workload before claiming an improvement.

### 3C - Collapse the provider-set forwarding layer

- [ ] Move `ProviderInterfaceTable`'s fields and construction/registration logic into `SourceProviderDependencySet`. Keep the latter's external API and private storage. Delete the inner type and methods that only forward to `self.table`.
- [ ] Preserve `ProviderInterfaceId`, dependency-shell identity, provider kinds, pointer-repeat fast path, equal-origin agreement checks and one `ProviderBindingView` per provider.

This is an internal container consolidation, not removal of the provider input boundary. It adds no filesystem discovery, semantic compilation or earlier/later validation stage.

Verify existing dependency-binding tests for duplicate shells, implicit prefixes, input-order-independent disagreement, repeated interfaces, dense IDs and reused binding views. Confirm no new external consumer of the inner type appeared before deleting it.

## Phase 4 - Coercion and post-parse TIR flow

Sources: CER-04-F01, F02 and F04, plus CER-06-F01 through F05. Evidence E7 and E8.

### 4A - Reuse the existing scalar receiving boundary and cast row

- [ ] Remove the unused `ScopeContext` argument from `coerce_expression_to_explicit_type_boundary` and update its callers.
- [ ] Route simple assignment's final compatibility/coercion pair through that existing helper with `TypeMismatchContext::Assignment`. Keep mutability checks, parser handling and direct-option-fallback rejection in their existing order.
- [ ] Keep compound assignment's validation of the computed result separate. Keep call-specific passing/access checks, including `FreshMutableRvalue`, separate.
- [ ] In cast selection, read `lookup_builtin_evidence` once and consume its row's policy and fallibility. Delete `builtin_evidence_fallibility`, `builtin_evidence_policy` and `resolve_builtin_cast_target` once their callers are migrated.

Keep `CoreCastTrait`, the metadata catalogue and trait registration unchanged. This slice removes repeated evidence lookup, not the trait identity model.

Verify exact matches, assignment Int-to-Float promotion, optional wrapping, assignment mismatch code/reason/span, unchanged compound assignment and call access behavior. Verify builtin fallible/infallible casts and fallback to generic/user evidence when no builtin row exists.

### 4B - Simplify read-only TIR traversal and branch selection

- [ ] In `fold_tir_branch_chain_with_insertion`, select the effective expression once using overlay-or-structural fallback, then match Bool versus option capture. Remove the selector-kind/override-presence cross-product.
- [ ] In `walk_tir_view_expression_payload_node`, iterate borrowed sequence children, branches and loop headers. Remove their read-only snapshots while preserving traversal order and exact-view transitions.
- [ ] In the preparation Slot arm, use context slot-overlay presence for the final presence-only question after `effective_slot_resolution` has validated and read the overlay. Do not replace it with occurrence-result absence.

Keep authoritative view accessors and their errors. Keep snapshots used to release borrows before mutation elsewhere. Preserve all root/phase/context dimensions, first-selected-branch behavior, capture bindings, provenance and lazy semantics.

Verify both selector kinds with and without expression overlays, fallback behavior, all supported loop headers, reused roots under distinct contexts and outer overlay precedence. Test absent slot overlay, present overlay without an occurrence and malformed overlay identity separately.

### 4C - Remove pass-through result ceremony

- [ ] Return `TemplateFoldResult` unchanged in the no-context, fresh-suppressed and no-wrapper-set paths of `apply_wrapper_context_overlay_to_child_emission`. Destructure only on the work path, after preserving the existing short-circuit checks.
- [ ] Change `fold_expression_kind_to_string` to return `Option<String>` and remove `FoldedStringPiece`. The current Char consumer already allocates a string. Keep the successful append path once and preserve the `None` non-foldable diagnostic branch explicitly.
- [ ] Correct stale operational cache comments around exact-view reduction, its finalisation caller and related tests. Describe the current reducer, not a hypothetical cache.

Keep structural strings on their existing earlier path. Keep `TemplateEmission`, `TemplateFoldResult` and borrowed/owned fold-resolution distinctions. No-output is not an empty string. A skipped wrapper must not introduce a new lookup or error.

Verify no-op preservation of emission, provenance and projection pieces, wrapper order, fresh suppression, Break/Continue and conditional child emission. Preserve scalar Float formatting and Char output. Correct comments without adding a cache or changing the future caching authority.

## Phase 5 - Formatter-local cleanup

Sources: CER-08-F01 through F03. Evidence E9.

### 5A - Reuse output pieces inside whitespace processing

- [ ] Resolve formatter input once into `Vec<FormatterOutputPiece>`, run the current whitespace passes over it and return the vector directly.
- [ ] Delete `ResolvedInputPiece`, the final variant-for-variant translation and the duplicate no-pass resolution branch. Keep the existing whitespace algorithm and pass order.

`FormatterInput` still carries interned text/source context. `FormatterOutput` still carries owned text. No TIR state or phase authority enters the whitespace helper.

Verify empty/no-pass inputs, leading blank lines, first-content indentation, each run position and sequential passes. Include empty text around child, dynamic-expression and SiteRoot anchors. Preserve sealed child content and separator whitespace. Compare existing `$raw`, `$code` and `.mtf` output expectations without regenerating them to accept differences.

### 5B - Share only the existing CSS transitions

- [ ] Keep `strip_css_comments` but delegate quote/comment/escape transitions to `advance_scan_state`. Its local driver records the old index and before/after comment state, advances ordinary characters when the scanner did not consume them and copies only non-comment steps.
- [ ] Preserve the existing character buffer, scanner consume contract and stripper output capacity. Add no scanner events, trait, mode framework or cross-language sharing. If the driver becomes harder to follow than the duplicate loop, reject this sub-change.
- [ ] In `validate_declaration_statement`, replace the character-vector/string materialisation chain with `split_once(':')`, borrowed trimmed slices and `property_source.chars().count()` for the scalar colon offset.

Preserve the current first-colon heuristic, Unicode trimming, custom-property policy, warning order, saturating offsets and analysis after comment stripping. Emitted CSS remains the original structured stream, including comments and opaque pieces. This change does not repair warning-coordinate mapping.

Verify comment markers inside both quote forms, escaped quotes/backslashes, adjacent comments, unterminated constructs and Unicode. Verify missing colon, empty property/value, multiple colons and combined invalid-name/missing-value warnings. Assert exact ordered reasons and scalar offsets, plus unchanged formatter output and source-piece association using real spans.

## Phase 6 - JavaScript emitter ownership and helper selection

Sources: CER-07-F01 through F03. Evidence E10.

### 6A - Move finished output

- [ ] Make private `JsEmitter::lower_module` consume `mut self`. Return `out`, `function_name_by_id` and `referenced_external_functions` by move. Remove the three terminal clones and the now-unnecessary mutable binding at the entry point.

Preserve `JsModule`, emission order, errors and the HTML consumer contract. Verify existing direct and selected-emission tests and byte-identical output. No ownership-shaped test or new output buffer abstraction is needed.

### 6B - Make cast demand pre-emission and single-owner

- [ ] Retain `collect_used_cast_policies` as the one cast-demand owner. Remove per-statement/terminator Float-to-String scans and their temporary sets.
- [ ] Derive Float-format helper demand from the collected cast set once, alongside direct numeric/FormatFloat/ValidateFloat demand, before the prelude is emitted.
- [ ] Remove the late `used_cast_policies.insert` from expression lowering after proving all emitted cast sites are covered by the pre-emission traversal. Update its field comment to name the actual owner.

Preserve validation/error ordering where possible by combining already-collected usage at the prelude join instead of unnecessarily reordering fallible scans. Keep selected functions/blocks, standalone all-function behavior and FloatToString's formatter/trap dependencies. Do not replace these inputs with build-owned reachability or add another retained emission plan.

### 6C - Remove unused numeric-emitter deduplication

- [ ] Remove the numeric module's local `emitted` set, its threaded parameters and private insertion guards while the closed orchestration still calls each numeric emitter once.
- [ ] Keep numeric usage flags and helper order. Keep the cast runtime's real dependency deduplication. Recheck the complete Rust-side emitter call graph, not calls embedded in JavaScript strings.

For 6B/6C compare helper presence, uniqueness, ordering and bytes for numeric-only, format-only, validation-only and combined use. Include nested casts, no-cast output, selected-out external functions and both standalone/HTML modes. Run JS runtime tests for checked/trap behavior. Generated helper bodies, evaluation order and result/ABI semantics remain unchanged.

## Phase 7 - Isolated representation trials

These are valid simplification candidates, not unconditional storage mandates. Trial each independently after the bounded cleanup. Capture a fresh parent baseline and retain the change only when its checks pass. An unsuccessful trial is a documented rejection, not permission for a broader redesign.

### 7A - One nominal definition payload

Source: CER-01-F01. Evidence E1.

- [ ] Keep `types: Vec<TypeDefinition>` as the authoritative nominal payload owner. Replace secondary struct/choice payload vectors, `NominalEntry` and the nominal-to-type hash map with one dense local-suffix `NominalTypeId -> TypeId` index. Keep the path-to-nominal index.
- [ ] Register a definition once and patch only that local payload. Keep all existing nominal query APIs, early identity registration and generic-instance refresh rules.
- [ ] Preserve inherited lookup dispatch. Delegate an inherited nominal to the correct base before resolving its payload. Avoid resolving through the base and then restarting TypeId lookup from the child environment, which could add another ancestry walk.
- [ ] Remove only remap work for deleted duplicate payloads. Preserve remap order, inherited snapshot sharing, canonical/path aliases and every remaining name/path field.

Measure record size/alignment, total retained nominal/member bytes and lookup/update/remap behavior. Exercise local and inherited nominal queries, nested generated overlays, interleaved non-nominal TypeIds, generic member queries and import-heavy/generic-scaling workloads. Keep successful-compilation controls.

Focused correctness coverage must include recursive and mutually referring nominal shells, struct/choice distinction, final-field updates, generic member-cache refresh, wrong-kind/missing queries, inherited mutation rejection and sibling/nested remapping. Keep borrowed member access without reconstruction or new ownership wrappers.

Accept only if the result has one clear payload owner with stable identity and no reproducible throughput or memory regression. Reject a version that needs flattened environments, per-query allocation, broad interior mutability or a new interner framework. If rejected, retain current storage and record the measured reason.

### 7B - Flatten only enum-owned canonical payload wrappers

Source: CER-02-F01. Evidence E5.

- [ ] Recheck independent uses, mutable access and construction contracts for `CollectionTypeIdentity`, `OrderedMapTypeIdentity`, `GenericInstanceTypeIdentity` and `ModulePrivateGenericInstanceTypeIdentity`.
- [ ] If they remain enum-only records with no invariant-bearing constructor, replace them with named fields on their existing `CanonicalTypeIdentity` variants. Preserve field order, variant order, boxes, ordered argument storage and projection-time arity checks.
- [ ] Delete those four wrapper implementations and mechanically update producers/consumers. Leave FallibleCarrier and all validated/reusable origin records unchanged. Keep import walker pruning from Phase 3A.

Field privacy and current read-only APIs still need review even though the constructors perform no validation. Confirm that flattening creates no used mutation route around canonical identity/index consistency. Stop if a new independent or invariant-bearing role makes the wrapper useful.

Compare size/alignment of `CanonicalTypeIdentity` and affected aggregate records on supported relevant target layouts. Check allocation behavior, comparison/hash workloads, canonical projection, inverse interning and public-interface/generated consumers. Do not assume Rust representation or actual fingerprint consumers are unaffected because the logical fields look the same.

Keep projection, discriminating identity, malformed-arity, origin/provenance and real interner tests. Preserve capacity, key/value and argument ordering and the public/private distinction. Retain the current representation if the flattened form grows materially, slows affected work or makes consumer code harder to read.

## Phase 8 - Closeout

- [ ] Re-run focused tests for changed owners, `cargo fmt --all -- --check` and `just validate` on the final committed candidate. Follow the current validation authority if command ownership has changed.
- [ ] Run `just test-feature-matrix` for final default/standard-feature coverage. `just validate` alone does not establish every feature configuration. Run the named opt-in Boracle gate when the use inventory shows an affected shared type/identity boundary. Do not substitute a new oracle or merge independent solver logic.
- [ ] Compare final benchmark/output evidence with activation and isolate any regression to its slice. Keep unchanged budgets and report inherited exceptions without claiming a full pass.
- [ ] Recheck all changed APIs and their callers for superseded paths, hidden copies, new context/visitor machinery, widened visibility and wrong-layer dependencies. Confirm that every no-op deletion preserves its previous error/short-circuit behavior.
- [ ] Perform the repository's Slice review and an independent final review of retained Phase 7 changes, traversal semantics and output/diagnostic parity. Review tests by the contract they protect rather than the number remaining.
- [ ] Update module comments and index entries only where ownership/files changed. A pure refactor does not change language progress. Mark affected existing audit coverage stale where required, without claiming a new broad audit.
- [ ] Record each finding as implemented, superseded, rejected by its gate or deferred with a concrete reason. Retire this plan and its roadmap entry in the completion commit under the repository's normal plan convention.

## Items deliberately outside implementation

**CER-02-F02: standalone equality/hash-test pruning.** Some inspected tests are simple derive smoke tests, but their removal is not needed to make the production path simpler. Keep semantic discrimination, repeated construction, projection and interner coverage through representation changes. Remove or retarget assertions solely tied to an API actually deleted. This plan does not authorise a broad test-count reduction.

**CER-04-F03: removing `CoreCastTrait`.** The pair-to-enum-to-row translation exists, but deleting the closed named catalogue is a separate readability/API choice and the reports record an earlier deliberate decision to retain it. The clear repeated builtin-evidence lookup is removed in Phase 4A without reopening trait registration. Keep the enum and catalogue in this plan. Reconsider separately with a complete registration/use review if its remaining translation cost warrants the churn.

**Trait requirement nominal reachability.** The inspected import walker ignores Trait semantics while later trait projection may intern concrete nominal types. This is a credible correctness lead, not an executed reproducer. Verify it separately using a provider trait whose signature references a nominal selected only through that trait. Do not add missing reachability to a semantics-preserving traversal refactor or treat a newly passing test as harmless cleanup.

**CSS warning coordinates.** Scalar offsets, comment-normalised analysis text and whole-piece span mapping need their own diagnostic review. The CSS changes here preserve the existing mapping. Keep the incidental concern visible without deleting offsets or silently changing units.

**Other planned work.** Fallible carriers, reactive fields/scans, evaluator/store fusion, diagnostic snapshots, broader CFG scan reuse, formatter buffer reuse and new caches remain with their existing semantic or measured-optimisation owners. A surviving adapter after those plans finish may justify a later local deletion, not an early replacement framework here.

## Complete finding disposition

`Bounded` means the source supports the local correction, subject to activation and ordinary validation. `Narrowed` rejects part of the original prescription. `Gated` requires the independent representation trial. `Not scheduled` is a scope/benefit decision, not a claim that every original observation was false.

| Report finding | Disposition | Owning slice / decision |
|---|---|---|
| CER-01-F01 | Gated | 7A. One nominal payload, preserving identity and measured lookup locality. |
| CER-01-F02 | Bounded | 1A. Remove reverse key duplication, keep interning key and publication order. |
| CER-01-F03 | Bounded | 1B. Owned substitution source only on miss, protect miss-path cost. |
| CER-01-F04 | Bounded | 1B. Retain concrete arguments instead of allocating a completeness result. |
| CER-01-F05 | Bounded | 1A. Delete unused TypeKey after committed-tree use inventory. |
| CER-01-F06 | Bounded | 1A. Remove ignored stage label, update migrated callers. |
| CER-02-F01 | Gated | 7B. Four payload wrappers only, with layout and consumer checks. |
| CER-02-F02 | Not scheduled | No separate equality/hash-test pruning campaign. |
| CER-03-F01 | Narrowed | 3A. Share equivalent closure recursion, retain import-side pruning. |
| CER-03-F02 | Narrowed | 3B. Filter before cloning one payload, avoid an all-payload work vector. |
| CER-03-F03 | Bounded | 3C. Keep provider-set boundary, remove its nested forwarding owner. |
| CER-04-F01 | Bounded | 4A. Shared simple-assignment coercion, retain specialised receivers. |
| CER-04-F02 | Bounded | 4A. Consume the complete builtin evidence row once. |
| CER-04-F03 | Not scheduled | Retain CoreCastTrait and existing trait catalogue. |
| CER-04-F04 | Bounded | 4C. Remove scalar Text/Char result distinction, preserve None error. |
| CER-05-F01 | Bounded | 2A. Metadata-only completed facts, transient resolver values remain. |
| CER-05-F02 | Narrowed | 2B. Use rich rows where needed, keep genuinely cheap path iteration. |
| CER-05-F03 | Bounded | 2B. Shared borrowed piece projection, preserve owned stable boundary. |
| CER-05-F04 | Bounded | 2B. Remove no-op place walks after checking current place variants. |
| CER-06-F01 | Bounded | 4B. Resolve effective expression before selector-kind dispatch. |
| CER-06-F02 | Bounded | 4B. Borrow read-only payloads, retain mutation snapshots elsewhere. |
| CER-06-F03 | Bounded | 4B. Presence from context after validated effective-slot lookup. |
| CER-06-F04 | Bounded | 4C. Return unchanged fold results on skipped wrapper paths. |
| CER-06-F05 | Bounded | 4C. Correct stale cache descriptions only. |
| CER-07-F01 | Bounded | 6B. Single pre-emission cast-demand owner. |
| CER-07-F02 | Bounded | 6A. Consume emitter and move its completed result. |
| CER-07-F03 | Bounded | 6C. Remove numeric dedupe only while Rust call graph is single-owner. |
| CER-08-F01 | Bounded | 5A. Delete whitespace-local duplicate piece vocabulary. |
| CER-08-F02 | Bounded | 5B. Shared CSS transitions with a simple local copy/drop decision. |
| CER-08-F03 | Bounded | 5B. Borrow declaration slices and preserve scalar offsets. |

## Source verification index

All entries below refer to the full review revision above. Line ranges identify inspected source, not future implementation locations. Re-locate by symbol at activation. The supplied CER reports remain the historical inventories and test leads, while the narrower decisions in this plan govern implementation.

**E1 - Type payload ownership and substitution.**
`src/compiler_frontend/datatypes/environment.rs:645-835,980-1395` contains the cache ordering, owned substitution-source construction, duplicated generic key, duplicate nominal registration/patching and current inherited queries. `datatypes/ids.rs:1-95` contains the suppressed planned TypeKey. `datatypes/generic_parameters.rs:130-235` contains the ignored stage parameter.

**E2 - Generic argument materialisation.**
`src/compiler_frontend/datatypes/generic_bindings.rs` and `src/compiler_frontend/ast/generic_functions/calls.rs:410-510` establish the allocating predicate and immediate repeated materialisation.

**E3 - Advisory const facts.**
`src/compiler_frontend/ast/const_values/facts.rs`, `resolver.rs:160-240`, `src/compiler_frontend/ast/module_ast/finalization/const_fact_collection.rs:1-235,440-495` and `src/compiler_frontend/hir/const_facts.rs:1-115` show durable expression retention, the environment clone and span-only HIR consumption. `const_values/body_local.rs:230-298` shows the other no-op place walk.

**E4 - Store iteration and structural strings.**
`src/compiler_frontend/ast/const_values/store.rs:650-710` proves that rich-row iteration accesses metadata while path-only iteration does not. `ast/generic_functions/materialisation/sidecar_build.rs:65-135` and `preparation_freeze.rs:145-205` show the round-trip consumers. `src/compiler_frontend/folded_value.rs:80-435` contains the shared stable string conversion and temporary piece clone.

**E5 - Canonical payloads and traversal distinction.**
`src/compiler_frontend/canonical_type_identity.rs:35-365`, `folded_value.rs:250-330`, `public_interface/interface_closure.rs:480-960` and `ast/module_ast/environment/builder/import_projection/nominal.rs:1-170` establish the duplicate closure recursion and the different import-side private-subtree policy. `src/compiler_frontend/tests/canonical_type_identity_tests.rs:1090-1170` provides the inspected smoke-test and real-projection contrast.

**E6 - Provider and import ownership.**
`src/compiler_frontend/public_interface/dependency_bindings.rs` contains both nested containers and the validation/index policy. `ast/module_ast/environment/builder/import_projection/values.rs:1-220` and `callable.rs:1-145` show pre-filter record cloning. `ast/module_ast/environment/builder.rs:285-500` confirms that the builder owns its binding environment and preserves an explicit import pass order.

**E7 - Coercion and cast evidence.**
`src/compiler_frontend/type_coercion/contextual.rs`, `ast/expressions/mutation.rs:50-370`, `builtins/casts/evidence.rs:110-185` and `builtins/casts/resolution.rs:165-375` establish equivalent scalar boundary behavior and duplicate evidence projections. `builtins/casts/traits.rs` and `ast/module_ast/environment/traits.rs:1-270` establish the catalogue and current registration boundary retained here.

**E8 - TIR and scalar string folding.**
`src/compiler_frontend/type_coercion/string.rs`, `ast/templates/template_folding.rs:40-125`, `tir/fold/reducer.rs:495-570,910-990`, `tir/expression_sites.rs:145-300`, `tir/view.rs:415-570`, `tir/fold/control_flow.rs:1-150`, `tir/preparation.rs:485-570` and `tir/fold/wrappers.rs:1-105`, under the same template directory, establish the retained view authority and local simplifications.

**E9 - Formatter-local representations and CSS.**
`src/compiler_frontend/ast/templates/formatter_contract.rs:15-125`, `ast/templates/styles/whitespace.rs:220-460` and `src/projects/html_project/styles/css.rs:300-625` establish piece equivalence, existing boundary use, duplicate transitions and borrowed-substring opportunity.

**E10 - JavaScript emission.**
`src/backends/js/emitter.rs:1-300,530-790`, `src/backends/js/js_expr.rs:120-190` and the complete `src/backends/js/runtime/numeric.rs` establish terminal result copies, cast discovery versus late mutation and the closed numeric emission sequence.

**E11 - Validation ownership.**
`docs/src/developer-docs/style-guide/validation.mtf` distinguishes `just validate`, standard feature execution, opt-in Boracle lanes and non-recording benchmark checks. Re-read it and `justfile` at activation rather than copying historical gate outcomes from the reports.
