//! Unit tests for type-name rendering at the diagnostic boundary.
//!
//! WHAT: exercises `DiagnosticRenderContext` rather than raw datatype display helpers.
//! WHY: diagnostics carry `TypeId`s, so the renderer is the contract that turns semantic type
//! identity into source-level names when a module `TypeEnvironment` is available.

use crate::compiler_frontend::compiler_messages::render::terminal::format_payload_guidance;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostics_with_context;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, diagnostic_type_name, unsupported_operator_types_message,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticOperator, InvalidAssignmentTargetReason,
    InvalidFallibleHandlingReason, InvalidFieldAccessReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, NominalTypeId, TypeConstructor, TypeId, builtin_type_ids,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn diagnostic_render_context_renders_builtin_type_names() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

    assert_eq!(
        diagnostic_type_name(type_environment.builtins().int, context),
        "Int"
    );
    assert_eq!(
        diagnostic_type_name(type_environment.builtins().string, context),
        "String"
    );
}

#[test]
fn rule_diagnostics_render_receiver_type_names() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);
    let int_type = type_environment.builtins().int;

    let field_access = CompilerDiagnostic::invalid_field_access(
        InvalidFieldAccessReason::UnknownMember,
        None,
        Some(int_type),
        Vec::new(),
        None,
    );
    let field_guidance = format_payload_guidance(&field_access.payload, context);
    assert!(field_guidance.iter().any(|line| line.contains("'Int'")));

    let assignment = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::TemporaryNotAssignable,
        None,
        Some(int_type),
        None,
        None,
        None,
        None,
    );
    let assignment_guidance = format_payload_guidance(&assignment.payload, context);
    assert!(
        assignment_guidance
            .iter()
            .any(|line| line.contains("A temporary value cannot be assigned through"))
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
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);
    let cases = [
        (
            InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                first_error_type_id: error_types[0],
                second_error_type_id: error_types[1],
                first_producer_span: None,
                second_producer_span: None,
            },
            vec!["`ParseFailure!`", "`StoreFailure!`", "convert the errors explicitly"],
        ),
        (
            InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                error_type_id: error_types[0],
                typed_producer_span: None,
                implicit_producer_span: None,
            },
            vec!["`ParseFailure!`", "implicit built-in failure", "separately handled expressions"],
        ),
        (
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                error_type_id: error_types[0],
                implicit_producer_span: None,
            },
            vec!["custom `ParseFailure!` slot", "explicitly to `ParseFailure`", "`return!`"],
        ),
        (
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction,
            vec!["exported function", "Recover locally", "final Error! return slot"],
        ),
    ];

    for (reason, expected_fragments) in cases {
        let diagnostic = CompilerDiagnostic::invalid_fallible_handling(reason, None);
        let guidance = format_payload_guidance(&diagnostic.payload, context).join("\n");
        let terse = format_terse_diagnostics_with_context(&[diagnostic], context).join("\n");
        for fragment in expected_fragments {
            assert!(guidance.contains(fragment), "missing {fragment:?} in {guidance}");
            assert!(terse.contains(fragment), "missing {fragment:?} in {terse}");
        }
    }
}

#[test]
fn diagnostic_render_context_renders_nominal_struct_and_choice_names() {
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

    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

    assert_eq!(diagnostic_type_name(point_type, context), "Point");
    assert_eq!(
        diagnostic_type_name(status_type, context),
        "Status::{Ready, Failed(...)}"
    );
}

#[test]
fn diagnostic_render_context_renders_constructed_type_names() {
    let mut type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let builtins = *type_environment.builtins();
    let collection = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([builtins.int]),
    );
    let option = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([builtins.string]),
    );
    let result = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([builtins.int, builtins.string]),
    );

    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

    assert_eq!(diagnostic_type_name(collection, context), "{Int}");
    assert_eq!(diagnostic_type_name(option, context), "String?");
    assert_eq!(diagnostic_type_name(result, context), "Int, String!");
}

#[test]
fn diagnostic_render_context_falls_back_to_type_id_without_matching_environment() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let orphan_type = TypeId(999);

    let no_environment_context = DiagnosticRenderContext::new(&string_table);
    assert_eq!(
        diagnostic_type_name(orphan_type, no_environment_context),
        "TypeId(999)"
    );

    let missing_type_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));
    assert_eq!(
        diagnostic_type_name(orphan_type, missing_type_context),
        "TypeId(999)"
    );
}

#[test]
fn terse_type_mismatch_uses_type_environment_names_when_available() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::Assignment,
        None,
    );

    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);
    let lines = format_terse_diagnostics_with_context(&[diagnostic], context);

    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("expected Int, found String"));
    assert!(!lines[0].contains("TypeId("));
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
fn unsupported_operator_message_explains_fixed_numeric_mixing() {
    let cases = [
        (
            DiagnosticOperator::Add,
            builtin_type_ids::fixed_scalar(FixedScalar::U8),
            builtin_type_ids::INT,
            "`U8` and `Int` do not mix implicitly. Convert one operand with `cast` first.",
        ),
        (
            DiagnosticOperator::Equality,
            builtin_type_ids::fixed_scalar(FixedScalar::F32),
            builtin_type_ids::FLOAT,
            "`F32` and `Float` do not mix implicitly. Convert one operand with `cast` first.",
        ),
        (
            DiagnosticOperator::Multiply,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::fixed_scalar(FixedScalar::F16),
            "`I32` and `F16` do not mix implicitly. Convert one operand with `cast` first.",
        ),
    ];

    for (operator, lhs, rhs, expected) in cases {
        assert_eq!(
            unsupported_operator_message(operator, lhs, Some(rhs)),
            expected
        );
    }
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
    let string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

    unsupported_operator_types_message(operator, lhs, rhs, context)
}
