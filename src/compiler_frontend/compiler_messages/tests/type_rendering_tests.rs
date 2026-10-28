//! Unit tests for type-name rendering at the diagnostic boundary.
//!
//! WHAT: exercises production-captured type snapshots through frozen report views and renderers.
//! WHY: diagnostics carry `TypeId`s, so the boundary captures semantic type identity before the
//! renderer turns it into source-level names.

use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    self, InvalidFallibleHandlingCode, InvalidFallibleHandlingView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    self, InvalidGenericInstantiationView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    self, IncompatibleChoiceComparisonView, TypeMismatchView, UnsupportedOperatorTypesView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::terminal::format_guidance_view;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_compilation_result;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terminal, terse, unsupported_operator_types_message,
};
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, BuiltinFailureWitness, DiagnosticBag, DiagnosticOperator,
    GenericInferenceSubject, IncompatibleChoiceComparisonReason, InvalidAssignmentTargetReason,
    InvalidFallibleHandlingReason, InvalidFieldAccessReason, InvalidGenericInstantiationReason,
    InvalidReceiverDeclarationReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::display::{TypeDisplayShape, TypeDisplayView};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, GenericParameterId, NominalTypeId, TypeConstructor, TypeId,
    builtin_type_ids,
};
use crate::compiler_frontend::symbols::path_interner::{PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::sync::Arc;

fn capture_frozen_failure(
    diagnostics: Vec<DiagnosticDraft>,
    string_table: StringTable,
    type_environment: Option<&TypeEnvironment>,
    path_table: PathTable,
) -> CompilationResult<()> {
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(diagnostics),
        string_table,
        type_environment,
        Some(Arc::new(path_table)),
    );
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}

fn diagnosed_reports(
    result: &CompilationResult<()>,
) -> &crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        other => panic!("expected diagnosed result, got {other:?}"),
    }
}

fn frozen_type_names(
    type_ids: &[TypeId],
    string_table: StringTable,
    type_environment: Option<&TypeEnvironment>,
    path_table: PathTable,
) -> Vec<String> {
    let diagnostics = type_ids
        .iter()
        .map(|type_id| {
            types_borrows_targets::type_mismatch(
                *type_id,
                *type_id,
                TypeMismatchContext::Assignment,
                None,
            )
        })
        .collect();
    let failure = capture_frozen_failure(diagnostics, string_table, type_environment, path_table);

    diagnosed_reports(&failure)
        .iter()
        .map(|(report, view)| {
            let mismatch = view
                .family_view::<TypeMismatchView>()
                .expect("the frozen diagnostic must retain type-mismatch facts");
            let context = DiagnosticRenderContext::for_report(report);
            view.types().display_for_render(
                mismatch.expected,
                context.string_table,
                context.path_table(),
            )
        })
        .collect()
}

#[test]
fn frozen_snapshot_renders_builtin_type_names() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let type_names = frozen_type_names(
        &[
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            type_environment.builtins().string,
        ],
        string_table,
        Some(&type_environment),
        path_fork.snapshot_table(),
    );

    assert_eq!(type_names, vec!["I32", "String"]);
}

#[test]
fn rule_diagnostics_render_receiver_type_names() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let int_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let field_access = calls_data::invalid_field_access(
        InvalidFieldAccessReason::UnknownMember,
        None,
        Some(int_type),
        Vec::new(),
        None,
    );
    let assignment = names_declarations::invalid_assignment_target(
        InvalidAssignmentTargetReason::TemporaryNotAssignable,
        None,
        None,
        None,
        None,
    );
    let failure = capture_frozen_failure(
        vec![field_access, assignment],
        string_table,
        Some(&type_environment),
        path_fork.snapshot_table(),
    );
    let mut guidance = diagnosed_reports(&failure).iter().map(|(report, view)| {
        format_guidance_view(view, DiagnosticRenderContext::for_report(report))
    });
    let field_guidance = guidance.next().expect("field-access report");
    assert!(field_guidance.iter().any(|line| line.contains("'I32'")));

    let assignment_guidance = guidance.next().expect("assignment report");
    assert!(
        assignment_guidance
            .iter()
            .any(|line| line.contains("A temporary value cannot be assigned through"))
    );
}

