//! Unit coverage for bounded const execution and allocation accounting.

use super::budget::{
    ExecutionBudget, MAX_CALL_DEPTH, MAX_RETAINED_VALUES, MAX_TOTAL_RETAINED_VALUES,
    charge_new_value_text, value_own_allocation,
};
use super::eligibility::{
    ReleaseCandidate, ReleaseCandidateProof, candidate_dependency_order, eligible_acyclic_paths,
    prepare_release_executable, retain_acyclic_release_candidates, reverse_call_graph,
};
use super::instructions::{ConstInstruction, ConstInstructionKind};
use super::rewrite::{RewriteMode, inherit_replaced_provenance};
use super::state::{ActiveHandler, ConstExecutionServices, ExecutionState, FrameBindings};
use super::*;
use crate::compiler_frontend::ast::const_eval::{
    ConstBudgetCategory, ConstBudgetUnit, ConstantFoldError,
};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolved_operation;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, OwnedExpr, PlaceRecord,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::module_ast::environment::declaration_table::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::ContextKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionParameter, FunctionSignature, ParameterContract,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::datatypes::FunctionReturnContract;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::module_compilation::DEFAULT_CONST_EXECUTION_MEMORY_LIMIT_BYTES;
use crate::compiler_frontend::source::{FrozenIdentityHandle, LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::cell::RefCell;
use std::sync::Arc;

/// A module scope with no declarations, enough for preparation and rewrite tests.
fn empty_module_scope() -> ScopeContext {
    ScopeContext::new(
        ContextKind::Module,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::empty()),
        Arc::new(ExternalPackageRegistry::new()),
        0,
        Rc::new(RefCell::new(TemplateIrStore::default())),
    )
}

/// A prepared const executable with no catch bodies or donor domain.
fn const_executable(
    parameters: Vec<(PathId, TypeId)>,
    return_slots: Vec<TypeId>,
    instructions: Vec<ConstInstruction>,
) -> ConstFunctionExecutable {
    ConstFunctionExecutable {
        parameters,
        return_slots,
        instructions: Rc::from(instructions),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    }
}

/// A release candidate row with no parameters and one `I32` success slot.
fn release_candidate(
    ordinary_calls: impl IntoIterator<Item = PathId>,
    provenance: SyntheticInterfaceProvenance,
) -> ReleaseCandidate {
    ReleaseCandidate {
        executable: const_executable(
            Vec::new(),
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            Vec::new(),
        ),
        proof: ReleaseCandidateProof {
            ordinary_calls: ordinary_calls.into_iter().collect(),
            body_provenance: provenance,
        },
    }
}

/// One release rewrite of a no-argument call to a literal-returning const callee.
struct ReleaseCallRewrite {
    builder: AstExpressionBuilder,
    authored: ExprId,
    rewritten: ExprId,
    callee: PathId,
    erased: Vec<(PathId, SourceSpan)>,
}

/// Rewrite one ordinary call to a literal-returning const callee in release mode.
///
/// WHAT: prepares the callee, attaches project-context provenance and runs the release walk.
/// WHY: the span-present and spanless cases differ only in the call's authored span, so one
///      fixture keeps the producer invariant and the provenance transfer on the same shape.
fn release_rewrite_of_literal_call(call_span: Option<SourceSpan>) -> ReleaseCallRewrite {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let returned = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("returned literal")
        .id();
    let callee = PathId::try_from_index(4).expect("callee path");
    let executable = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![ConstInstruction {
            kind: ConstInstructionKind::Return(vec![returned]),
            span: None,
        }]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    };
    let mut executables = FxHashMap::default();
    executables.insert(callee, executable);
    let provenance = SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "project",
        "name",
    ));
    let mut release_provenances = FxHashMap::default();
    release_provenances.insert(callee, provenance);
    let authored = builder
        .function_call_with_typed_arguments(callee, vec![], vec![i32_type], call_span)
        .expect("ordinary call")
        .id();
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let mut state = ExecutionState::default();
    let types = TypeEnvironment::default();
    let rewritten = {
        let mut budget = ExecutionBudget::new(constants.execution_meter());
        let mut services = ConstExecutionServices {
            builder: &mut builder,
            constants: &mut constants,
            executables: &executables,
            scope: &scope,
            string_table: &mut strings,
            types: &types,
            path_fork: &paths,
        };
        let mut interpreter = services.interpreter(&mut budget, &mut state);
        interpreter
            .rewrite_with_bindings(
                authored,
                RewriteMode::Release,
                &release_provenances,
                &mut FrameBindings::default(),
            )
            .expect("the literal-returning ordinary call is release-eligible")
    };
    ReleaseCallRewrite {
        builder,
        authored,
        rewritten,
        callee,
        erased: state.release_erased_calls,
    }
}

/// A short authored span at `offset`, distinguishable from the empty source-start span.
fn inline_span(offset: u32, length: u32) -> SourceSpan {
    SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact_inline(offset, length).expect("a short authored span is inline-encodable"),
    )
}

#[test]
fn aggregate_meter_stops_work_across_budget_attempts() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    let total = meter.borrow().limits.max_total_steps;
    meter.borrow_mut().steps = total - 1;
    let mut first = ExecutionBudget::new(Rc::clone(&meter));
    assert!(first.step().is_ok());
    let mut second = ExecutionBudget::new(meter);
    assert!(
        second.step().is_err(),
        "a second attempt must consume the shared aggregate allowance"
    );
}

#[test]
fn unrelated_expression_discovery_does_not_consume_the_execution_meter() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let meter = constants.execution_meter();
    let total = meter.borrow().limits.max_total_steps;
    meter.borrow_mut().steps = total;
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let expression = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("literal")
        .id();
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let rewritten = rewrite_required_const_calls(
        ConstExecutionServices {
            builder: &mut builder,
            constants: &mut constants,
            executables: &FxHashMap::default(),
            scope: &scope,
            string_table: &mut strings,
            types: &TypeEnvironment::default(),
            path_fork: &paths,
        },
        expression,
    )
    .expect("an ordinary expression with no const call is not const-execution work");
    assert_eq!(rewritten, expression);
}

#[test]
fn aggregate_text_byte_meter_stops_growth_across_attempts() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    let total = meter.borrow().limits.max_total_memory_bytes;
    meter.borrow_mut().retained_memory_bytes = total - 1;
    let mut first = ExecutionBudget::new(Rc::clone(&meter));
    assert!(first.retain_memory_bytes(1, None).is_ok());
    let mut second = ExecutionBudget::new(meter);
    assert!(
        second.retain_memory_bytes(1, None).is_err(),
        "a second attempt must consume the shared aggregate text allowance"
    );
}

