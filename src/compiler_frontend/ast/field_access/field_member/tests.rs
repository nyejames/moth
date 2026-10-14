//! Module-store authority tests for compile-time field inlining.
//!
//! WHAT: exercises receiver-authored and resolved-default field values whose templates are
//!       resolved from the shared module-local TIR store.
//! WHY: field access must classify the exact effective TIR view and preserve its overlay
//!      identity at the access site.

use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use super::{FieldMemberResolution, receiver_value_expression, resolve_field_member};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ConstructorField, ExpressionOperationView,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidFieldAccessReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};
use crate::compiler_frontend::value_mode::ValueMode;

fn slot_template(store: &mut TemplateIrStore) -> Template {
    let mut builder = TemplateIrBuilder::new(store);
    let slot = builder.push_slot_node(SlotKey::Default, None);
    let template_id = builder.finish_template(
        slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );

    Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        span: None,
    }
}

fn store_with_template() -> (Rc<RefCell<TemplateIrStore>>, Template) {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let template = slot_template(&mut store.borrow_mut());
    (store, template)
}

#[test]
fn receiver_authored_field_uses_foreign_effective_tir() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (registry, template) = store_with_template();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let field_name = string_table.intern("content");
    let card_path = path_fork
        .try_intern_portable_path("Card", &mut string_table)
        .expect("test path fits");
    let field_path = path_fork
        .try_intern_child(card_path, field_name)
        .expect("test path fits");
    let mut type_environment = TypeEnvironment::new();
    let (_, card_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        // `register_nominal_struct` overwrites this placeholder and returns the real identity,
        // so the fixture takes the canonical `TypeId` from registration.
        id: NominalTypeId(0),
        path: card_path,
        fields: vec![FieldDefinition {
            name: field_path,
            type_id: type_environment.builtins().string,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: true,
    });
    let field_value = expression_builder
        .template(template, ValueMode::ImmutableOwned)
        .expect("template constructor succeeds");
    let receiver_value = expression_builder
        .struct_instance(
            vec![ConstructorField {
                name: field_path,
                slot: 0,
                value: field_value,
            }],
            card_type_id,
            None,
            ValueMode::ImmutableOwned,
            true,
        )
        .expect("record constructor succeeds");
    let receiver = AstNode {
        kind: NodeKind::ExpressionStatement(receiver_value.id()),
        span: None,
        scope: path_fork
            .try_intern_portable_path("scope", &mut string_table)
            .expect("test path fits"),
    };

    let receiver_expression = receiver_value_expression(&receiver);
    let inlined = resolve_field_member(FieldMemberResolution {
        receiver_value: receiver_expression,
        receiver_type_id: card_type_id,
        field_name,
        type_environment: &type_environment,
        resolved_struct_fields_by_path: None,
        receiver_is_const_record: true,
        template_ir_store: Some(&registry),
        scope_context: None,
        path_fork: &path_fork,
        expression_builder: &mut expression_builder,
        const_values: &const_values,
    })
    .expect("effective TIR classification should succeed")
    .expect("receiver-authored const field should resolve")
    .const_inline_value
    .expect("receiver-authored const field should inline");

    let inlined = expression_builder
        .view(inlined)
        .expect("inlined expression is owned");
    assert!(matches!(
        inlined.operation(),
        ExpressionOperationView::Template(_)
    ));
    assert_eq!(inlined.value_mode(), ValueMode::ImmutableOwned);
}

