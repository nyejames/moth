//! Projected source-constraint boundaries and fast-use witnesses.
//!
//! WHAT: proves HIR carries the function/statement boundaries, the folded-operation ledger rows,
//!       generated fast range guards and inactive static-branch filtering.
//! WHY: verification runs after AST bodies are gone, so evidence must be projected intact onto
//!      each HIR function while inactive static branches stay excluded.

use crate::compiler_frontend::hir::failure_facts::{
    HirConstraintBoundary, HirFastUseProvenance, HirFunctionFailureFacts,
};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

fn lower_source(source: &str) -> (HirModule, PathInternerFork, StringTable) {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
    (hir.hir_module, path_fork, string_table)
}

fn function_id_by_name(
    hir: &HirModule,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> FunctionId {
    hir.functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|component| string_table.resolve(component) == name)
        })
        .unwrap_or_else(|| panic!("lowered module should contain function {name}"))
        .id
}

fn facts_by_name<'a>(
    hir: &'a HirModule,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a HirFunctionFailureFacts {
    let function = function_id_by_name(hir, path_fork, string_table, name);
    hir.function_failure_facts
        .get(&function)
        .unwrap_or_else(|| panic!("function {name} should carry projected failure facts"))
}

fn source_text(source: &str, span: crate::compiler_frontend::source::SourceSpan) -> &str {
    let range = span
        .local()
        .inline_range()
        .expect("small authored test spans resolve inline");
    &source[range.start() as usize..range.end() as usize]
}

fn fast_statement_count(hir: &HirModule) -> usize {
    hir.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| match &statement.kind {
            HirStatementKind::NumericOp { op, .. } => op.semantics.is_fast(),
            HirStatementKind::LoopStepFailure { semantics, .. } => semantics.is_fast(),
            HirStatementKind::FastStoreConversion { .. } => true,
            _ => false,
        })
        .count()
}

#[test]
fn function_and_statement_boundaries_project_with_directive_spans() {
    let source = "\
$safe_math
guarded || -> I32:
    $infallible
    return 1
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "guarded");

    assert_eq!(
        facts.constraint.boundaries.len(),
        2,
        "the function body and its annotated statement each keep one boundary: {:?}",
        facts.constraint.boundaries
    );
    let HirConstraintBoundary {
        statement: None,
        constraints,
        directive_spans,
    } = facts.constraint.boundaries[0]
    else {
        panic!(
            "the function boundary must come first, got {:?}",
            facts.constraint.boundaries
        );
    };
    assert!(constraints.safe_math_active());
    assert!(!constraints.infallible_active());
    assert_eq!(
        source_text(
            source,
            directive_spans
                .safe_math
                .expect("the function boundary retains its own guarantee directive")
        ),
        "$safe_math"
    );
    assert_eq!(
        directive_spans.infallible, None,
        "an inactive guarantee keeps no label span"
    );

    let HirConstraintBoundary {
        statement: Some(statement),
        constraints,
        directive_spans,
    } = facts.constraint.boundaries[1]
    else {
        panic!(
            "the statement boundary must follow the function boundary, got {:?}",
            facts.constraint.boundaries
        );
    };
    assert!(
        constraints.safe_math_active(),
        "the statement inherits safe math"
    );
    assert!(constraints.infallible_active());
    assert_eq!(
        directive_spans.safe_math, None,
        "an inherited guarantee leaves the declaring boundary's label in place"
    );
    assert_eq!(
        source_text(
            source,
            directive_spans
                .infallible
                .expect("the statement boundary retains its own guarantee directive")
        ),
        "$infallible"
    );
    assert_eq!(
        source_text(source, statement.start),
        "return",
        "the statement boundary starts at the annotated statement"
    );
}

#[test]
fn inactive_static_branch_evidence_is_pruned_while_the_selected_branch_is_kept() {
    let source = "\
$fast_math
inactive || -> U8:
    if false:
        $safe_math
        return 200 + 56
    else
        return 1
    ;
;
$fast_math
active || -> U8:
    if true:
        return 200 + 56
    else
        return 1
    ;
;
";
    let (hir, path_fork, string_table) = lower_source(source);

    let inactive = facts_by_name(&hir, &path_fork, &string_table, "inactive");
    assert!(
        inactive.constraint.fast_use.is_empty(),
        "the discarded branch's folded fast addition must not survive projection: {:?}",
        inactive.constraint.fast_use
    );
    assert!(
        inactive
            .constraint
            .boundaries
            .iter()
            .all(|boundary| boundary.statement.is_none()),
        "the discarded branch's $safe_math statement boundary must not survive: {:?}",
        inactive.constraint.boundaries
    );

    let active = facts_by_name(&hir, &path_fork, &string_table, "active");
    assert_eq!(
        active.constraint.fast_use.len(),
        1,
        "the selected branch keeps exactly one folded fast-operation witness: {:?}",
        active.constraint.fast_use
    );
    assert_eq!(
        active.constraint.fast_use[0].provenance,
        HirFastUseProvenance::AuthoredArithmetic
    );
    let span = active.constraint.fast_use[0]
        .span
        .expect("the authored operator keeps its span");
    assert_eq!(
        source_text(source, span),
        "+",
        "the witness keeps the authored operator span"
    );

    assert_eq!(
        fast_statement_count(&hir),
        0,
        "both additions fold before HIR, so only the ledger can prove the active use"
    );
}