#[test]
fn receiver_declaration_reason_renders_the_receiver_type_identity() {
    // WHAT: the payload carries the receiver type as semantic identity, so the rule message must
    //       name it through the captured type snapshot rather than an interned spelling.
    let mut type_environment = TypeEnvironment::new();
    let element_type = type_environment.builtins().string;
    let receiver_type = type_environment.intern_collection(element_type, None);
    let mut string_table = StringTable::new();
    let function_name = string_table.intern("describe");
    let path_fork = PathInternerFork::empty();

    let diagnostic = generics_traits_api::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name,
            type_id: receiver_type,
        },
        None,
    );
    let failure = capture_frozen_failure(
        vec![diagnostic],
        string_table,
        Some(&type_environment),
        path_fork.snapshot_table(),
    );
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("receiver report");
    let guidance = format_guidance_view(view, DiagnosticRenderContext::for_report(report));
    assert!(
        guidance
            .iter()
            .any(|line| line
                .contains("Function 'describe' uses unsupported receiver type '{String}'")),
        "receiver declaration message must render the receiver type identity: {guidance:?}"
    );
}

#[test]
fn failure_handling_renderers_resolve_custom_types_and_recovery_guidance() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut error_types = Vec::new();

    for name in ["ParseFailure", "StoreFailure"] {
        let path = path_fork
            .try_intern_portable_path(name, &mut string_table)
            .expect("test path fits");
        let (_, type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path,
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });
        error_types.push(type_id);
    }

    let path_table = path_fork.snapshot_table();
    let cases = [
        (
            InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                first_error_type_id: error_types[0],
                second_error_type_id: error_types[1],
                first_producer_span: None,
                second_producer_span: None,
            },
            vec![
                "`ParseFailure!`",
                "`StoreFailure!`",
                "convert the errors explicitly",
            ],
        ),
        (
            InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                error_type_id: error_types[0],
                typed_producer_span: None,
                implicit_producer_span: None,
            },
            vec![
                "`ParseFailure!`",
                "implicit built-in failure",
                "separately handled expressions",
            ],
        ),
        (
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                error_type_id: error_types[0],
                witness: Box::new(BuiltinFailureWitness {
                    call_spans: vec![None, None, None],
                    origin_span: None,
                    origin: BuiltinFailureOriginKind::Operation,
                    elided_call_hops: 2,
                }),
            },
            vec![
                "custom `ParseFailure!` slot",
                "explicitly to `ParseFailure`",
                "`return!`",
                "final error slot to `Error!`",
                "explicit invariant choice",
                "Witness omits 2 additional private call hop(s).",
            ],
        ),
        (
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                witness: Box::new(BuiltinFailureWitness {
                    call_spans: Vec::new(),
                    origin_span: None,
                    origin: BuiltinFailureOriginKind::Operation,
                    elided_call_hops: 0,
                }),
            },
            vec![
                "exported function",
                "Recover locally",
                "final Error! return slot",
                "explicit invariant choice",
            ],
        ),
        (
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
            vec![
                "Assertion messages must be ordinary values.",
                "cannot escape its evaluation",
            ],
        ),
    ];

    let (diagnostics, expected_fragments): (Vec<DiagnosticDraft>, Vec<Vec<&str>>) = cases
        .into_iter()
        .map(|(reason, expected_fragments)| {
            (
                choices_control_flow::invalid_fallible_handling(reason, None),
                expected_fragments,
            )
        })
        .unzip();
    let failure = capture_frozen_failure(
        diagnostics,
        string_table,
        Some(&type_environment),
        path_table,
    );
    let terse_lines = format_terse_compilation_result(&failure);

    let reports = diagnosed_reports(&failure);
    for ((expected_fragments, (report, view)), terse) in expected_fragments
        .iter()
        .zip(reports.iter())
        .zip(terse_lines.iter())
    {
        let fallible = view
            .family_view::<InvalidFallibleHandlingView>()
            .expect("the frozen diagnostic must retain fallible-handling facts");
        let guidance =
            format_guidance_view(view, DiagnosticRenderContext::for_report(report)).join("\n");
        if fallible.reason == InvalidFallibleHandlingCode::AssertionMessageCannotEscape {
            assert_eq!(view.durable_labels().count(), 0);
            assert!(!guidance.contains("explicit invariant choice"));
            assert!(!guidance.contains("Error!"));
        }
        if matches!(
            fallible.reason,
            InvalidFallibleHandlingCode::UnhandledBuiltinFailureInExportedFunction
                | InvalidFallibleHandlingCode::AssertionMessageCannotEscape,
        ) {
            assert!(!guidance.contains("Witness omits"));
        }
        for &fragment in expected_fragments {
            assert!(
                guidance.contains(fragment),
                "missing {fragment:?} in {guidance}"
            );
            assert!(terse.contains(fragment), "missing {fragment:?} in {terse}");
        }
    }
}