#[test]
fn missing_member_name_after_dot_points_at_offending_token_boundary() {
    // A non-EOF token after the dot is the immediate missing-member boundary. The diagnostic
    // must point at that offending token, not the authored dot or the receiver start. This
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let offending_span =
        LocalSpan::exact(12, 1, &mut span_builder).expect("offending token span should fit");

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_static(TokenTag::COMMA, offending_span)
        .expect("offending token fixture should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("canonical fixture tokens should build");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let error = super::parse_member_name_typed(&stream, &mut string_table)
        .expect_err("a non-name token after '.' must be rejected as a missing member name");
    let crate::compiler_frontend::ast::expressions::error::ExpressionParseError::Diagnostic(
        diagnostic,
    ) = error
    else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}")
    };
    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, offending_span))
    );

    match diagnostic.payload {
        DiagnosticPayload::InvalidFieldAccess {
            reason: InvalidFieldAccessReason::ExpectedNameAfterDot,
            ..
        } => {}
        other => panic!("expected InvalidFieldAccess::ExpectedNameAfterDot, got {other:?}"),
    }
}
#[test]
fn resolved_default_field_uses_foreign_effective_tir() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (registry, template) = store_with_template();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let field_name = string_table.intern("content");
    let struct_path = path_fork
        .try_intern_portable_path("Card", &mut string_table)
        .expect("test path fits");
    let field_path = path_fork
        .try_intern_child(struct_path, field_name)
        .expect("test path fits");
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        // `register_nominal_struct` overwrites this placeholder and returns the real identity.
        id: NominalTypeId(0),
        path: struct_path,
        fields: vec![FieldDefinition {
            name: field_path,
            type_id: type_environment.builtins().string,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: true,
    });
    let resolved_fields = FxHashMap::from_iter([(
        struct_path,
        vec![{
            let value = expression_builder
                .template(template, ValueMode::ImmutableOwned)
                .expect("template constructor succeeds");
            Declaration {
                id: field_path,
                type_id: expression_builder
                    .view_owned(value)
                    .expect("field is owned")
                    .value_type_id()
                    .expect("field is single-valued"),
                diagnostic_type: DataType::StringSlice,
                value_mode: ValueMode::ImmutableOwned,
                value: DeclarationValue::Initializer(value.id()),
                binding_span: None,
                config_qualifier: None,
            }
        }],
    )]);
    let receiver = AstNode {
        kind: NodeKind::ExpressionStatement(expression_builder.reference_with_type_id(
            path_fork
                .try_intern_portable_path("card", &mut string_table)
                .expect("test path fits"),
            struct_type_id,
            None,
            ValueMode::ImmutableReference,
            crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::ConstRecord,
        ).expect("reference constructor succeeds").id()),
        span: None,
        scope: path_fork
            .try_intern_portable_path("scope", &mut string_table)
            .expect("test path fits"),
    };

    let receiver_value = receiver_value_expression(&receiver);
    let inlined = resolve_field_member(FieldMemberResolution {
        receiver_value,
        receiver_type_id: struct_type_id,
        field_name,
        type_environment: &type_environment,
        resolved_struct_fields_by_path: Some(&resolved_fields),
        receiver_is_const_record: true,
        template_ir_store: Some(&registry),
        scope_context: None,
        path_fork: &path_fork,
        expression_builder: &mut expression_builder,
        const_values: &const_values,
    })
    .expect("effective TIR classification should succeed")
    .expect("resolved const default should resolve")
    .const_inline_value
    .expect("resolved const default should inline");

    let inlined = expression_builder
        .view(inlined)
        .expect("inlined expression is owned");
    assert!(matches!(
        inlined.operation(),
        ExpressionOperationView::Template(_)
    ));
    assert_eq!(inlined.value_mode(), ValueMode::ImmutableOwned);
}

#[test]
fn resolved_field_default_uses_constant_content_without_reexpansion() {
    use crate::compiler_frontend::ast::const_values::store::ConstValuePayload;
    use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;

    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let registry = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut environment = TypeEnvironment::new();
    let field_name = strings.intern("enabled");
    let record_path = paths
        .try_intern_portable_path("Settings", &mut strings)
        .expect("record path fits");
    let field_path = paths
        .try_intern_child(record_path, field_name)
        .expect("field path fits");
    let bool_type = environment.builtins().bool;
    let (_, record_type) = environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: record_path,
        fields: vec![FieldDefinition {
            name: field_path,
            type_id: bool_type,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: true,
    });
    let default = const_values
        .insert_resolved(bool_type, ConstValuePayload::Bool(true), None)
        .expect("constant default fits");
    let resolved_fields = FxHashMap::from_iter([(
        record_path,
        vec![Declaration {
            id: field_path,
            type_id: bool_type,
            diagnostic_type: DataType::Bool,
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Default(default),
            binding_span: None,
            config_qualifier: None,
        }],
    )]);
    let receiver_path = paths
        .try_intern_portable_path("settings", &mut strings)
        .expect("receiver path fits");
    let receiver = expression_builder
        .reference_with_type_id(
            receiver_path,
            record_type,
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::ConstRecord,
        )
        .expect("receiver constructor succeeds");
    let field = resolve_field_member(FieldMemberResolution {
        receiver_value: Some(receiver.id()),
        receiver_type_id: record_type,
        field_name,
        type_environment: &environment,
        resolved_struct_fields_by_path: Some(&resolved_fields),
        receiver_is_const_record: true,
        template_ir_store: Some(&registry),
        scope_context: None,
        path_fork: &paths,
        expression_builder: &mut expression_builder,
        const_values: &const_values,
    })
    .expect("field resolution succeeds")
    .expect("field exists");
    let inlined = expression_builder
        .view(field.const_inline_value.expect("default inlines"))
        .expect("inlined occurrence is owned");
    assert!(matches!(inlined.operation(), ExpressionOperationView::Constant(id) if id == default));
    assert_eq!(inlined.value_mode(), ValueMode::ImmutableOwned);
    assert_eq!(inlined.success_types(), &[bool_type]);
    assert_eq!(
        expression_builder
            .view_owned(receiver)
            .expect("receiver is owned")
            .value_mode(),
        ValueMode::ImmutableReference
    );
}