#[test]
fn a_folded_fast_operation_still_projects_as_a_witness() {
    let source = "\
$fast_math
wrapped || -> U8:
    return 200 + 56
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "wrapped");

    assert_eq!(facts.constraint.fast_use.len(), 1);
    assert_eq!(
        facts.constraint.fast_use[0].provenance,
        HirFastUseProvenance::AuthoredArithmetic
    );
    assert_eq!(
        source_text(
            source,
            facts.constraint.fast_use[0]
                .span
                .expect("folded fast operations retain the authored operator span")
        ),
        "+"
    );
    assert_eq!(
        fast_statement_count(&hir),
        0,
        "the fast addition folded to a constant and has no HIR operation"
    );
}

#[test]
fn nested_template_range_ordinals_have_one_authored_witness_each() {
    let source = "\
card #= [:<div>[$slot title]</div>]
$fast_math
render |limit I32, show Bool| -> String:
    return [: [if show: [loop 0 to limit |item, ordinal|:[ordinal]]] [card:[$in title: [loop 0 to limit |item, ordinal|:[ordinal]]]]]
;
";
    let (hir, paths, strings) = lower_source(source);
    let facts = facts_by_name(&hir, &paths, &strings, "render");
    let guards: Vec<_> = facts
        .constraint
        .fast_use
        .iter()
        .filter(|witness| witness.provenance == HirFastUseProvenance::RangeGuard)
        .collect();
    assert_eq!(
        guards.len(),
        2,
        "each loop must have one deduplicated witness: {guards:?}"
    );
    for guard in guards {
        let span = guard
            .span
            .expect("template loops retain their authored span");
        assert!(source_text(source, span).contains("loop"));
    }
}

#[test]
fn a_generated_fast_range_guard_is_a_witness_without_authored_arithmetic() {
    let source = "\
$fast_math
guarded_range || -> I32:
    value ~= 0
    loop 0 to 3 by 1 |item|:
        ~value = item
    ;
    return value
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "guarded_range");

    assert!(
        !facts.constraint.fast_use.is_empty(),
        "a fast range guard selects fast semantics even with no authored arithmetic"
    );
    assert!(
        facts
            .constraint
            .fast_use
            .iter()
            .all(|witness| witness.provenance == HirFastUseProvenance::RangeGuard),
        "the only fast evidence is the generated guard: {:?}",
        facts.constraint.fast_use
    );
    assert!(
        hir.blocks
            .iter()
            .any(|block| block.statements.iter().any(|statement| {
                matches!(
                    statement.kind,
                    HirStatementKind::LoopStepFailure { semantics, .. } if semantics.is_fast()
                )
            })),
        "the projected witness must come from a retained fast guard"
    );
    let guard_span = facts.constraint.fast_use[0]
        .span
        .expect("the generated guard is attributed to its authored step position");
    assert_eq!(
        source_text(source, guard_span),
        "1",
        "the guard is attributed to the authored step position, not to its first body statement"
    );
}

#[test]
fn dec_arithmetic_under_fast_math_records_no_fast_witness() {
    let source = "\
$fast_math
decimal || -> Dec2:
    return 1.5 + 2.5
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "decimal");

    assert!(
        facts.constraint.fast_use.is_empty(),
        "Dec arithmetic never selects fast semantics, even under $fast_math: {:?}",
        facts.constraint.fast_use
    );
    assert_eq!(fast_statement_count(&hir), 0);
}

#[test]
fn discarded_inline_value_if_folded_fast_arithmetic_is_pruned() {
    let source = "\
$fast_math
inline_discard || -> I32:
    value = if false then 100 + 1 else 2
    return value
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "inline_discard");

    assert!(
        facts.constraint.fast_use.is_empty(),
        "a folded fast operation in the discarded inline value-if branch must not project: {:?}",
        facts.constraint.fast_use
    );
    assert!(
        facts
            .constraint
            .boundaries
            .iter()
            .all(|boundary| boundary.statement.is_none()),
        "the discarded inline branch must contribute no statement boundary: {:?}",
        facts.constraint.boundaries
    );
}