#[test]
fn frozen_snapshot_renders_nominal_struct_and_choice_names() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

    let point_path = path_fork
        .try_intern_portable_path("Point", &mut string_table)
        .expect("test path fits");
    let (_, point_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let status_path = path_fork
        .try_intern_portable_path("Status", &mut string_table)
        .expect("test path fits");
    let ready = string_table.get_or_intern("Ready".to_owned());
    let failed = string_table.get_or_intern("Failed".to_owned());
    let (_, status_type) = type_environment.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: status_path,
        variants: vec![
            ChoiceVariantDefinition {
                name: ready,
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Unit,
                span: None,
            },
            ChoiceVariantDefinition {
                name: failed,
                tag: 1,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: Box::new([]),
                },
                span: None,
            },
        ]
        .into_boxed_slice(),
        generic_parameters: None,
    });

    let type_names = frozen_type_names(
        &[point_type, status_type],
        string_table,
        Some(&type_environment),
        path_fork.snapshot_table(),
    );

    assert_eq!(type_names, vec!["Point", "Status::{Ready, Failed(...)}"]);
}

#[test]
fn frozen_snapshot_renders_constructed_type_names() {
    let mut type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let builtins = *type_environment.builtins();
    let i32_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let collection = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([i32_id]),
    );
    let option = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([builtins.string]),
    );

    let type_names = frozen_type_names(
        &[collection, option],
        string_table,
        Some(&type_environment),
        path_fork.snapshot_table(),
    );

    assert_eq!(type_names, vec!["{I32}", "String?"]);
}

#[test]
fn captured_unresolved_type_ids_preserve_their_raw_spelling() {
    let orphan_type = TypeId(999);
    let path_fork = PathInternerFork::empty();

    let no_environment_names = frozen_type_names(
        &[orphan_type],
        StringTable::new(),
        None,
        path_fork.snapshot_table(),
    );
    assert_eq!(no_environment_names, vec!["TypeId(999)"]);

    let type_environment = TypeEnvironment::new();
    let missing_type_names = frozen_type_names(
        &[orphan_type],
        StringTable::new(),
        Some(&type_environment),
        path_fork.snapshot_table(),
    );
    assert_eq!(missing_type_names, vec!["TypeId(999)"]);
}

#[test]
fn terse_type_mismatch_uses_captured_type_names() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let diagnostic = types_borrows_targets::type_mismatch(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        type_environment.builtins().string,
        TypeMismatchContext::Assignment,
        None,
    );
    let failure = capture_frozen_failure(
        vec![diagnostic],
        string_table,
        Some(&type_environment),
        path_fork.snapshot_table(),
    );
    let lines = format_terse_compilation_result(&failure);

    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("expected I32, found String"));
    assert!(!lines[0].contains("TypeId("));
}

