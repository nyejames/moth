//! Struct parsing regression tests.
//!
//! WHAT: validates struct definitions, defaults, constructors, and field access.
//! WHY: struct parsing feeds both type resolution and HIR place lowering.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrStore, TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabelMessage, DiagnosticLabelStyle, DiagnosticPayload, GenericInferenceSubject,
    InvalidFieldAccessReason, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::declaration_syntax::r#struct::validate_struct_default_values;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn body_local_struct_default_preserves_missing_template_authority() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let fields = [Declaration {
        id: InternedPath::new(),
        value: Expression::template(
            Template {
                tir_reference: TemplateTirReference {
                    root: TemplateIrId::new(99),
                    phase: TemplateTirPhase::Composed,
                    context: TemplateViewContext::default(),
                },
                location: SourceLocation::default(),
            },
            ValueMode::ImmutableOwned,
        ),
        config_qualifier: None,
    }];

    let error = validate_struct_default_values(&fields, &store)
        .expect_err("missing struct-default TIR authority must fail");

    assert!(matches!(error, TemplateError::Infrastructure(_)));
}

#[test]
fn authored_runtime_struct_default_remains_a_source_diagnostic() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let fields = [Declaration {
        id: InternedPath::new(),
        value: Expression::reference_with_type_id(
            InternedPath::new(),
            DataType::Bool,
            builtin_type_ids::BOOL,
            SourceLocation::default(),
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        ),
        config_qualifier: None,
    }];

    let error = validate_struct_default_values(&fields, &store)
        .expect_err("runtime struct default must be rejected");
    let TemplateError::Diagnostic(diagnostic) = error else {
        panic!("authored runtime default should remain a source diagnostic");
    };

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStructDefaultValue
    ));
}

#[test]
fn parses_struct_definitions_with_field_defaults() {
    let (ast, string_table) = parse_single_file_ast("Point = |\n    x Int,\n    y Int = 2,\n|\n");

    let struct_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path.name_str(&string_table) == Some("Point")
            )
        })
        .expect("expected struct definition");

    let NodeKind::StructDefinition(path, fields) = &struct_node.kind else {
        panic!("expected struct definition node");
    };

    assert_eq!(path.name_str(&string_table), Some("Point"));
    assert_eq!(fields.len(), 2);
    assert!(matches!(fields[0].value.kind, ExpressionKind::NoValue));
    assert!(matches!(fields[1].value.kind, ExpressionKind::Int(2)));
}

#[test]
fn struct_optional_string_default_preserves_canonical_string_type_id() {
    let (ast, string_table) =
        parse_single_file_ast("Label = |\n    text String? = \"fallback\",\n|\n");

    let struct_node = ast
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.kind,
                NodeKind::StructDefinition(path, ..)
                    if path.name_str(&string_table) == Some("Label")
            )
        })
        .expect("expected Label struct definition");

    let NodeKind::StructDefinition(_, fields) = &struct_node.kind else {
        panic!("expected struct definition node");
    };

    assert_eq!(fields[0].value.type_id, builtin_type_ids::STRING);
}

#[test]
fn parses_struct_construction_and_field_access_in_declarations() {
    let (ast, string_table) = parse_single_file_ast(
        "Point = |\n    x Int,\n    y Int,\n|\n\npoint = Point(1, 2)\nvalue = point.x\n",
    );

    let body = start_function_body(&ast, &string_table);

    let NodeKind::VariableDeclaration(point_decl) = &body[0].kind else {
        panic!("expected point declaration");
    };
    assert!(matches!(
        point_decl.value.kind,
        ExpressionKind::StructInstance(..)
    ));

    let NodeKind::VariableDeclaration(value_decl) = &body[1].kind else {
        panic!("expected field-read declaration");
    };
    assert!(
        matches!(value_decl.value.kind, ExpressionKind::FieldAccess { .. }),
        "field access should be stored as an expression-owned field-access payload"
    );
}

#[test]
fn parses_builtin_error_with_default_code_field() {
    let (ast, string_table) = parse_single_file_ast("err = Error(\"bad\")\n");

    let body = start_function_body(&ast, &string_table);
    let NodeKind::VariableDeclaration(error_decl) = &body[0].kind else {
        panic!("expected error declaration");
    };
    let ExpressionKind::StructInstance(fields) = &error_decl.value.kind else {
        panic!("expected Error constructor to lower as a struct instance");
    };

    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].id.name_str(&string_table), Some("message"));
    assert!(matches!(
        fields[0].value.kind,
        ExpressionKind::StringSlice(..)
    ));
    assert_eq!(fields[1].id.name_str(&string_table), Some("code"));
    assert!(matches!(fields[1].value.kind, ExpressionKind::Int(0)));
}

#[test]
fn rejects_removed_builtin_error_fields() {
    let diagnostic = parse_single_file_ast_diagnostic("err = Error(\"bad\")\nvalue = err.kind\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFieldAccess {
            reason: InvalidFieldAccessReason::UnknownMember,
            ..
        }
    ));
}

#[test]
fn generic_struct_conflict_keeps_argument_and_expected_type_evidence_locations() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "Pair type T = |\n\
             left T,\n\
             right T,\n\
         |\n\
         bad Pair of Int = Pair(\"two\", 1)\n",
    );

    let DiagnosticPayload::InvalidGenericInstantiation {
        reason:
            InvalidGenericInstantiationReason::ConflictingInference {
                subject,
                current_evidence_location,
                previous_evidence_location,
                ..
            },
        ..
    } = &diagnostic.payload
    else {
        panic!(
            "expected a generic inference conflict, got {:?}",
            diagnostic.payload
        );
    };
    let previous_evidence_location = previous_evidence_location
        .as_ref()
        .expect("expected type evidence should be retained");

    assert_eq!(*subject, GenericInferenceSubject::NominalType);
    assert_eq!(diagnostic.primary_location, *current_evidence_location);
    assert_eq!(
        current_evidence_location.start_pos.line_number,
        previous_evidence_location.start_pos.line_number
    );
    assert!(
        current_evidence_location.start_pos.char_column
            > previous_evidence_location.start_pos.char_column,
        "the argument evidence should follow the receiving-boundary evidence"
    );
    assert!(diagnostic.labels.iter().any(|label| {
        label.style == DiagnosticLabelStyle::Secondary
            && label.location == *previous_evidence_location
            && label.message == Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence)
    }));
}
