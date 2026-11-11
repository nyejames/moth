# Recent compiler performance and redundancy follow-up

## Status

```text
STATUS: ACTIVE - dedicated performance branch
CURRENT_SLICE: 1 - one-pass ordering and range work
BLOCKERS: none
NEXT_ACTION: implement P1 from the rebased occurrence index, then evaluate P2 and P3 against measured work
```

Branch: `perf/recent-compiler-performance`

## Goal

Investigate regressions reported after the `$export` and const-default entry-template cutovers and the recent module, template, Never and diagnostics changes. Remove demonstrably redundant work and simplify ownership or dispatch where the replacement is clearer. Attribute performance improvements to measurements rather than to clone counts or timing summaries.

This is a bounded code-quality and performance plan, not a frontend redesign. Each accepted patch should remove work or complexity, retain a single semantic owner and leave the compiler smaller or easier to understand. Stop rather than building a new cache, IR, generic dispatch framework or compatibility path.

## Authorities and starting state

Before implementation, read the current worktree's `AGENTS.md`, style guide, testing and validation routes, the relevant compiler and build architectural sections, the canonical language references for directives and templates and the current progress matrix. Use `index.md` only to find owners. Read the code-review guide provided with the handoff.

At activation, `main` is `b7aa8476146a3c4bfbdb3efd393b87fa12ae2cd8` (2026-11-11). This branch was created from that commit. Implementation begins with a new benchmark on this branch. Historical comparison points are pre-export `c86b6000b205b1af088bafde1a87dcd39963d8f1`, export-on-main `b83d965dd0235b0e7a6c6a93e8dbb104d6877773` and the early template branch `84923ba4bdea70af3008dd80f4b5651cf724a905`. They are historical experiments, not current implementation targets.

The November 11, 13:52 macOS Apple Silicon (6D851D) report recorded about +2 ms average, zero faster and five slower cases among 50 comparable cases. Twenty-four of 74 workload fingerprints changed. Aggregate check movement was +69 ms with +64 ms in frontend and single-file frontend. These totals are not per-invocation overhead and changed-fingerprint cases cannot establish a regression in the compiler. No new Apple Silicon timings were run during plan preparation.

Recent merged follow-ups already:
- correct source directive attachment, metadata target remapping and adjacent directive validation
- reject direct runtime `$note` using the parsed TIR kind, not a raw next-token probe
- give header metadata folding its own capacity allowance without consuming a body-root reservation
- move documentation metadata to its AST owner

Do not reinstate the older parser probe, introduce a second documentation fragment owner or charge metadata parsing against executable body capacity. Review recommendations below against the live tree after each slice.

## Engineering constraints

- Each source is tokenized and declaration-shell parsed once per applicable lane. One TIR store remains authoritative for templates.
- Preserve the existing Stage 0 preparation, Stage 2 reference capture, Stage 3 ordering, Stage 4 AST, HIR, borrow and builder boundaries.
- Keep `SourceId`, `StringId`, `PathId`, qualified-reference ordinals, donor identities, module interfaces and stable diagnostic witnesses correct. An optimization cannot hide missing dependencies, errors or source spans.
- Preserve top-level const fragments, explicit runtime `~[...]`, named builder slots, structural strings and `$note` no-output semantics.
- Do not create parallel metadata, compatibility shims, new persistent indexes, second scanners or cross-stage caches.
- Reuse existing tests and instrumentation. Tests belong in `tests/` owners, not inline production modules. Add one test per distinct failure boundary, not fixture permutations for their own sake.
- Performance work requires before/after comparable input, release/build profile, medians, dispersion and stage-level attribution. Pure cleanup needs proof of equivalent behaviour and a visible reduction in code or data movement, not a manufactured speedup.

## Confirmed opportunities and investigations

### P1. Eliminate repeated qualified-reference ordering visits

Owner: `src/compiler_frontend/headers/ordering_hints.rs::collect_qualified_reference_ordering_hints`, with `headers/symbol_collection.rs::build_module_symbols` as read-only context.

Current code loops over module headers. For each header it fetches that source's complete `qualified_references_by_source` list and filters on `owning_header_index`. With H headers and Q occurrences for a source, this can inspect O(H * Q) entries even though each occurrence has one owner. This is confirmed redundant work, not a measured source of the +2 ms signal.

