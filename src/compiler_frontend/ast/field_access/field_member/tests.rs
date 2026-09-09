//! Module-store authority tests for compile-time field inlining.
//!
//! WHAT: exercises receiver-authored and resolved-default field values whose templates are
//!       resolved from the shared module-local TIR store.
//! WHY: field access must classify the exact effective TIR view and preserve its overlay
//!      identity at the access site.

use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use super::{const_inline_field_value, const_inline_field_value_from_receiver};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
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
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
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
    let (registry, template) = store_with_template();
    let field_name = string_table.intern("content");
    let field_path = InternedPath::from_components(vec![field_name]);
    let receiver_value = Expression::struct_instance(
        InternedPath::from_single_str("Card", &mut string_table),
        vec![Declaration {
            id: field_path,
            value: Expression::template(template, ValueMode::ImmutableOwned),
            binding_span: None,
            config_qualifier: None,
        }],
        None,
        ValueMode::ImmutableOwned,
        true,
        None,
        TypeEnvironment::new().builtins().none,
    );
    let receiver = AstNode {
        kind: NodeKind::ExpressionStatement(receiver_value),
        span: None,
        scope: InternedPath::from_single_str("scope", &mut string_table),
    };

    let inlined = const_inline_field_value_from_receiver(&receiver, field_name, &registry, None)
        .expect("effective TIR classification should succeed")
        .expect("receiver-authored const field should inline");

    assert!(matches!(inlined.kind, ExpressionKind::Template(_)));
    assert_eq!(inlined.value_mode, ValueMode::ImmutableOwned);
}

#[test]
fn missing_member_name_after_dot_points_at_offending_token_boundary() {
    // A non-EOF token after the dot is the immediate missing-member boundary. The diagnostic
    // must point at that offending token, not the authored dot or the receiver start. This
    // complements the integration case, which pins the EOF span at the authored dot.
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);
    let mut span_builder = ExtendedSpanBuilder::new();
    let offending_span =
        LocalSpan::exact(12, 1, &mut span_builder).expect("offending token span should fit");

    let stream = FileTokens::new(
        scope,
        SourceId::COMPILATION_ROOT,
        vec![
            Token::new(TokenKind::Comma, offending_span),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
    );

    let error = super::parse_member_name_typed(&stream, &string_table)
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
    let (registry, template) = store_with_template();
    let mut type_environment = TypeEnvironment::new();
    let field_name = string_table.intern("content");
    let struct_path = InternedPath::from_single_str("Card", &mut string_table);
    let field_path = struct_path.clone().append(field_name);
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: vec![FieldDefinition {
            name: field_path.clone(),
            type_id: type_environment.builtins().string,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: true,
    });
    let resolved_fields = FxHashMap::from_iter([(
        struct_path.clone(),
        vec![Declaration {
            id: field_path,
            value: Expression::template(template, ValueMode::ImmutableOwned),
            binding_span: None,
            config_qualifier: None,
        }],
    )]);
    let receiver = AstNode {
        kind: NodeKind::ExpressionStatement(Expression::reference_with_type_id(
            InternedPath::from_single_str("card", &mut string_table),
            DataType::const_struct_record(struct_path, struct_type_id),
            struct_type_id,
            None,
            ValueMode::ImmutableReference,
            crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::ConstRecord,
        )),
        span: None,
        scope: InternedPath::from_single_str("scope", &mut string_table),
    };

    let inlined = const_inline_field_value(
        &receiver,
        struct_type_id,
        field_name,
        &type_environment,
        Some(&resolved_fields),
        &registry,
        None,
    )
    .expect("effective TIR classification should succeed")
    .expect("resolved const default should inline");

    assert!(matches!(inlined.kind, ExpressionKind::Template(_)));
    assert_eq!(inlined.value_mode, ValueMode::ImmutableOwned);
}
