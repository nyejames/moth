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
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, InvalidFieldAccessReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, FileTokens, SourceLocation, Token, TokenKind,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn slot_template(store: &mut TemplateIrStore) -> Template {
    let location = SourceLocation::default();
    let mut builder = TemplateIrBuilder::new(store);
    let slot = builder.push_slot_node(SlotKey::Default, location.clone());
    let template_id = builder.finish_template(
        slot,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        location.clone(),
    );

    Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location,
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
            config_qualifier: None,
        }],
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
        true,
        None,
        TypeEnvironment::new().builtins().none,
    );
    let receiver = AstNode {
        kind: NodeKind::ExpressionStatement(receiver_value),
        location: SourceLocation::default(),
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
    // complements the integration case, which pins the EOF location at the authored dot.
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);

    let offending_position = CharPosition {
        line_number: 4,
        char_column: 12,
    };
    let offending_location =
        SourceLocation::new(scope.clone(), offending_position, offending_position);
    let end_location = SourceLocation::new(
        scope.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );

    let stream = FileTokens::new(
        scope,
        vec![
            Token::new(TokenKind::Comma, offending_location.clone()),
            Token::new(TokenKind::Eof, end_location),
        ],
    );

    let error = super::parse_member_name_typed(&stream, &string_table)
        .expect_err("a non-name token after '.' must be rejected as a missing member name");

    let diagnostic = CompilerDiagnostic::from(error);
    assert_eq!(diagnostic.primary_location, offending_location);

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
            location: SourceLocation::default(),
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
            config_qualifier: None,
        }],
    )]);
    let receiver = AstNode {
        kind: NodeKind::ExpressionStatement(Expression::reference(
            InternedPath::from_single_str("card", &mut string_table),
            DataType::const_struct_record(struct_path, struct_type_id),
            SourceLocation::default(),
            ValueMode::ImmutableReference,
        )),
        location: SourceLocation::default(),
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