Important: `build_module_symbols` already rebases each occurrence's file-local `owning_header_index` using `header_base` to the aggregated module-wide header order. Reuse that index. The implementation should visit each occurrence once and insert its existing resolved hint into `headers.get_mut(occurrence.owning_header_index)` after checking the source and index contract. There is no need for a new header locator, dependency graph or permanent side table.

**Identity and ordering gate:** inspect the header aggregation order at `headers/parse_file_headers.rs` and prove that each rebased index points at a header owned by the occurrence's source. Do not iterate an unordered map if that would change the selection of an error witness or diagnostic order. If needed, drive the visit from an existing deterministic source order, not a newly invented order. Preserve the early no-resolved-references fast path.

**Validation:** existing qualified-reference and namespace binding tests plus a focused multifile case with independently rebased indexes, multiple occurrences per header, runtime-only occurrences, imported/local constant targets, missing targets and cycle witnesses. Check that every occurrence is inspected once and emits at most its intended hint. Use increasing header/occurrence counts to demonstrate O(H + Q) visits. This cleanup is approved even if wall time is neutral.

### P2. Bound const-range overlap visits without another scanner

Owner: `src/compiler_frontend/headers/qualified_references.rs::split_runtime_ranges_around_const_required`.

The helper sorts and merges const-required intervals, then searches from the first merged interval for every runtime range. Worst-case candidate visits grow O(R * C), where R is runtime ranges and C is disjoint const intervals. The empty-const case already returns directly.

Prefer a small in-place improvement: after merging, find the first possibly overlapping interval using `partition_point` (or an equally short correct search), then visit actual overlaps only. Avoid a second sorted index or interval tree. Keep the final scan order and the priority of const-required ranges unchanged. Check adjacent intervals, overlapping runtime ranges, fully covered ranges, missing ranges and paths spanning separate sources.

**Gate:** compare code size and interval visit counts on sparse and dense defaults/operand workloads. If the indexed implementation adds complexity for negligible work reduction, keep the current helper and record why. The algorithm is an improvement candidate, not an automatic rewrite.

### P3. Inspect repeated dependency selection lookups

Owner: `src/compiler_frontend/headers/ordering_hints.rs::dependency_path_for_local_name`.

For each bare type spelling the helper walks dependency clauses and their selections. Many named types against high import fanout could repeat the same lookup. The exact cost has not been measured.

First inspect existing dependency-selection and binding tables for an already canonical name-to-path lookup usable at this stage without changing provider-independent preparation. Profile dense type references and import-fanout inputs. Use that existing table if correct. A temporary file-local index is justified only by profiling and a proven reduction in code/work. Do not duplicate the namespace authority or move provider-dependent resolution into preparation.

### P4. Avoid copying owned const-fragment results at finalization

Owners: `src/compiler_frontend/ast/templates/top_level_templates.rs::collect_const_top_level_fragments` and `ast/module_ast/finalization/finalizer.rs::finalize`.

The collector borrows `FxHashMap<PathId, FoldedConstTemplateResult>` and clones each output's `ConstStringValue` and `ConstFragmentDestination`. `finalize` already owns `AstEmission` and, on the inspected path, has no later consumer of this map. This is an ownership cleanup independent of timing.

Prefer transferring the map with `std::mem::take` or consuming the owned map and removing values by path while constructing ordered `AstConstTopLevelFragment` records. Preserve the missing-result compiler-invariant error, `NoOutput` skip, authored ordering and named slot destination. Avoid adding per-value `Arc` or a replacement result map. If another consumer exists, locate it before changing the signature.

**Validation:** large folded fragments, plain and piecewise structural strings, site-root and resource values, named destinations, `$note` and interleaved static/runtime output. Verify output and all identity/remapping boundaries.

### P5. Borrow folded source text rather than cloning it

Owner: `src/compiler_frontend/style_directives/source.rs::source_directive_text` and `flatten_source_string`.

Both accepted const-value payload variants clone `ConstStringValue` before text flattening. The const-value store is only read here and the string table is independently mutable.

Change `flatten_source_string` to take `&ConstStringValue`. Return the existing `StringId` for a plain value and borrow pieces while assembling text. Keep the special `SiteRoot` spelling and the existing error for unresolved `Resource`. No new adapter or string type is needed.