#[test]
fn aggregate_retained_value_meter_reports_the_module_scope() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    meter.borrow_mut().retained_values = MAX_TOTAL_RETAINED_VALUES - 1;
    let mut first = ExecutionBudget::new(Rc::clone(&meter));
    assert!(
        first.retain_values(1, None).is_ok(),
        "the last piece inside the module allowance is allowed"
    );
    let mut second = ExecutionBudget::new(meter);
    let error = second
        .retain_values(1, None)
        .expect_err("a second attempt must consume the shared aggregate retained-value allowance");
    let ConstantFoldError::Budget(exhaustion) = error else {
        panic!("an aggregate retained-value stop is a budget refusal");
    };
    assert!(
        exhaustion.aggregate,
        "a module-wide retained-value stop must not be reported as one call"
    );
    assert_eq!(exhaustion.limit, MAX_TOTAL_RETAINED_VALUES as u64);
}

#[test]
fn shared_text_is_charged_once_through_its_owning_row() {
    let mut strings = StringTable::new();
    let text = strings.intern("abcdef");
    let mut constants = ConstValueStore::default();
    let string = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(text)),
            None,
        )
        .expect("string");
    let collection = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Collection(vec![string, string]),
            None,
        )
        .expect("collection");
    let string_charge = value_own_allocation(&constants, &strings, string);
    let collection_charge = value_own_allocation(&constants, &strings, collection);
    assert_eq!(string_charge.1, 1);
    assert_eq!(collection_charge.1, 3);
    assert!(
        collection_charge.0 >= 2 * std::mem::size_of::<ConstValueId>(),
        "the collection's owned item slots are charged"
    );
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    let mut budget = ExecutionBudget::new(meter);
    charge_new_value_text(&constants, &strings, 0, &mut budget)
        .expect("the first charge includes both existing rows");
    assert_eq!(
        budget.retained_memory_bytes,
        string_charge.0 + collection_charge.0
    );
    assert_eq!(budget.retained_values, 4);
    // A nested fold may capture an earlier `start`; the attempt high-water mark prevents the
    // string row from being charged again when a later parent is appended.
    let collection = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Collection(vec![string]),
            None,
        )
        .expect("second collection");
    let later_collection = value_own_allocation(
        &constants,
        &strings,
        constants.value_id_at(2).expect("third row"),
    );
    charge_new_value_text(&constants, &strings, 0, &mut budget)
        .expect("the later parent row fits without recharging the shared string");
    assert_eq!(
        budget.retained_memory_bytes,
        string_charge.0 + collection_charge.0 + later_collection.0
    );
    assert_eq!(budget.retained_values, 6);
    assert!(constants.payload(collection).is_some());
}

#[test]
fn precharged_mutation_rows_are_not_charged_again_at_the_boundary() {
    let mut strings = StringTable::new();
    let text = strings.intern("abcdef");
    let mut constants = ConstValueStore::default();
    let string = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(text)),
            None,
        )
        .expect("string");
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    let mut budget = ExecutionBudget::new(meter);
    budget.precharged_value_indices.insert(0);
    budget.retained_values = 1;
    budget.retained_memory_bytes = 6;
    charge_new_value_text(&constants, &strings, 0, &mut budget).expect("precharged row fits");
    assert_eq!(budget.retained_values, 1);
    assert_eq!(budget.retained_memory_bytes, 6);
    assert!(matches!(
        constants.payload(string),
        Some(ConstValuePayload::String(ConstStringValue::Text(_)))
    ));
}

#[test]
fn structural_piece_growth_consumes_the_shared_value_allowance() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let text = strings.intern("resource");
    let pieces = vec![
        ConstStringPiece::Text(text),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::SiteRoot,
    ];
    constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
            None,
        )
        .expect("structural string");
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    meter.borrow_mut().retained_values = MAX_TOTAL_RETAINED_VALUES;
    let mut budget = ExecutionBudget::new(meter);
    assert!(
        charge_new_value_text(&constants, &strings, 0, &mut budget).is_err(),
        "retained structural pieces must consume the shared allocation meter"
    );
}

#[test]
fn structural_piece_vectors_are_charged_as_bytes() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let text = strings.intern("resource");
    let pieces = vec![
        ConstStringPiece::Text(text),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::SiteRoot,
    ];
    let string = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
            None,
        )
        .expect("structural string");
    let (bytes, retained) = value_own_allocation(&constants, &strings, string);
    assert_eq!(retained, 4);
    assert!(bytes >= 3 * std::mem::size_of::<ConstStringPiece>());
}

#[test]
fn template_work_spends_the_executor_step_allowance() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    {
        let mut meter = meter.borrow_mut();
        meter.limits.max_attempt_steps = 4;
        meter.limits.max_total_steps = 4;
    }
    let mut budget = ExecutionBudget::new(meter);
    for _ in 0..3 {
        budget.step().expect("executor steps inside the allowance");
    }

    let work = budget.template_work(None);
    work.meter
        .borrow_mut()
        .charge_step(&work.attempt_steps, None)
        .expect("the shared fourth step still fits");
    let refusal = work
        .meter
        .borrow_mut()
        .charge_step(&work.attempt_steps, None)
        .expect_err("the shared allowance is exhausted at the fifth step");

    assert_eq!(work.attempt_steps.get(), 5);
    assert_eq!(refusal.category, ConstBudgetCategory::Work);
    assert_eq!(refusal.unit, ConstBudgetUnit::Steps);
    assert_eq!(refusal.limit, 4);
    assert_eq!(refusal.setting, Some("const_work_limit"));
    assert!(!refusal.aggregate);
}

#[test]
fn template_output_limits_are_the_remaining_attempt_allowance() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    let mut budget = ExecutionBudget::new(meter);
    budget.retained_memory_bytes = 11;
    budget.retained_values = 13;
    let limits = budget.template_output_limits(None);
    assert_eq!(
        limits.max_bytes,
        DEFAULT_CONST_EXECUTION_MEMORY_LIMIT_BYTES - 11
    );
    assert_eq!(limits.max_structural_pieces, MAX_RETAINED_VALUES - 13);
}

#[test]
fn call_stack_is_the_call_depth_owner() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    let mut budget = ExecutionBudget::new(meter);
    for _ in 0..MAX_CALL_DEPTH {
        budget
            .enter_call(None, None)
            .expect("depth below the limit is accepted");
    }
    assert!(budget.enter_call(None, None).is_err());
    budget.leave_call();
    assert_eq!(budget.call_stack.len(), MAX_CALL_DEPTH - 1);
}

