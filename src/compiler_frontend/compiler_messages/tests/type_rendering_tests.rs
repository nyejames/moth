//! Unit tests for type-name rendering at the diagnostic boundary.
//!
//! WHAT: exercises `DiagnosticRenderContext` rather than raw datatype display helpers.
//! WHY: diagnostics carry `TypeId`s, so the renderer is the contract that turns semantic type
//! identity into source-level names when a module `TypeEnvironment` is available.

use crate::compiler_frontend::compiler_messages::render::terminal::format_payload_guidance;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostics_with_context;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, diagnostic_type_name,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidFieldAccessReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, NominalTypeId, TypeConstructor, TypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn diagnostic_render_context_renders_builtin_type_names() {
    let type_environment = TypeEnvironment::new();
    let string_table = StringTable::new();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));

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
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));
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
fn diagnostic_render_context_renders_nominal_struct_and_choice_names() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let point_path = InternedPath::from_single_str("Point", &mut string_table);
    let (_, point_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let status_path = InternedPath::from_single_str("Status", &mut string_table);
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

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));

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

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));

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
    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::Assignment,
        None,
    );

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));
    let lines = format_terse_diagnostics_with_context(&[diagnostic], context);

    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("expected Int, found String"));
    assert!(!lines[0].contains("TypeId("));
}
