//! Expression payload tests for owned runtime-template handoffs.
//!
//! WHAT: proves the final AST expression variants can carry the neutral owned handoff payloads.
//! WHY: Phase 11 introduces the expression shape before HIR lowering consumes it, so these tests
//! stay intentionally construction-focused and avoid changing runtime template behavior.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn test_location() -> SourceLocation {
    SourceLocation::default()
}

fn empty_runtime_template_handoff() -> OwnedRuntimeTemplateHandoff {
    let location = test_location();
    OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: Vec::new(),
        }),
        location,
    }
}

fn empty_runtime_slot_application_handoff() -> OwnedRuntimeSlotApplicationHandoff {
    let location = test_location();
    OwnedRuntimeSlotApplicationHandoff {
        wrapper: OwnedRuntimeTemplateNode::Sequence {
            children: Vec::new(),
        },
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
        location,
    }
}

#[test]
fn runtime_template_handoff_expression_carries_owned_payload() {
    let expression = Expression::runtime_template_handoff(
        empty_runtime_template_handoff(),
        ValueMode::ImmutableOwned,
    );

    assert_eq!(expression.type_id, builtin_type_ids::STRING);
    assert_eq!(expression.diagnostic_type, DataType::Template);
    assert!(expression.reactive_template.is_some());

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = expression.kind else {
        panic!("expected runtime template handoff expression");
    };

    assert!(matches!(
        handoff.body,
        OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence { .. })
    ));
}

#[test]
fn runtime_slot_application_handoff_expression_carries_owned_payload() {
    let expression = Expression::runtime_slot_application_handoff(
        empty_runtime_slot_application_handoff(),
        ValueMode::ImmutableOwned,
    );

    assert_eq!(expression.type_id, builtin_type_ids::STRING);
    assert_eq!(expression.diagnostic_type, DataType::Template);
    assert!(expression.reactive_template.is_some());

    let ExpressionKind::RuntimeSlotApplicationHandoff(handoff) = expression.kind else {
        panic!("expected runtime slot application handoff expression");
    };

    assert!(handoff.contribution_sources.is_empty());
    assert!(handoff.slot_sites.is_empty());
}