/// A hard depth refusal must resolve the primary and every call-chain owner in the source that
/// authored each site, not in the body that happens to be executing when the bound trips.
#[test]
fn call_depth_refusal_keeps_each_mixed_donor_owner() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let root_to_a_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact_inline(10, 18).expect("root call span"),
    );
    let a_to_b_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact_inline(110, 118).expect("package A call span"),
    );
    let b_to_a_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact_inline(210, 218).expect("package B call span"),
    );
    let a_path = PathId::try_from_index(10).expect("package A path");
    let b_path = PathId::try_from_index(11).expect("package B path");
    let call_a_from_root = builder
        .function_call_with_typed_arguments(a_path, vec![], vec![i32_type], Some(root_to_a_span))
        .expect("project call into package A")
        .id();
    let call_b_from_a = builder
        .function_call_with_typed_arguments(b_path, vec![], vec![i32_type], Some(a_to_b_span))
        .expect("package A call into package B")
        .id();
    let call_a_from_b = builder
        .function_call_with_typed_arguments(a_path, vec![], vec![i32_type], Some(b_to_a_span))
        .expect("package B call into package A")
        .id();
    let a_donor = FrozenIdentityHandle::new();
    let b_donor = FrozenIdentityHandle::new();
    let root_caller = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![ConstInstruction {
            kind: ConstInstructionKind::Return(vec![call_a_from_root]),
            span: None,
        }]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    };
    let a_executable = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![ConstInstruction {
            kind: ConstInstructionKind::Return(vec![call_b_from_a]),
            span: None,
        }]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: Some(a_donor.clone()),
    };
    let b_executable = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![ConstInstruction {
            kind: ConstInstructionKind::Return(vec![call_a_from_b]),
            span: None,
        }]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: Some(b_donor.clone()),
    };
    let mut executables = FxHashMap::default();
    executables.insert(a_path, a_executable);
    executables.insert(b_path, b_executable);

    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let mut state = ExecutionState::default();
    let types = TypeEnvironment::default();
    let mut budget = ExecutionBudget::new(constants.execution_meter());
    let mut services = ConstExecutionServices {
        builder: &mut builder,
        constants: &mut constants,
        executables: &executables,
        scope: &scope,
        string_table: &mut strings,
        types: &types,
        path_fork: &paths,
    };
    let mut interpreter = services.interpreter(&mut budget, &mut state);
    let error = interpreter
        .execute_body(None, &root_caller, FrameBindings::default())
        .expect_err("a call chain deeper than the hard bound must be refused");
    let ConstantFoldError::Budget(exhaustion) = error else {
        panic!("the depth bound must stay a budget refusal");
    };
    assert_eq!(exhaustion.category, ConstBudgetCategory::Work);
    assert_eq!(exhaustion.unit, ConstBudgetUnit::CallFrames);
    assert_eq!(exhaustion.limit, MAX_CALL_DEPTH as u64);
    assert_eq!(
        exhaustion.span,
        Some(b_to_a_span),
        "the refused call site belongs to the executing package B body"
    );
    assert_eq!(
        exhaustion.span_owner.as_ref(),
        Some(&b_donor),
        "the innermost provider body owns the refusal primary"
    );
    assert_eq!(exhaustion.call_span, Some(b_to_a_span));
    assert_eq!(
        exhaustion.call_span_owner.as_ref(),
        Some(&b_donor),
        "the refused package B call site keeps its own owner"
    );
    assert_eq!(
        exhaustion.call_chain.len(),
        MAX_CALL_DEPTH + 1,
        "every active site plus the refused site keeps a label"
    );

    for (index, site) in exhaustion.call_chain.iter().enumerate() {
        let (expected_span, expected_owner) = match index {
            0 => (root_to_a_span, None),
            index if index % 2 == 1 => (a_to_b_span, Some(&a_donor)),
            _ => (b_to_a_span, Some(&b_donor)),
        };
        assert_eq!(
            site.span, expected_span,
            "call site {index} must keep its authored span"
        );
        assert_eq!(
            site.owner.as_ref(),
            expected_owner,
            "call site {index} must resolve in its caller's source domain"
        );
    }
    let refused_site = exhaustion
        .call_chain
        .last()
        .expect("the refused site is labelled with the active chain");
    assert_eq!(refused_site.span, b_to_a_span);
    assert_eq!(refused_site.owner.as_ref(), Some(&b_donor));
}

/// A step refused after a provider call returns must point at the caller's call site and carry
/// the caller's donor, not the callee's span under the caller's owner.
#[test]
fn budget_refusal_after_nested_provider_call_keeps_the_callers_span_and_donor() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let caller_to_callee_span = inline_span(10, 8);
    let callee_return_span = inline_span(110, 8);
    let callee_literal_span = inline_span(210, 8);
    let callee_path = PathId::try_from_index(10).expect("callee path");
    let call_callee = builder
        .function_call_with_typed_arguments(
            callee_path,
            vec![],
            vec![i32_type],
            Some(caller_to_callee_span),
        )
        .expect("caller call into the callee package")
        .id();
    let callee_literal = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("I32 literal"),
            Some(callee_literal_span),
            ValueMode::ImmutableOwned,
        )
        .expect("callee return literal")
        .id();
    let caller_donor = FrozenIdentityHandle::new();
    let callee_donor = FrozenIdentityHandle::new();
    let caller = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![
            ConstInstruction {
                kind: ConstInstructionKind::Eval(call_callee),
                span: Some(caller_to_callee_span),
            },
            ConstInstruction {
                // Never reached: the step charged to enter it is the refusal under test.
                kind: ConstInstructionKind::Return(vec![callee_literal]),
                span: None,
            },
        ]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: Some(caller_donor.clone()),
    };
    let callee = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![ConstInstruction {
            kind: ConstInstructionKind::Return(vec![callee_literal]),
            span: Some(callee_return_span),
        }]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: Some(callee_donor),
    };
    let mut executables = FxHashMap::default();
    executables.insert(callee_path, callee);

    // The caller's entry, the callee's body and both completed-call folds fit; the caller's
    // next step is refused.
    let meter = constants.execution_meter();
    {
        let mut meter = meter.borrow_mut();
        meter.limits.max_attempt_steps = 4;
        meter.limits.max_total_steps = 4;
    }
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let mut state = ExecutionState::default();
    let types = TypeEnvironment::default();
    let mut budget = ExecutionBudget::new(meter);
    let mut services = ConstExecutionServices {
        builder: &mut builder,
        constants: &mut constants,
        executables: &executables,
        scope: &scope,
        string_table: &mut strings,
        types: &types,
        path_fork: &paths,
    };
    let mut interpreter = services.interpreter(&mut budget, &mut state);
    let error = interpreter
        .execute_body(None, &caller, FrameBindings::default())
        .expect_err("the caller's next step exceeds the allowance");
    let ConstantFoldError::Budget(exhaustion) = error else {
        panic!("the step allowance must stay a budget refusal");
    };
    assert_eq!(exhaustion.category, ConstBudgetCategory::Work);
    assert_eq!(exhaustion.unit, ConstBudgetUnit::Steps);
    assert_eq!(
        exhaustion.span,
        Some(caller_to_callee_span),
        "the refused step belongs to the caller's call site, not the callee it returned from"
    );
    assert_eq!(
        exhaustion.span_owner.as_ref(),
        Some(&caller_donor),
        "the caller's donor must own the caller's span"
    );
}