#[test]
fn inline_value_if_selected_branch_evidence_survives_shared_scope_pruning() {
    let source = "\
$fast_math
inline || -> I32:
    value = if true then 100 + 1 else 2
    return value
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "inline");

    assert_eq!(
        facts.constraint.fast_use.len(),
        1,
        "the selected inline branch keeps its folded fast-operation witness: {:?}",
        facts.constraint.fast_use
    );
    assert_eq!(
        facts.constraint.fast_use[0].provenance,
        HirFastUseProvenance::AuthoredArithmetic
    );
}

#[test]
fn selected_call_sites_keep_every_span_for_one_target() {
    let source = "\
calm |value I32| -> I32:
    return value
;
$fast_math
witness |left I32, right I32| -> Bool:
    marker = left + right
    return marker > 0
;
probe |value I32| -> Bool:
    early = witness(value, 2)
    $safe_math
    result = calm(value) < calm(1) catch |error|:
        return witness(value, 1)
    ;
    return result
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "probe");

    assert_eq!(
        facts.constraint.selected_call_sites.len(),
        4,
        "every direct call site in selected code must be retained: {:?}",
        facts.constraint.selected_call_sites
    );
    let witness_sites = facts
        .constraint
        .selected_call_sites
        .iter()
        .filter(|site| {
            site.span
                .is_some_and(|span| source_text(source, span).starts_with("witness"))
        })
        .count();
    assert_eq!(
        witness_sites, 2,
        "both witness call sites must survive target deduplication: {:?}",
        facts.constraint.selected_call_sites
    );
}

#[test]
fn skipped_handler_range_guard_is_selected_evidence() {
    let source = "\
$fast_math
literal_handler || -> I32:
    result I32 = 10 // 2 catch |error|:
        loop 0 to 3 by 1:
            io.line(\"x\")
        ;
        return 0
    ;
    return result
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "literal_handler");

    let guard = facts
        .constraint
        .fast_use
        .iter()
        .find(|witness| witness.provenance == HirFastUseProvenance::RangeGuard)
        .expect("the handler is statically infallible, so only the AST walk can see its guard");
    let span = guard
        .span
        .expect("the skipped loop keeps its authored loop statement span");
    assert_eq!(
        source_text(source, span),
        "loop",
        "the skipped guard is attributed to the loop header it belongs to"
    );
}

#[test]
fn statements_after_a_terminator_keep_their_calls_and_range_guards() {
    let source = "\
$fast_math
witness |value I32| -> I32:
    return value + 1
;
$fast_math $safe_math
dead_after_return |limit I32| -> I32:
    return 0
    value I32 = witness(1)
    loop 0 to limit by 1:
        io.line(\"x\")
    ;
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "dead_after_return");

    assert!(
        facts.constraint.selected_call_sites.iter().any(|site| {
            site.span
                .is_some_and(|span| source_text(source, span).starts_with("witness"))
        }),
        "a call in dead statements is still a selected call site: {:?}",
        facts.constraint.selected_call_sites
    );
    let guard = facts
        .constraint
        .fast_use
        .iter()
        .find(|witness| witness.provenance == HirFastUseProvenance::RangeGuard)
        .expect("a fast loop after a terminator still selects a fast range guard");
    let span = guard
        .span
        .expect("the skipped loop keeps its authored loop statement span");
    assert_eq!(
        source_text(source, span),
        "loop",
        "the skipped guard is attributed to the loop header it belongs to"
    );
}

#[test]
fn skipped_range_without_generated_checks_selects_no_guard_evidence() {
    // WHAT: a `$fast_math` loop after a terminator, with no authored step and no ordinal binding.
    // WHY: lowering emits no zero guard, no float no-progress guard and no ordinal update for this
    //      shape, and the skipped-source walker must use the same rule. Recording a guard here
    //      would reject `$safe_math` for a loop that is accepted when it is live.
    let source = "\
$fast_math
dead_unstepped |limit I32| -> I32:
    return 0
    loop 0 to limit:
        io.line(\"x\")
    ;
;
";
    let (hir, path_fork, string_table) = lower_source(source);
    let facts = facts_by_name(&hir, &path_fork, &string_table, "dead_unstepped");

    assert!(
        !facts
            .constraint
            .fast_use
            .iter()
            .any(|witness| witness.provenance == HirFastUseProvenance::RangeGuard),
        "a skipped loop with no generated range check must select no guard: {:?}",
        facts.constraint.fast_use
    );
}
