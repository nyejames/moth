//! Bound expression views carry neutral runtime handoff payloads without TIR identity.

use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView,
};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tests::ast_fixture_support::{
    runtime_slot_application_handoff_expr, runtime_template_handoff_expr,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_runtime_template_handoff() -> OwnedRuntimeTemplateHandoff {
    OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: Vec::new(),
            span: None,
        }),
    }
}

fn empty_runtime_slot_application_handoff() -> OwnedRuntimeSlotApplicationHandoff {
    OwnedRuntimeSlotApplicationHandoff {
        wrapper: OwnedRuntimeTemplateNode::Sequence {
            children: Vec::new(),
            span: None,
        },
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
    }
}

#[test]
fn runtime_template_handoff_expression_carries_owned_payload() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expression = runtime_template_handoff_expr(
        &mut expressions,
        empty_runtime_template_handoff(),
        None,
        ValueMode::ImmutableOwned,
    );
    let expression = expressions.view(expression).expect("owner-bound view");
    assert_eq!(
        expression.value_type_id().expect("single string"),
        builtin_type_ids::STRING
    );

    let ExpressionOperationView::RuntimeTemplate(handoff) = expression.operation() else {
        panic!("expected runtime template handoff expression");
    };

    assert!(matches!(
        handoff.body,
        OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { .. })
    ));
}

#[test]
fn runtime_slot_application_handoff_expression_carries_owned_payload() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let expression = runtime_slot_application_handoff_expr(
        &mut expressions,
        empty_runtime_slot_application_handoff(),
        None,
        ValueMode::ImmutableOwned,
    );
    let expression = expressions.view(expression).expect("owner-bound view");
    assert_eq!(
        expression.value_type_id().expect("single string"),
        builtin_type_ids::STRING
    );

    let ExpressionOperationView::RuntimeSlotApplication(handoff) = expression.operation() else {
        panic!("expected runtime slot application handoff expression");
    };

    assert!(handoff.contribution_sources.is_empty());
    assert!(handoff.slot_sites.is_empty());
}