/// An aggregate refusal on the provider body's first step must use the provider's entry span and
/// donor, never the caller's call span under the provider's identity.
#[test]
fn budget_refusal_on_provider_entry_uses_the_provider_span_and_donor() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let caller_call_span = inline_span(30, 8);
    let provider_entry_span = inline_span(330, 8);
    let provider_literal_span = inline_span(430, 8);
    let provider_path = PathId::try_from_index(11).expect("provider path");
    let call_provider = builder
        .function_call_with_typed_arguments(
            provider_path,
            vec![],
            vec![i32_type],
            Some(caller_call_span),
        )
        .expect("caller call into the provider")
        .id();
    let provider_literal = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 3).expect("I32 literal"),
            Some(provider_literal_span),
            ValueMode::ImmutableOwned,
        )
        .expect("provider return literal")
        .id();
    let caller_donor = FrozenIdentityHandle::new();
    let provider_donor = FrozenIdentityHandle::new();
    let caller = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![
            ConstInstruction {
                kind: ConstInstructionKind::Eval(call_provider),
                span: Some(caller_call_span),
            },
            ConstInstruction {
                kind: ConstInstructionKind::Return(vec![provider_literal]),
                span: None,
            },
        ]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: Some(caller_donor),
    };
    let provider = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![ConstInstruction {
            kind: ConstInstructionKind::Return(vec![provider_literal]),
            span: Some(provider_entry_span),
        }]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: Some(provider_donor.clone()),
    };
    let mut executables = FxHashMap::default();
    executables.insert(provider_path, provider);

    // The caller's entry spends the aggregate exactly; the provider's first frame is the step
    // that must be refused.
    let meter = constants.execution_meter();
    {
        let mut meter = meter.borrow_mut();
        meter.limits.max_attempt_steps = 100;
        meter.limits.max_total_steps = 1;
    }
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let mut state = ExecutionState::default();
    let types = TypeEnvironment::default();
    let mut budget = ExecutionBudget::new(meter);
    let mut services = ConstExecutionServices {
        builder: &mut builder,
        constants: &mut constants,
        executables: &executables,
        scope: &scope,
        string_table: &mut strings,
        types: &types,
        path_fork: &paths,
    };
    let mut interpreter = services.interpreter(&mut budget, &mut state);
    let error = interpreter
        .execute_body(None, &caller, FrameBindings::default())
        .expect_err("the provider's entry step exceeds the aggregate");
    let ConstantFoldError::Budget(exhaustion) = error else {
        panic!("the step allowance must stay a budget refusal");
    };
    assert!(
        exhaustion.aggregate,
        "the refusal must charge the aggregate"
    );
    assert_eq!(
        exhaustion.span,
        Some(provider_entry_span),
        "the provider's entry step must use the provider's entry span"
    );
    assert_eq!(
        exhaustion.span_owner.as_ref(),
        Some(&provider_donor),
        "the provider's donor must own the provider's entry span"
    );
}

#[test]
fn release_candidate_graph_propagates_ineligibility_in_one_pass() {
    let mut graph: FxHashMap<PathId, FxHashSet<PathId>> = FxHashMap::default();
    let nodes: Vec<PathId> = (0..10_000)
        .map(|index| PathId::try_from_index(index).expect("test path"))
        .collect();
    for pair in nodes.windows(2) {
        graph.insert(pair[0], [pair[1]].into_iter().collect());
    }
    graph.insert(
        *nodes.last().expect("non-empty chain"),
        [PathId::try_from_index(20_000).expect("missing path")]
            .into_iter()
            .collect(),
    );
    assert!(
        eligible_acyclic_paths(&graph, &reverse_call_graph(&graph)).is_empty(),
        "one missing dependency must invalidate the whole predecessor chain"
    );
}

#[test]
fn release_candidate_graph_removes_cyclic_components() {
    let a = PathId::try_from_index(0).expect("path a");
    let b = PathId::try_from_index(1).expect("path b");
    let dependent = PathId::try_from_index(2).expect("path dependent");
    let independent = PathId::try_from_index(3).expect("path independent");
    let mut graph: FxHashMap<PathId, FxHashSet<PathId>> = FxHashMap::default();
    graph.insert(a, [b].into_iter().collect());
    graph.insert(b, [a].into_iter().collect());
    graph.insert(dependent, [a].into_iter().collect());
    graph.insert(independent, FxHashSet::default());
    let eligible = eligible_acyclic_paths(&graph, &reverse_call_graph(&graph));
    assert!(!eligible.contains(&a) && !eligible.contains(&b));
    assert!(!eligible.contains(&dependent));
    assert!(eligible.contains(&independent));
}

#[test]
fn release_call_replacement_inherits_candidate_provenance() {
    let span = inline_span(12, 20);
    let rewrite = release_rewrite_of_literal_call(Some(span));
    let facts = rewrite
        .builder
        .view(rewrite.rewritten)
        .expect("rewritten call")
        .occurrence_facts()
        .expect("replacement provenance");
    assert!(
        facts
            .provenance
            .contains_class(SyntheticInterfaceClass::ProjectContext),
        "the release replacement must inherit the candidate's transitive provenance"
    );
    assert_eq!(
        rewrite.erased,
        vec![(rewrite.callee, span)],
        "release rewriting must record the erased call site with its authored span"
    );
}

/// Release speculation must never erase a call whose liveness cannot be decided.
///
/// WHAT: a spanless ordinary call stays at runtime and records no erased-call row.
/// WHY: HIR decides whether an erased call was live from authored source spans. Guessing on an
///      absent span could reject `$infallible` source that debug accepts, breaking debug/release
///      verdict parity. No legal source or materialised callable produces a spanless call today,
///      so this unit test pins the producer invariant that the required ledger span now encodes.
#[test]
fn release_rewrite_declines_a_spanless_call() {
    let rewrite = release_rewrite_of_literal_call(None);
    assert_eq!(
        rewrite.rewritten, rewrite.authored,
        "a spanless call must stay at runtime in release"
    );
    assert!(
        matches!(
            rewrite
                .builder
                .view(rewrite.rewritten)
                .expect("retained call")
                .operation(),
            ExpressionOperationView::Call(_)
        ),
        "the retained expression must still be the authored call"
    );
    assert!(
        rewrite.erased.is_empty(),
        "a spanless call must not enter the erased-call ledger: {:?}",
        rewrite.erased
    );
}

