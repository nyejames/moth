//! Guaranteed-output classification tests for the runtime slot-application walker.
//!
//! WHAT: pins guaranteed output for structural text pieces and ensures visible bodies under
//!       conditional control flow do not inherit that guarantee.
//! WHY: these classifications control slot-wrapper output tracking, so both piece-bearing text
//!       and lazy no-output paths need focused coverage.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId;
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use std::path::Path;

fn fixture_resource_origin(relative_path: &str) -> StableResourceOriginId {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("hir-slot-guarantee-tests"),
        String::new(),
        ModuleRootRole::Normal,
    );

    let resource_path = PortableResourcePath::from_relative_logical_path(Path::new(relative_path))
        .expect("fixture resource path should be portable");

    StableResourceOriginId::module_owned(module_origin, resource_path)
}

fn piece_bearing_text_node(pieces: Vec<OwnedFoldedStringPiece>) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::Text {
        text: OwnedFoldedString::Pieces(pieces),
    }
}

fn plain_text_node(text: &str) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::Text {
        text: OwnedFoldedString::Text(text.to_owned()),
    }
}

fn false_boolean_expression() -> Expression {
    Expression::bool(false, None, ValueMode::default())
}

fn slot_application_with_false_conditional_source(
    wrapper: OwnedRuntimeTemplateNode,
) -> OwnedRuntimeSlotApplicationHandoff {
    OwnedRuntimeSlotApplicationHandoff {
        wrapper,
        contribution_sources: vec![OwnedRuntimeSlotContributionSource {
            source: RuntimeSlotContributionSourceId(0),
            render_root: OwnedRuntimeTemplateNode::Conditional {
                selector: Box::new(TemplateBranchSelector::Bool(false_boolean_expression())),
                body: Box::new(plain_text_node("contribution")),
                span: None,
            },
            renders_wrapper_unconditionally: false,
            span: None,
        }],
        slot_sites: Vec::new(),
        span: None,
    }
}

#[test]
fn text_node_with_a_resource_piece_guarantees_output() {
    let string_table = StringTable::new();

    let node = piece_bearing_text_node(vec![OwnedFoldedStringPiece::Resource(
        fixture_resource_origin("assets/logo.svg"),
    )]);

    assert!(
        super::owned_runtime_template_node_guarantees_output(&node, &string_table),
        "a resource piece renders a URL once the build assigns contexts, so the node must keep the wrapper's unconditional-output guarantee"
    );
}

#[test]
fn text_node_with_a_site_root_piece_guarantees_output() {
    let string_table = StringTable::new();
    let node = piece_bearing_text_node(vec![
        OwnedFoldedStringPiece::SiteRoot,
        OwnedFoldedStringPiece::Text("   ".to_owned()),
    ]);

    assert!(
        super::owned_runtime_template_node_guarantees_output(&node, &string_table),
        "a site-root piece guarantees output even when every literal piece is blank"
    );
}

#[test]
fn plain_text_and_all_text_pieces_keep_the_trimming_classification() {
    let string_table = StringTable::new();

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &plain_text_node("hello"),
            &string_table
        ),
        "nonempty plain text keeps its guarantee"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(&plain_text_node(""), &string_table),
        "empty plain text still produces no output"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &plain_text_node("   "),
            &string_table
        ),
        "blank plain text still produces no output"
    );

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![OwnedFoldedStringPiece::Text("hello".to_owned())]),
            &string_table
        ),
        "an all-text piece list with literal runs keeps the same guarantee as plain text"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![
                OwnedFoldedStringPiece::Text(" ".to_owned()),
                OwnedFoldedStringPiece::Text(String::new()),
            ]),
            &string_table
        ),
        "an all-text piece list of blank runs still produces no output"
    );

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![
                OwnedFoldedStringPiece::Text("a".to_owned()),
                OwnedFoldedStringPiece::Text(" ".to_owned()),
            ]),
            &string_table
        ),
        "an all-text piece list with one nonempty run keeps its guarantee"
    );
}

#[test]
fn false_conditional_body_does_not_guarantee_output() {
    let string_table = StringTable::new();
    let body = plain_text_node("visible");
    let node = OwnedRuntimeTemplateNode::Conditional {
        selector: Box::new(TemplateBranchSelector::Bool(false_boolean_expression())),
        body: Box::new(body.clone()),
        span: None,
    };

    assert!(super::owned_runtime_template_node_guarantees_output(
        &body,
        &string_table
    ));
    assert!(!super::owned_runtime_template_node_guarantees_output(
        &node,
        &string_table
    ));
}

#[test]
fn false_conditional_loop_body_does_not_guarantee_output() {
    let string_table = StringTable::new();
    let body = plain_text_node("visible");
    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Conditional {
            condition: Box::new(false_boolean_expression()),
        },
        body: Box::new(body.clone()),
        aggregate_wrapper: None,
        span: None,
    };

    assert!(super::owned_runtime_template_node_guarantees_output(
        &body,
        &string_table
    ));
    assert!(!super::owned_runtime_template_node_guarantees_output(
        &node,
        &string_table
    ));
}

#[test]
fn nested_slot_application_body_uses_wrapper_output_proof() {
    let string_table = StringTable::new();
    let handoff = slot_application_with_false_conditional_source(plain_text_node("<shell>"));
    let template = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::RuntimeSlotApplication(Box::new(handoff)),
        span: None,
    };
    let child = OwnedRuntimeTemplateNode::ChildTemplate {
        template: Box::new(template.clone()),
        span: None,
    };

    assert!(super::owned_runtime_template_node_guarantees_output(
        &child,
        &string_table
    ));
    assert!(super::runtime_template_handoff_guarantees_output(
        &template,
        &string_table
    ));
}

#[test]
fn nested_slot_application_expression_uses_wrapper_dynamic_output_proof() {
    let mut string_table = StringTable::new();
    let dynamic_text = Expression::string_slice(
        string_table.intern("dynamic wrapper text"),
        None,
        ValueMode::default(),
    );
    let wrapper = OwnedRuntimeTemplateNode::DynamicExpression {
        expression: Box::new(dynamic_text),
        span: None,
    };
    let expression = Expression::runtime_slot_application_handoff(
        slot_application_with_false_conditional_source(wrapper),
        ValueMode::default(),
    );

    assert!(super::dynamic_expression_guarantees_output(
        &expression,
        &string_table
    ));
}

#[test]
fn nested_slot_application_with_output_free_wrapper_stays_unproven() {
    let string_table = StringTable::new();

    for wrapper in [
        plain_text_node(""),
        plain_text_node(" \n\t"),
        OwnedRuntimeTemplateNode::Slot { span: None },
    ] {
        let handoff = slot_application_with_false_conditional_source(wrapper);
        let template = OwnedRuntimeTemplateHandoff {
            body: OwnedRuntimeTemplateBody::RuntimeSlotApplication(Box::new(handoff.clone())),
            span: None,
        };
        let child = OwnedRuntimeTemplateNode::ChildTemplate {
            template: Box::new(template.clone()),
            span: None,
        };
        let expression =
            Expression::runtime_slot_application_handoff(handoff, ValueMode::default());

        assert!(!super::owned_runtime_template_node_guarantees_output(
            &child,
            &string_table
        ));
        assert!(!super::runtime_template_handoff_guarantees_output(
            &template,
            &string_table
        ));
        assert!(!super::dynamic_expression_guarantees_output(
            &expression,
            &string_table
        ));
    }
}
