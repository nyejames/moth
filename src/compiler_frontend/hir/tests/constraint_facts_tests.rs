//! Post-lane constraint analysis over lowered HIR.
//!
//! WHAT: proves the liveness-conditioned trap/fail walk, the bounded literal proof and the
//!       selected fast-use projection independently of borrow validation or lane installation.
//! WHY: phase B and verification share this analyzer; a regression here would silently widen or
//!      narrow every summary bit, which end-to-end output tests cannot isolate.

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::numeric_operators::NumericSemantics;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::constraint_facts::analysis::{
    FunctionConstraintAnalysis, failure_destination_in_region,
};
use crate::compiler_frontend::hir::constraint_facts::{ConstraintCalleeFacts, analyze_function};
use crate::compiler_frontend::hir::failure_facts::HirFastUseProvenance;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::LoopStepFailureCause;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatement};
use crate::compiler_frontend::hir::terminators::{
    HirTerminator, NO_MATCH_RUNTIME_FAILURE_MESSAGE, RuntimeFailureCause, RuntimeFailureKind,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanResolutionSources,
};
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

fn analyze_named(source: &str, name: &str) -> FunctionConstraintAnalysis {
    let (hir, path_fork, string_table) = lower_source(source);
    let function = function_id_by_name(&hir, &path_fork, &string_table, name);
    analyze_function(&hir, function, |_| {
        ConstraintCalleeFacts::from_bits(Default::default())
    })
}

fn single_function_hir(terminator: HirTerminator) -> HirModule {
    let mut module = HirModule::new();
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.blocks = vec![HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator,
    }];
    module.functions = vec![HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
    }];
    module.start_function = Some(FunctionId(0));
    module
}

#[test]
fn literal_division_is_total_before_lane_installation() {
    let analysis = analyze_named(
        "halve |value I32| -> I32:\n    return value // 2\n;\n",
        "halve",
    );
    assert!(!analysis.bits.may_trap);
    assert!(!analysis.bits.may_fail);
    assert!(!analysis.bits.uses_fast_semantics);
    assert_eq!(analysis.trap_witness, None);
}

#[test]
fn an_unproved_divisor_keeps_a_statement_trap() {
    let analysis = analyze_named(
        "halve |value I32, divisor I32| -> I32:\n    return value // divisor\n;\n",
        "halve",
    );
    assert!(analysis.bits.may_trap);
    assert!(!analysis.bits.may_fail);
}

#[test]
fn a_reachable_assertion_is_a_trap() {
    let analysis = analyze_named(
        "check || -> I32:\n    assert(false, \"no\")\n    return 1\n;\n",
        "check",
    );
    assert!(analysis.bits.may_trap);
    assert!(!analysis.bits.may_fail);
}

#[test]
fn the_exhaustive_match_fallback_is_not_a_trap() {
    let hir = single_function_hir(HirTerminator::RuntimeFailure {
        message: NO_MATCH_RUNTIME_FAILURE_MESSAGE.to_owned(),
        cause: None,
        kind: RuntimeFailureKind::NoMatchFallback,
    });
    let analysis = analyze_function(&hir, FunctionId(0), |_| {
        ConstraintCalleeFacts::from_bits(Default::default())
    });
    assert!(
        !analysis.bits.may_trap,
        "the defensive exhaustive-match stop is semantically unreachable"
    );
}

#[test]
fn an_unrecovered_conversion_runtime_failure_is_a_trap() {
    let hir = single_function_hir(HirTerminator::RuntimeFailure {
        message: "Cast conversion failed".to_owned(),
        cause: Some(RuntimeFailureCause::StoreConversion { error: LocalId(0) }),
        kind: RuntimeFailureKind::Conversion,
    });
    let analysis = analyze_function(&hir, FunctionId(0), |_| {
        ConstraintCalleeFacts::from_bits(Default::default())
    });
    assert!(analysis.bits.may_trap);
    assert_eq!(
        analysis.trap_witness.map(|witness| witness.kind),
        Some(crate::compiler_frontend::hir::constraint_facts::analysis::ConstraintWitnessKind::TrapConversion)
    );
}