/// A refused release candidate must discard the folds it speculated in its arguments.
///
/// WHAT: a two-argument call with one non-constant argument and one foldable ordinary call stays
///       byte-for-byte the authored runtime call, and no erased-call row survives.
/// WHY: a debug build evaluates every argument as authored. Keeping the rewritten argument under
///      a call that stays at runtime would let an optimiser-created constant reach HIR and the
///      numeric proof without the runtime call debug retains, changing `$infallible` verdicts in
///      release only.
#[test]
fn release_refused_call_discards_speculative_argument_folds() {
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");

    let returned = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("returned literal")
        .id();
    let foldable = PathId::try_from_index(4).expect("foldable path");
    let keep = PathId::try_from_index(5).expect("keep path");
    let first_parameter = PathId::try_from_index(6).expect("first parameter");
    let second_parameter = PathId::try_from_index(7).expect("second parameter");
    let runtime = PathId::try_from_index(8).expect("runtime callee");

    let mut executables = FxHashMap::default();
    executables.insert(
        foldable,
        const_executable(
            Vec::new(),
            vec![i32_type],
            vec![ConstInstruction {
                kind: ConstInstructionKind::Return(vec![returned]),
                span: None,
            }],
        ),
    );
    let returned_parameter = builder
        .reference_with_type_id(
            first_parameter,
            i32_type,
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("parameter reference")
        .id();
    executables.insert(
        keep,
        const_executable(
            vec![(first_parameter, i32_type), (second_parameter, i32_type)],
            vec![i32_type],
            vec![ConstInstruction {
                kind: ConstInstructionKind::Return(vec![returned_parameter]),
                span: None,
            }],
        ),
    );

    let keep_span = inline_span(40, 30);
    let foldable_call = builder
        .function_call_with_typed_arguments(
            foldable,
            Vec::<CallArgument<OwnedExpr>>::new(),
            vec![i32_type],
            Some(inline_span(12, 20)),
        )
        .expect("foldable call")
        .id();
    let runtime_call = builder
        .function_call_with_typed_arguments(
            runtime,
            Vec::<CallArgument<OwnedExpr>>::new(),
            vec![i32_type],
            Some(keep_span),
        )
        .expect("runtime call")
        .id();
    let first_argument = builder.own(runtime_call);
    let second_argument = builder.own(foldable_call);
    let authored = builder
        .function_call_with_typed_arguments(
            keep,
            vec![
                CallArgument::positional(first_argument, CallAccessMode::Shared, None),
                CallArgument::positional(second_argument, CallAccessMode::Shared, None),
            ],
            vec![i32_type],
            Some(keep_span),
        )
        .expect("runtime keep call")
        .id();

    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let types = TypeEnvironment::default();
    let release_provenances = FxHashMap::default();
    let mut state = ExecutionState::default();
    let rewritten = {
        let mut budget = ExecutionBudget::new(constants.execution_meter());
        let mut services = ConstExecutionServices {
            builder: &mut builder,
            constants: &mut constants,
            executables: &executables,
            scope: &scope,
            string_table: &mut strings,
            types: &types,
            path_fork: &paths,
        };
        let mut interpreter = services.interpreter(&mut budget, &mut state);
        interpreter
            .rewrite_with_bindings(
                authored,
                RewriteMode::Release,
                &release_provenances,
                &mut FrameBindings::default(),
            )
            .expect("release rewriting leaves the runtime call in place")
    };

    assert_eq!(
        rewritten, authored,
        "a refused candidate must leave the runtime call byte-for-byte as authored"
    );
    assert!(
        state.release_erased_calls.is_empty(),
        "speculative argument folds must not enter the erased-call ledger: {:?}",
        state.release_erased_calls
    );
}

/// A refused release candidate must discard an auxiliary fold that leaves no ledger row.
///
/// WHAT: an eligible return-second call proves that a constant collection's `.length()` folds
///       inside release speculation, then a structurally identical call with one runtime argument
///       refuses and must restore the authored `.length()` child byte-for-byte.
/// WHY: an ordinary argument fold records an erased-call row, but a collection read is a genuine
///      auxiliary fold that records none. Keeping it under a call that stays at runtime is
///      therefore invisible to erased-call evidence while still changing the AST a debug build
///      would evaluate.
#[test]
fn release_refused_call_discards_an_auxiliary_collection_length_fold() {
    use crate::compiler_frontend::builtins::CollectionBuiltinOp;

    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut types = TypeEnvironment::new();
    let collection_type = types.intern_collection(i32_type, None);
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let callee = paths
        .try_intern_portable_path("test/callee", &mut strings)
        .expect("callee path");
    let first_parameter = paths
        .try_intern_portable_path("test/first", &mut strings)
        .expect("first parameter");
    let second_parameter = paths
        .try_intern_portable_path("test/second", &mut strings)
        .expect("second parameter");
    let missing_runtime = paths
        .try_intern_portable_path("test/missing_runtime", &mut strings)
        .expect("missing runtime callee");
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");

    let item = constants
        .insert_resolved(
            i32_type,
            ConstValuePayload::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 1).expect("I32 one"),
            ),
            None,
        )
        .expect("collection item");
    let collection = constants
        .insert_resolved(
            collection_type,
            ConstValuePayload::Collection(vec![item, item, item]),
            None,
        )
        .expect("constant collection");

    let returned_parameter = builder
        .reference_with_type_id(
            second_parameter,
            i32_type,
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("parameter reference")
        .id();
    let mut executables = FxHashMap::default();
    executables.insert(
        callee,
        const_executable(
            vec![(first_parameter, i32_type), (second_parameter, i32_type)],
            vec![i32_type],
            vec![ConstInstruction {
                kind: ConstInstructionKind::Return(vec![returned_parameter]),
                span: None,
            }],
        ),
    );

    let length_span = inline_span(40, 20);
    let make_length = |builder: &mut AstExpressionBuilder| {
        let receiver = builder
            .constant(
                collection,
                collection_type,
                Some(length_span),
                ValueMode::ImmutableOwned,
            )
            .expect("collection receiver");
        builder
            .collection_builtin_call_with_typed_arguments(
                receiver,
                CollectionBuiltinOp::Length,
                Vec::<CallArgument<OwnedExpr>>::new(),
                vec![i32_type],
                Some(length_span),
            )
            .expect("length call")
            .id()
    };
    let constant_argument = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("I32 seven"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("constant argument")
        .id();
    let eligible_length = make_length(&mut builder);
    let eligible = builder
        .function_call_with_typed_arguments(
            callee,
            vec![
                CallArgument::positional(
                    builder.own(constant_argument),
                    CallAccessMode::Shared,
                    None,
                ),
                CallArgument::positional(
                    builder.own(eligible_length),
                    CallAccessMode::Shared,
                    None,
                ),
            ],
            vec![i32_type],
            Some(inline_span(140, 40)),
        )
        .expect("eligible call")
        .id();

    let runtime_call = builder
        .function_call_with_typed_arguments(
            missing_runtime,
            Vec::<CallArgument<OwnedExpr>>::new(),
            vec![i32_type],
            Some(inline_span(90, 10)),
        )
        .expect("runtime call")
        .id();
    let refused_length = make_length(&mut builder);
    let refused = builder
        .function_call_with_typed_arguments(
            callee,
            vec![
                CallArgument::positional(builder.own(runtime_call), CallAccessMode::Shared, None),
                CallArgument::positional(builder.own(refused_length), CallAccessMode::Shared, None),
            ],
            vec![i32_type],
            Some(inline_span(140, 40)),
        )
        .expect("refused call")
        .id();

    let scope = empty_module_scope();
    let release_provenances = FxHashMap::default();
    let mut state = ExecutionState::default();
    let eligible_rewritten = {
        let mut budget = ExecutionBudget::new(constants.execution_meter());
        let mut services = ConstExecutionServices {
            builder: &mut builder,
            constants: &mut constants,
            executables: &executables,
            scope: &scope,
            string_table: &mut strings,
            types: &types,
            path_fork: &paths,
        };
        let mut interpreter = services.interpreter(&mut budget, &mut state);
        interpreter
            .rewrite_with_bindings(
                eligible,
                RewriteMode::Release,
                &release_provenances,
                &mut FrameBindings::default(),
            )
            .expect("the constant-argument call folds")
    };
    let ExpressionOperationView::Constant(folded) = builder
        .view(eligible_rewritten)
        .expect("folded view")
        .operation()
    else {
        panic!("an eligible call returning its second constant argument must fold");
    };
    assert!(
        matches!(
            constants.payload(folded),
            Some(ConstValuePayload::FixedScalar(value))
                if *value == FixedScalarValue::signed(FixedScalar::I32, 3).expect("I32 three")
        ),
        "the folded value must come through the auxiliary collection-length fold"
    );

    let erased_before_refusal = state.release_erased_calls.len();
    let refused_rewritten = {
        let mut budget = ExecutionBudget::new(constants.execution_meter());
        let mut services = ConstExecutionServices {
            builder: &mut builder,
            constants: &mut constants,
            executables: &executables,
            scope: &scope,
            string_table: &mut strings,
            types: &types,
            path_fork: &paths,
        };
        let mut interpreter = services.interpreter(&mut budget, &mut state);
        interpreter
            .rewrite_with_bindings(
                refused,
                RewriteMode::Release,
                &release_provenances,
                &mut FrameBindings::default(),
            )
            .expect("a runtime argument leaves the call in place")
    };
    assert_eq!(
        refused_rewritten, refused,
        "a refused candidate must restore the whole authored call"
    );
    assert_eq!(
        state.release_erased_calls.len(),
        erased_before_refusal,
        "a refused candidate must not add a row for the auxiliary child"
    );
    let ExpressionOperationView::Call(call) = builder
        .view(refused_rewritten)
        .expect("refused view")
        .operation()
    else {
        panic!("the refused call must remain an authored call");
    };
    let length_argument = call.arguments().iter().nth(1).expect("second argument");
    assert!(
        matches!(
            length_argument.value().operation(),
            ExpressionOperationView::CollectionBuiltin { .. }
        ),
        "the auxiliary length read must stay authored under the refused call"
    );
}

/// An abandoned rewrite must not leave the catch handler it installed on the caller's budget.
///
/// This is not observable from Moth source today: a failing handler body aborts the const attempt
/// instead of being recovered by an outer handler, and every caller creates a fresh attempt
/// budget. The invariant still protects callers that recover or reuse a budget, so a direct
/// budget-state assertion is the honest place to pin it.
#[test]
fn failed_rewrite_restores_the_handler_it_replaced() {
    use crate::compiler_frontend::ast::expressions::expression_store::SuccessShape;
    use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
    use crate::compiler_frontend::ast::statements::value_production::types::{
        ValueBlock, ValueCatchBlock,
    };

    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let returned = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("returned literal")
        .id();
    let instructions: Rc<[ConstInstruction]> = Rc::from(vec![ConstInstruction {
        kind: ConstInstructionKind::Return(vec![returned]),
        span: None,
    }]);
    let callee = PathId::try_from_index(4).expect("callee path");
    let executable = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::clone(&instructions),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    };
    let mut executables = FxHashMap::default();
    executables.insert(callee, executable);
    let call = builder
        .function_call_with_typed_arguments(callee, vec![], vec![i32_type], None)
        .expect("ordinary call")
        .id();
    let catch = builder
        .value_block(
            ValueBlock::Catch(ValueCatchBlock {
                handled_value: call,
                handler: FallibleHandling::Handler {
                    error: None,
                    body: Vec::new(),
                },
                result_type_ids: vec![i32_type],
            }),
            None,
            SuccessShape::One(i32_type),
            ValueMode::ImmutableOwned,
        )
        .expect("catch expression")
        .id();
    let root = builder
        .coerced(builder.own(catch), i32_type)
        .expect("catch holder")
        .id();
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();

    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    meter.borrow_mut().limits.max_attempt_steps = 0;
    let mut budget = ExecutionBudget::new(Rc::clone(&meter));
    let mut state = ExecutionState::default();
    let saved_handler: Rc<[ConstInstruction]> = Rc::from(vec![ConstInstruction {
        kind: ConstInstructionKind::Return(vec![returned]),
        span: None,
    }]);
    let saved_error_binding = PathId::try_from_index(8).expect("error binding path");
    state.active_handler = ActiveHandler {
        body: Some(Rc::clone(&saved_handler)),
        error_binding: Some(saved_error_binding),
        boundary: true,
    };
    let mut handler_bodies = FxHashMap::default();
    handler_bodies.insert(catch, Rc::clone(&instructions));
    state.handler_bodies = Some(Rc::new(handler_bodies));
    let types = TypeEnvironment::default();
    let mut services = ConstExecutionServices {
        builder: &mut builder,
        constants: &mut constants,
        executables: &executables,
        scope: &scope,
        string_table: &mut strings,
        types: &types,
        path_fork: &paths,
    };
    let mut interpreter = services.interpreter(&mut budget, &mut state);

    let error = interpreter
        .rewrite_with_bindings(
            root,
            RewriteMode::InterpretedBody,
            &FxHashMap::default(),
            &mut FrameBindings::default(),
        )
        .expect_err("the zero attempt allowance refuses the nested call");
    assert!(matches!(error, ConstantFoldError::Budget(_)));
    assert!(
        state
            .active_handler
            .body
            .as_ref()
            .is_some_and(|handler| Rc::ptr_eq(handler, &saved_handler)),
        "the rewrite must restore the handler it replaced before returning the error"
    );
    assert_eq!(
        state.active_handler.error_binding,
        Some(saved_error_binding),
        "the rewrite must restore the replaced error binding"
    );
    assert!(
        state.active_handler.boundary,
        "the rewrite must restore the replaced boundary flag"
    );
}