#[test]
fn terse_type_mismatch_without_environment_prints_type_ids() {
    let path_fork = PathInternerFork::empty();
    let diagnostic = types_borrows_targets::type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        None,
    );
    let failure = capture_frozen_failure(
        vec![diagnostic],
        StringTable::new(),
        None,
        path_fork.snapshot_table(),
    );
    let lines = format_terse_compilation_result(&failure);

    assert_eq!(
        lines,
        vec![String::from(
            "E|MOTH-TYPE-0001|||Type mismatch in assignment: expected TypeId(999), found TypeId(1000)"
        )]
    );
}

#[test]
fn unsupported_operator_message_explains_unsigned_negation() {
    assert_eq!(
        unsupported_operator_message(
            DiagnosticOperator::Subtract,
            builtin_type_ids::fixed_scalar(FixedScalar::U8),
            None,
        ),
        "Unary `-` does not apply to unsigned `U8`. Convert the value to a signed type with `cast` first."
    );
}

#[test]
fn unsupported_operator_message_explains_missing_fixed_integer_common_type() {
    assert_eq!(
        unsupported_operator_message(
            DiagnosticOperator::Add,
            builtin_type_ids::fixed_scalar(FixedScalar::I64),
            Some(builtin_type_ids::fixed_scalar(FixedScalar::U64)),
        ),
        "`I64` and `U64` have no common integer type for `+`. Convert one operand with `cast` first."
    );
}

#[test]
fn unsupported_operator_message_explains_byte_arithmetic() {
    assert_eq!(
        unsupported_operator_message(
            DiagnosticOperator::Add,
            builtin_type_ids::fixed_scalar(FixedScalar::Byte),
            Some(builtin_type_ids::fixed_scalar(FixedScalar::Byte)),
        ),
        "`Byte` has no arithmetic operators."
    );

    assert_eq!(
        unsupported_operator_message(
            DiagnosticOperator::Subtract,
            builtin_type_ids::fixed_scalar(FixedScalar::Byte),
            None,
        ),
        "`Byte` has no arithmetic operators."
    );
}

fn unsupported_operator_message(
    operator: DiagnosticOperator,
    lhs: TypeId,
    rhs: Option<TypeId>,
) -> String {
    let type_environment = TypeEnvironment::new();
    let diagnostic = types_borrows_targets::unsupported_operator_types(operator, lhs, rhs, None);
    let path_fork = PathInternerFork::empty();
    let failure = capture_frozen_failure(
        vec![diagnostic],
        StringTable::new(),
        Some(&type_environment),
        path_fork.snapshot_table(),
    );
    let (report, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("unsupported-operator report");
    let facts = view
        .family_view::<UnsupportedOperatorTypesView>()
        .expect("the frozen diagnostic must retain operator type facts");
    unsupported_operator_types_message(
        facts.operator,
        facts.lhs,
        facts.rhs,
        view.types(),
        DiagnosticRenderContext::for_report(report),
    )
}

#[test]
fn type_mismatch_constructor_carries_type_facts_without_rendering() {
    let type_environment = TypeEnvironment::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::type_mismatch(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            type_environment.builtins().string,
            TypeMismatchContext::Declaration,
            None,
        )),
        Some(&type_environment),
    );
    let view = compact.first().expect("type mismatch compacts");
    let facts = view
        .family_view::<TypeMismatchView>()
        .expect("type mismatch facts");
    assert_eq!(facts.context, TypeMismatchContext::Declaration);
    assert_eq!(
        view.types().shape(facts.expected),
        TypeDisplayShape::FixedScalar(FixedScalar::I32)
    );
    assert_eq!(view.types().shape(facts.found), TypeDisplayShape::String);
}

#[test]
fn type_mismatch_terminal_guidance_renders_captured_type_names() {
    let string_table = StringTable::new();
    let path_table = PathInternerFork::empty().snapshot_table();
    let type_environment = TypeEnvironment::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::type_mismatch(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            type_environment.builtins().string,
            TypeMismatchContext::Declaration,
            None,
        )),
        Some(&type_environment),
    );
    let view = compact.first().expect("captured mismatch");
    let guidance = terminal::format_guidance_view(
        view,
        DiagnosticRenderContext::new(&string_table).with_path_table(&path_table),
    );
    assert!(guidance.iter().any(|line| line == "Expected: I32"));
    assert!(guidance.iter().any(|line| line == "Found: String"));
    assert!(!guidance.iter().any(|line| line.contains("type id")));
}