**Validation:** plain, empty, large and interpolated `$doc`/`$todo` strings, grouped operands, donor-backed generic bodies, structural resources and exact error spans. This is an approved small simplification, even if timing is unchanged.

### P6. Investigate visible-declaration set clones at function creation

Owner: `src/compiler_frontend/ast/module_ast/emission/emitter.rs` in `emit_function`, `validate_generic_function_body` and `emit_generic_function_instance`. Adjacent readers: `ast/module_ast/scope_context.rs` and its `builders.rs`.

These paths clone the entire immutable `visibility.visible_declaration_paths` set, insert parameter paths and wrap the result in an `Arc`. The repeated pattern is real, but the parameters may need a merged visibility contract. Inspect all `visible_declaration_ids` reads and local-frame registration first. Test whether the existing local scope frame already provides the parameter names while the shared file-visible set can remain borrowed or arc-cloned.

Prefer deleting the copied-set construction if the current lookup ownership proves it redundant. Do not replace it with a generalized layered set, a fresh scope trait or a more complicated lifetime graph. Retain copies if they are the simplest correct representation. Measure many-function/many-visible-symbol combinations and generated or frozen generic contexts. Preserve private imports, receiver methods, parameter collision checks and cross-file visibility.

### P7. Simplify header/statement dispatch only where the hot path stays explicit

Owners: `src/compiler_frontend/headers/file_parser.rs`, `headers/top_level_classifier.rs`, `style_directives/source.rs` and `ast/statements/body_dispatch.rs`.

The current header walk tracks `runtime_nesting`, executable block depth and an optional pending source-directive run. The statement parser shares operand capture/folding but must retain its own AST-local attachment and semantic validation. These are distinct stage responsibilities, so a generic unified parser would add indirection.

Candidates:
- Compute the statement-boundary predicate for runtime `~[...]` only when the current token is `MUTABLE`, the context is `Module` and the next token is `TEMPLATE_HEAD`. This removes an unconditional previous-token lookup on unrelated statements and makes the candidate guard visible.
- Audit which source-directive registration/duplicate checks are genuinely common. Share only a short validation rule if it deletes duplicated behaviour while preserving target-specific diagnostics. Keep the existing common `capture_source_directive_operand` and `fold_source_directive_operand` helpers.
- Inspect whether `runtime_nesting` and executable block depth must remain separate dimensions. Preserve nested runtime templates and in-block declaration classification. Combine them only if the new structure is demonstrably smaller and equally correct.
- Inspect the per-annotated-header `fold_header_source_directives` scope/context creation. A reusable context or cached operand is not justified without a profile and proof that each header's diagnostics, visibility and const store remain isolated.

**Gate:** compare annotation-free sources, export-only files, doc/todo-heavy sources and direct const/runtime templates. Keep balanced syntax, same-line whitespace, blank-line stops, duplicate names, unexpected arguments and diagnostic sites exact. The accepted runtime `$note` check is TIR-based. Do not restore the removed raw-token probe. Continue to fold `$todo` because it is semantically validated.

### P8. Profile adjacent work but avoid scope creep

- `headers/file_parser.rs::attach_structural_file_facts`, `qualified_references.rs` and `ordering_hints.rs` walk overlapping token ranges to produce different facts. Inspect call counts and ownership before suggesting a shared scan. Prefer one authoritative prepared fact over rescanning, but do not build a new combined token walker merely for fewer loops.
- Recent Never and private-failure work may affect `frontend.hir`, `frontend.borrow.initial` and `frontend.borrow.converge`. Profile on normal and Never-heavy call chains before touching them.
- General borrow-state cloning, recursive return-alias reachability and private-failure rechecks already have a living production performance/research owner. Record new evidence there. Do not replace that plan or modify borrow algorithms under this limited cleanup.
- Broad template parser/formatter/TIR fold scheduling and cache changes already have their own queued performance owner. Transfer evidence rather than expanding this work into a second parser optimization programme.
- Stage 0 already removed earlier whole-string-table and path-resolver copies. Reinvestigate only if current profiling demonstrates a reintroduced problem.

## Execution

### Slice 0 - Establish comparable measurements