#[test]
fn replaced_child_provenance_reaches_the_surviving_parent() {
    use crate::compiler_frontend::synthetic_interface_provenance::{
        SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
    };

    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let child = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("child literal")
        .id();
    let parent = builder
        .coerced(builder.own(child), builtin_type_ids::STRING)
        .expect("parent expression")
        .id();
    let replacement = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 2).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("replacement literal")
        .id();
    let inherited = SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "replacement",
    ));
    let replacement = builder
        .with_synthetic_interface_provenance(builder.own(replacement), inherited)
        .expect("replacement provenance")
        .id();

    let rewritten =
        inherit_replaced_provenance(&mut builder, parent, parent, &[(child, replacement)])
            .expect("parent should survive replacement");

    let facts = builder
        .view(rewritten)
        .expect("rewritten parent")
        .occurrence_facts()
        .expect("rewritten parent provenance");
    assert!(
        facts
            .provenance
            .contains_class(SyntheticInterfaceClass::ProjectContext),
        "a folded child's project-context dependency must survive on its parent"
    );
}

#[test]
fn nested_scalar_calls_execute_once_per_loop_iteration() {
    use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeEndKind, RangeLoopSpec};
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

    const ITERATIONS: i64 = 16;
    const CALLEE_RESULT: i64 = 7;

    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let returned = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, CALLEE_RESULT).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("returned literal")
        .id();
    let returned_value =
        values::constant_operation(&builder, returned, &mut strings).expect("returned constant");
    let callee = PathId::try_from_index(4).expect("callee path");
    let callee_executable = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![
            ConstInstruction {
                kind: ConstInstructionKind::BindValue {
                    path: PathId::try_from_index(5).expect("callee binding"),
                    value: returned_value,
                },
                span: None,
            },
            ConstInstruction {
                kind: ConstInstructionKind::Return(vec![returned]),
                span: None,
            },
        ]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    };
    let mut executables = FxHashMap::default();
    executables.insert(callee, callee_executable);

    let call = builder
        .function_call_with_typed_arguments(callee, vec![], vec![i32_type], None)
        .expect("nested const call");
    let start = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 0).expect("range start"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("range start")
        .id();
    let end = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, ITERATIONS).expect("range end"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("range end")
        .id();
    // The running total is read through a place so the sum depends on the value the previous
    // iteration left behind as well as this iteration's call result.
    let accumulator = PathId::try_from_index(6).expect("accumulator path");
    let no_projections = builder
        .append_place_projections(Vec::new(), None)
        .expect("no projections");
    let accumulator_place = builder
        .append_place(
            PlaceRecord {
                root: accumulator,
                root_type_id: i32_type,
                projections: no_projections,
                type_id: i32_type,
                value_mode: ValueMode::ImmutableOwned,
                span: None,
            },
            None,
        )
        .expect("accumulator place");
    let accumulator_read = builder
        .copy_with_type_id(accumulator_place, i32_type, None, ValueMode::ImmutableOwned)
        .expect("accumulator read")
        .id();
    let Ok(addition) = resolved_operation(
        Operator::Add,
        None,
        i32_type,
        i32_type,
        Some(i32_type),
        &TypeEnvironment::default(),
        false,
    ) else {
        panic!("the I32 addition resolves");
    };
    let accumulator_operand = builder.own(accumulator_read);
    let accumulated = builder
        .resolved_operation(
            accumulator_operand,
            Some(call),
            addition,
            ValueMode::ImmutableOwned,
        )
        .expect("accumulated total")
        .id();
    let loop_body = Rc::from(vec![ConstInstruction {
        kind: ConstInstructionKind::Bind {
            path: accumulator,
            expression: accumulated,
            value_mode: ValueMode::ImmutableOwned,
        },
        span: None,
    }]);
    let caller = ConstFunctionExecutable {
        parameters: Vec::new(),
        return_slots: vec![i32_type],
        instructions: Rc::from(vec![
            ConstInstruction {
                kind: ConstInstructionKind::Bind {
                    path: accumulator,
                    expression: start,
                    value_mode: ValueMode::ImmutableOwned,
                },
                span: None,
            },
            ConstInstruction {
                kind: ConstInstructionKind::Range {
                    bindings: Box::new(LoopBindings {
                        item: None,
                        index: None,
                    }),
                    range: Box::new(RangeLoopSpec {
                        start,
                        end,
                        end_kind: RangeEndKind::Exclusive,
                        step: None,
                        domain: NumericScalar::Fixed(FixedScalar::I32),
                        fast_math: false,
                    }),
                    body: loop_body,
                    span: None,
                },
                span: None,
            },
            ConstInstruction {
                kind: ConstInstructionKind::Return(vec![accumulator_read]),
                span: None,
            },
        ]),
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    };

    let meter = constants.execution_meter();
    let mut budget = ExecutionBudget::new(Rc::clone(&meter));
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let mut state = ExecutionState::default();
    let types = TypeEnvironment::default();
    let mut services = ConstExecutionServices {
        builder: &mut builder,
        constants: &mut constants,
        executables: &executables,
        scope: &scope,
        string_table: &mut strings,
        types: &types,
        path_fork: &paths,
    };
    let mut interpreter = services.interpreter(&mut budget, &mut state);
    let returns = interpreter
        .execute_body(None, &caller, FrameBindings::default())
        .expect("a loop of nested scalar calls executes");
    assert_eq!(returns.len(), 1);
    assert!(
        matches!(
            constants.payload(returns[0]),
            Some(ConstValuePayload::FixedScalar(value))
                if value.as_i64() == Some(ITERATIONS * CALLEE_RESULT)
        ),
        "each iteration must add the callee's returned scalar to the running total"
    );
    assert!(
        meter.borrow().steps > ITERATIONS as usize,
        "the nested call body must execute once per iteration"
    );
}