#[test]
fn type_mismatch_terse_renderer_renders_captured_type_names() {
    let string_table = StringTable::new();
    let path_table = PathInternerFork::empty().snapshot_table();
    let type_environment = TypeEnvironment::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::type_mismatch(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            type_environment.builtins().string,
            TypeMismatchContext::FunctionArgument,
            None,
        )),
        Some(&type_environment),
    );
    let view = compact.first().expect("captured mismatch");
    let line = terse::format_terse_diagnostic_view(
        view,
        DiagnosticRenderContext::new(&string_table).with_path_table(&path_table),
    );
    assert!(line.contains("expected I32"));
    assert!(line.contains("found String"));
    assert!(!line.contains("Expected type id"));
    assert!(!line.contains("Found type id"));
}

#[test]
fn incompatible_choice_comparison_renderer_hides_reason_debug_names() {
    let string_table = StringTable::new();
    let path_table = PathInternerFork::empty().snapshot_table();
    let type_environment = TypeEnvironment::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::incompatible_choice_comparison(
            IncompatibleChoiceComparisonReason::ChoiceWithNonChoice,
            builtin_type_ids::BOOL,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            None,
        )),
        Some(&type_environment),
    );
    let view = compact.first().expect("choice-comparison draft compacts");
    assert!(
        view.family_view::<IncompatibleChoiceComparisonView>()
            .is_some()
    );
    let context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);
    let guidance = terminal::format_guidance_view(view, context);
    let terse_line = terse::format_terse_diagnostic_view(view, context);
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Cannot compare choice")),
        "{guidance:?}"
    );
    assert!(terse_line.contains("Cannot compare choice"), "{terse_line}");
    assert!(
        !guidance
            .iter()
            .any(|line| line.contains("ChoiceWithNonChoice"))
    );
    assert!(!terse_line.contains("ChoiceWithNonChoice"));
}

#[test]
fn generic_instantiation_rendering_resolves_type_name() {
    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(generics_traits_api::invalid_generic_instantiation(
            Some(box_name),
            InvalidGenericInstantiationReason::WrongArgumentCount {
                expected: 1,
                found: 2,
            },
            None,
        )),
        None,
    );
    let view = compact.first().expect("generic diagnostic compacts");
    assert!(
        view.family_view::<InvalidGenericInstantiationView>()
            .is_some()
    );
    let terse_line =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::new(&string_table));
    assert!(
        terse_line.contains("'Box'"),
        "Expected 'Box' in message, got: {terse_line}"
    );
}

#[test]
fn generic_conflict_rendering_resolves_concrete_type_names() {
    let mut string_table = StringTable::new();
    let function_name = string_table.intern("first");
    let parameter_name = string_table.intern("T");
    let path_table = PathInternerFork::empty().snapshot_table();
    let type_environment = TypeEnvironment::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from(generics_traits_api::invalid_generic_instantiation(
            Some(function_name),
            InvalidGenericInstantiationReason::ConflictingInference {
                subject: GenericInferenceSubject::Function,
                parameter_id: GenericParameterId(0),
                parameter_name,
                existing_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                replacement_type_id: type_environment.builtins().string,
            },
            None,
        )),
        Some(&type_environment),
    );
    let view = compact.first().expect("generic conflict compacts");
    assert!(
        view.family_view::<InvalidGenericInstantiationView>()
            .is_some()
    );
    let guidance = terminal::format_guidance_view(
        view,
        DiagnosticRenderContext::new(&string_table).with_path_table(&path_table),
    );
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Generic parameter 'T'")
                && line.contains("I32")
                && line.contains("String")),
        "expected captured types, got: {guidance:?}"
    );
}