#[test]
fn a_trapping_range_guard_statement_is_a_trap() {
    let mut hir = single_function_hir(HirTerminator::Return(Default::default()));
    hir.blocks[0].statements.push(HirStatement {
        id: crate::compiler_frontend::hir::ids::HirNodeId(0),
        kind: crate::compiler_frontend::hir::statements::HirStatementKind::LoopStepFailure {
            cause: LoopStepFailureCause::ZeroStep,
            semantics: NumericSemantics::Ordinary,
            result: HirLocalDestination::Define(LocalId(0)),
        },
        span: None,
    });
    let analysis = analyze_function(&hir, FunctionId(0), |_| {
        ConstraintCalleeFacts::from_bits(Default::default())
    });
    assert!(analysis.bits.may_trap);
}

#[test]
fn an_infallible_fast_store_conversion_is_fast_evidence() {
    let (hir, path_fork, string_table) = lower_source(
        "$fast_math\n\
         narrow |value I32| -> I8:\n\
             total ~I8 = 0\n\
             ~total += value\n\
             return total\n\
         ;\n",
    );
    let function = function_id_by_name(&hir, &path_fork, &string_table, "narrow");
    let facts = &hir.function_failure_facts[&function];
    assert!(
        facts
            .constraint
            .fast_use
            .iter()
            .any(|witness| { witness.provenance == HirFastUseProvenance::StoreConversion }),
        "a total integer wrap must still project as fast use: {:?}",
        facts.constraint.fast_use
    );
    let analysis = analyze_function(&hir, function, |_| {
        ConstraintCalleeFacts::from_bits(Default::default())
    });
    assert!(analysis.bits.uses_fast_semantics);
    assert!(
        !analysis.bits.may_trap,
        "an integer fast write-back wraps and never traps"
    );
}

#[test]
fn an_unproved_signed_division_records_both_remaining_causes() {
    let analysis = analyze_named(
        "halve |value I32, divisor I32| -> I32:\n    return value // divisor\n;\n",
        "halve",
    );
    let witness = analysis
        .trap_witness
        .expect("an unproved divisor is a trap witness");
    assert!(witness.trap_causes.contains(BuiltinErrorCode::DivideByZero));
    assert!(witness.trap_causes.contains(BuiltinErrorCode::IntOverflow));
}

#[test]
fn a_binary_float_product_records_the_non_finite_cause() {
    let analysis = analyze_named(
        "scale |left F64, right F64| -> F64:\n    return left * right\n;\n",
        "scale",
    );
    let witness = analysis
        .trap_witness
        .expect("a binary-float product is a trap witness");
    assert!(
        witness
            .trap_causes
            .contains(BuiltinErrorCode::FloatNonFinite)
    );
}

/// Build an inline source span that needs no live source database.
fn marked_span_range(builder: &mut ExtendedSpanBuilder, start: u32, length: u32) -> SourceSpan {
    SourceSpan::new(
        SourceId::from_index(3),
        LocalSpan::exact(start, length, builder).expect("test span should fit inline"),
    )
}

#[test]
fn spanless_failure_adapter_follows_jump_to_in_region_handler() {
    let mut hir = HirModule::new();
    let mut spans = ExtendedSpanBuilder::new();
    let statement_start = marked_span_range(&mut spans, 10, 20);
    let handler_span = marked_span_range(&mut spans, 15, 4);
    let statement_end = marked_span_range(&mut spans, 30, 5);

    let adapter = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: Vec::new(),
        statements: Vec::new(),
        terminator: HirTerminator::Jump {
            target: BlockId(1),
            args: Vec::new(),
        },
    };
    let handler = HirBlock {
        id: BlockId(1),
        region: RegionId(0),
        locals: Vec::new(),
        statements: Vec::new(),
        terminator: HirTerminator::Uninitialized,
    };
    hir.side_table.map_block(Some(handler_span), &handler);
    hir.blocks.push(adapter);
    hir.blocks.push(handler);

    assert!(
        failure_destination_in_region(
            &hir,
            BlockId(0),
            &SpanResolutionSources::new(None, None),
            statement_start,
            statement_end,
        ),
        "the spanless adapter must inherit its in-region handler's containment",
    );
}