#[test]
fn fold_to_constant_propagates_budget_refusals() {
    let mut strings = StringTable::new();
    let mut constants = ConstValueStore::default();
    let meter = constants.execution_meter();
    meter.borrow_mut().limits.max_attempt_steps = 0;
    let mut builder = AstExpressionBuilder::new().expect("builder");
    let expression = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("I32 literal"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("literal")
        .id();
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    let executables = FxHashMap::default();
    let types = TypeEnvironment::default();
    let mut budget = ExecutionBudget::new(meter);
    let mut state = ExecutionState::default();
    let mut services = ConstExecutionServices {
        builder: &mut builder,
        constants: &mut constants,
        executables: &executables,
        scope: &scope,
        string_table: &mut strings,
        types: &types,
        path_fork: &paths,
    };
    let mut interpreter = services.interpreter(&mut budget, &mut state);
    let error = interpreter
        .fold_to_constant(expression, &FrameBindings::default())
        .expect_err("a free fold must escape its budget refusal");
    assert!(
        matches!(error, ConstantFoldError::Budget(_)),
        "fold_to_constant must not turn a refusal into `not constant`"
    );
}

#[test]
fn release_preparation_refuses_signatures_it_cannot_represent() {
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let aggregate_type = builtin_type_ids::STRING;
    let parameter = |value_mode, type_id| FunctionParameter {
        id: PathId::try_from_index(20).expect("parameter path"),
        type_id,
        value_mode,
        default: None,
        binding_span: None,
        contract: ParameterContract::Value,
    };
    let scalar_contract = || FunctionReturnContract::from_slots([i32_type], None);

    // Every refusal below happens before the body compiler runs, so an empty body is enough to
    // reach the gate being pinned.
    let cases = [
        (
            "a mutable parameter",
            vec![parameter(ValueMode::MutableOwned, i32_type)],
            scalar_contract(),
            false,
        ),
        (
            "an aggregate parameter",
            vec![parameter(ValueMode::ImmutableOwned, aggregate_type)],
            scalar_contract(),
            false,
        ),
        (
            "a scalar parameter and scalar result",
            vec![parameter(ValueMode::ImmutableOwned, i32_type)],
            scalar_contract(),
            true,
        ),
        (
            "zero success slots",
            Vec::new(),
            FunctionReturnContract::from_slots([], None),
            false,
        ),
        (
            "two success slots",
            Vec::new(),
            FunctionReturnContract::from_slots([i32_type, i32_type], None),
            false,
        ),
        (
            "an aggregate success slot",
            Vec::new(),
            FunctionReturnContract::from_slots([aggregate_type], None),
            false,
        ),
        (
            "an error channel",
            Vec::new(),
            FunctionReturnContract::from_slots([i32_type], Some(aggregate_type)),
            false,
        ),
        (
            "a Never contract",
            Vec::new(),
            FunctionReturnContract::Never,
            false,
        ),
    ];

    let constants = ConstValueStore::default();
    let builder = AstExpressionBuilder::new().expect("builder");
    let types = TypeEnvironment::default();
    let registry = ExternalPackageRegistry::new();
    let scope = empty_module_scope();
    let paths = PathInternerFork::empty();
    for (label, parameters, return_contract, expected) in cases {
        let signature = FunctionSignature {
            parameters,
            return_contract,
        };
        let references = ConstBodyReferenceScope {
            scope: &scope,
            path_fork: &paths,
            constants: &constants,
        };
        let prepared =
            prepare_release_executable(&signature, &[], &builder, &types, &registry, references)
                .expect("signature gates never report infrastructure failures");
        assert_eq!(
            prepared.is_some(),
            expected,
            "{label} must {} a release candidate",
            if expected { "prepare" } else { "refuse" }
        );
    }
}

#[test]
fn retained_release_candidates_merge_transitive_provenance() {
    let callee = PathId::try_from_index(0).expect("callee path");
    let caller = PathId::try_from_index(1).expect("caller path");
    let callee_provenance =
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "render",
            "callee",
        ));
    let caller_provenance =
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "render",
            "caller",
        ));
    let mut candidates = FxHashMap::default();
    candidates.insert(callee, release_candidate([], callee_provenance.clone()));
    candidates.insert(
        caller,
        release_candidate([callee], caller_provenance.clone()),
    );

    let retained = retain_acyclic_release_candidates(candidates);

    let expected = caller_provenance.union(&callee_provenance);
    assert_eq!(
        retained.provenances.get(&caller),
        Some(&expected),
        "a caller's published provenance must include its callee's transitive set"
    );
    assert_eq!(
        retained.provenances.get(&callee),
        Some(&callee_provenance),
        "a callee keeps its own provenance"
    );
    assert_eq!(retained.executables.len(), 2);
}