1. Record the new branch head, toolchain, profile, flags and benchmark schema. Check a clean worktree. Record the old 6D851D report as historical evidence, not a reproducible benchmark claim.
2. Run `just bench-check` and `just bench-frontend-check` without updating benchmark history. Use `just bench-scaling` for existing scaling workloads. Use `just bench` only when intentionally recording a new canonical benchmark after the result is reviewed.
3. Collect raw per-case results. Split unchanged fingerprints from the 24 changed workloads. Use identical source bytes when comparing historical compilers that accept them. Otherwise author equivalent paired fixtures and record their token, header, module and fragment counts. Never compare an invalid pre-migration input with a valid post-migration input.
4. Use separate release artifacts/worktrees for the historical points if those comparisons remain useful. More importantly, obtain a fresh within-main baseline and before/after measurements for every patch.
5. Sample representative unchanged and template/directive-heavy cases. Attribute `stage0.directory.inventory`, `stage0.directory.compile`, `frontend.prepare`, `frontend.bind_headers`, `frontend.order_declarations`, `frontend.ast.total` and its emit/finalize metrics, `frontend.hir` and both borrow stages. Accumulated metrics and nested evidence are not additive. Use `just profile-case <case> terse` when useful. Record missing symbolication instead of guessing stack owners.
6. Record at least seven warm samples per comparison variant with median and spread, ideally interleaved, on an idle machine. Capture input fingerprints and exact invocation. Profiling probes must not dominate their own sample.
7. Create a brief evidence table under `benchmarks/` or PR notes that identifies real owners, changed metrics, supported hypotheses and noise.

**Gate:** identify a credible owner or explicitly report that the small reported regression remains unproven. Confirmed local cleanups P1, P4 and P5 may proceed independently of that attribution.

### Slice 1 - One-pass ordering and range work

Implement P1 first, reusing the already rebased occurrence index. Add one meaningful multifile regression case and an existing-owner scaling test. Benchmark before and after that patch. Investigate P2 and P3 independently. Keep P2/P3 only if the simpler implementation or measured work improvement justifies them.

**Gate:** no duplicate graph, scanner or index, deterministic witnesses, same source and provider edges and no normal-case regression.

### Slice 2 - Ownership and local dispatch cleanup

Apply P5 and P4 as small separate commits after proving their transfer/borrow invariants. Simplify the runtime marker predicate from P7. Review the remaining P7 code and P6 visibility clones against direct readers. Make further changes only when they remove real code/data work without a more elaborate abstraction.

**Gate:** no loss of output, source errors, donor identity, module visibility or compilation budgets. Neutral microbenchmarks are acceptable for demonstrably better ownership and simpler code.

### Slice 3 - Optional profiling-directed follow-up

Select at most one additional P8 owner if the baseline attributes a material regression to it. Record unrelated borrow or template evidence in its established companion instead. Stop when only noise or intended semantic validation remains.

### Slice 4 - Full verification and completion

Run the current `justfile` routine/full code gates selected by the validation guide, with the final merge gate including Clippy, formatting, source audits, docs validation, integration fixtures, scaling and timer erasure. Re-run `just bench-check`, `just bench-frontend-check` and `just bench-scaling` on the final code.

Include tiny single-file, standard module, multi-module, docs, qualified/default scaling, constant chain, template stress and borrow-heavy controls. Confirm output determinism, `$export` interfaces and fingerprints, exact diagnostics, generic materialization and static/runtime fragment insertion order.

Perform a strict final Slice review against the project style guide. Delete superseded code, redundant tests, stale comments and temporary probes. Document before/after medians, dispersion, allocator/memory evidence when available, the specific LOC/dataflow simplifications and discarded approaches. Do not assert a performance win for an unmeasured cleanup.

When implementation is complete, retire this plan in the completion commit and remove its roadmap entry, following the normal short-lived plan rule. Keep only enduring benchmark evidence, distinct regression tests and accepted design changes.

## Stop conditions

Stop or keep the original code if a proposed improvement needs a second parser, adds persistent metadata solely for speed, breaks file/module index mapping, changes diagnostic selection, duplicates builder/template identity, obscures the stage boundary or raises complexity without a credible benefit. A measured slowdown on representative normal workloads is a reason to revert an optional optimization, even if a synthetic stress test improves.