#[test]
fn retained_release_candidates_remove_the_dependent_closure_of_a_missing_callee() {
    let caller = PathId::try_from_index(0).expect("caller path");
    let callee = PathId::try_from_index(1).expect("callee path");
    let independent = PathId::try_from_index(2).expect("independent path");
    let missing = PathId::try_from_index(3).expect("missing path");
    let independent_provenance =
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "render",
            "independent",
        ));
    let mut candidates = FxHashMap::default();
    candidates.insert(
        caller,
        release_candidate([callee], SyntheticInterfaceProvenance::default()),
    );
    candidates.insert(
        callee,
        release_candidate([missing], SyntheticInterfaceProvenance::default()),
    );
    candidates.insert(
        independent,
        release_candidate([], independent_provenance.clone()),
    );

    let retained = retain_acyclic_release_candidates(candidates);

    assert_eq!(
        retained.executables.keys().copied().collect::<Vec<_>>(),
        vec![independent],
        "the dependent closure of a missing callee must be removed"
    );
    assert_eq!(
        retained.provenances.get(&independent),
        Some(&independent_provenance),
        "an unreachable candidate keeps its provenance"
    );
    assert!(!retained.provenances.contains_key(&caller));
    assert!(!retained.provenances.contains_key(&callee));
}

#[test]
fn dependency_order_finishes_shared_callee_before_its_caller() {
    let caller = PathId::try_from_index(0).expect("caller path");
    let caller_sibling = PathId::try_from_index(1).expect("sibling path");
    let shared_callee = PathId::try_from_index(2).expect("shared callee path");
    let mut graph: FxHashMap<PathId, FxHashSet<PathId>> = FxHashMap::default();
    graph.insert(
        caller,
        [caller_sibling, shared_callee].into_iter().collect(),
    );
    graph.insert(caller_sibling, [shared_callee].into_iter().collect());
    graph.insert(shared_callee, FxHashSet::default());
    let eligible: FxHashSet<PathId> = graph.keys().copied().collect();

    let order = candidate_dependency_order(&graph, &eligible);
    let position = |path: PathId| {
        order
            .iter()
            .position(|candidate| *candidate == path)
            .expect("every candidate is ordered")
    };
    assert!(
        position(shared_callee) < position(caller_sibling),
        "the shared callee must be visited before its caller"
    );
    assert!(
        position(caller_sibling) < position(caller),
        "the caller must be visited after its callee"
    );
}
