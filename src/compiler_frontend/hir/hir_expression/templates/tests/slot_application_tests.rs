//! Guaranteed-output classification tests for the runtime slot-application walker.
//!
//! WHAT: pins guaranteed output for structural text pieces, resolved slot
//!       sites and wrapper bodies under conditional control flow.
//! WHY: these classifications control slot-wrapper output tracking, so both
//!       fallback-bearing sites and lazy no-output paths need focused coverage.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource, OwnedRuntimeSlotSite,
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
use crate::compiler_frontend::tests::ast_fixture_support::runtime_slot_application_handoff_expr;
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

fn constant_text_node(text: &str) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::ConstantText {
        text: OwnedFoldedString::Text(text.to_owned()),
    }
}

fn expression_fixture() -> (AstExpressionBuilder, ConstValueStore, ExprId) {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let anchor = expressions
        .bool(&mut const_values, false, None, ValueMode::default())
        .unwrap()
        .id();
    (expressions, const_values, anchor)
}

fn slot_application_with_false_conditional_source(
    wrapper: OwnedRuntimeTemplateNode,
    anchor: ExprId,
) -> OwnedRuntimeSlotApplicationHandoff {
    OwnedRuntimeSlotApplicationHandoff {
        wrapper,
        contribution_sources: vec![OwnedRuntimeSlotContributionSource {
            source: RuntimeSlotContributionSourceId(0),
            render_root: OwnedRuntimeTemplateNode::Conditional {
                selector: Box::new(TemplateBranchSelector::Bool(anchor)),
                body: Box::new(plain_text_node("contribution")),
                span: None,
            },
            renders_wrapper_unconditionally: false,
        }],
        slot_sites: Vec::new(),
    }
}

#[test]
fn text_node_with_a_resource_piece_guarantees_output() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();

    let node = piece_bearing_text_node(vec![OwnedFoldedStringPiece::Resource(
        fixture_resource_origin("assets/logo.svg"),
    )]);

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &node,
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "a resource piece renders a URL once the build assigns contexts, so the node must keep the wrapper's unconditional-output guarantee"
    );
}

#[test]
fn text_node_with_a_site_root_piece_guarantees_output() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();
    let node = piece_bearing_text_node(vec![
        OwnedFoldedStringPiece::SiteRoot,
        OwnedFoldedStringPiece::Text("   ".to_owned()),
    ]);

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &node,
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "a site-root piece guarantees output even when every literal piece is blank"
    );
}

#[test]
fn authored_constant_text_guarantees_output_regardless_of_characters() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();

    for text in ["", " \n\t"] {
        let node = constant_text_node(text);
        assert!(
            super::owned_runtime_template_node_guarantees_output(
                &node,
                expressions.view(anchor).unwrap(),
                &const_values,
                &string_table,
                None,
            ),
            "a selected authored fallback is output by identity, including {text:?}"
        );
    }
}

#[test]
fn plain_text_and_all_text_pieces_keep_the_trimming_classification() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &plain_text_node("hello"),
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "nonempty plain text keeps its guarantee"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &plain_text_node(""),
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "empty plain text still produces no output"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &plain_text_node("   "),
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "blank plain text still produces no output"
    );

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![OwnedFoldedStringPiece::Text("hello".to_owned())]),
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "an all-text piece list with literal runs keeps the same guarantee as plain text"
    );

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![
                OwnedFoldedStringPiece::Text(" ".to_owned()),
                OwnedFoldedStringPiece::Text(String::new()),
            ]),
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "an all-text piece list of blank runs still produces no output"
    );

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &piece_bearing_text_node(vec![
                OwnedFoldedStringPiece::Text("a".to_owned()),
                OwnedFoldedStringPiece::Text(" ".to_owned()),
            ]),
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ),
        "an all-text piece list with one nonempty run keeps its guarantee"
    );
}

#[test]
fn false_conditional_body_does_not_guarantee_output() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();
    let body = plain_text_node("visible");
    let node = OwnedRuntimeTemplateNode::Conditional {
        selector: Box::new(TemplateBranchSelector::Bool(anchor)),
        body: Box::new(body.clone()),
        span: None,
    };

    assert!(super::owned_runtime_template_node_guarantees_output(
        &body,
        expressions.view(anchor).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
    assert!(!super::owned_runtime_template_node_guarantees_output(
        &node,
        expressions.view(anchor).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
}

#[test]
fn false_conditional_loop_body_does_not_guarantee_output() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();
    let body = plain_text_node("visible");
    let node = OwnedRuntimeTemplateNode::Loop {
        header: TemplateLoopHeader::Conditional { condition: anchor },
        body: Box::new(body.clone()),
        aggregate_wrapper: None,
        span: None,
    };

    assert!(super::owned_runtime_template_node_guarantees_output(
        &body,
        expressions.view(anchor).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
    assert!(!super::owned_runtime_template_node_guarantees_output(
        &node,
        expressions.view(anchor).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
}

#[test]
fn unresolved_default_slot_site_guarantees_its_fallback_output() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();
    let site = RuntimeSlotSiteId(0);
    let wrapper = OwnedRuntimeTemplateNode::Sequence {
        children: vec![OwnedRuntimeTemplateNode::RuntimeSlotSite { site, span: None }],
        span: None,
    };
    let slot_sites = vec![OwnedRuntimeSlotSite {
        site,
        render_root: constant_text_node("fallback"),
        span: None,
    }];

    assert!(
        super::owned_runtime_template_node_guarantees_output(
            &wrapper,
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            Some(slot_sites.as_slice())
        ),
        "an unresolved slot occurrence that carries a constant fallback guarantees that output"
    );
}

#[test]
fn routed_contribution_slot_site_stays_unproven() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();
    let site = RuntimeSlotSiteId(0);
    let wrapper = OwnedRuntimeTemplateNode::Sequence {
        children: vec![OwnedRuntimeTemplateNode::RuntimeSlotSite { site, span: None }],
        span: None,
    };
    let slot_sites = vec![OwnedRuntimeSlotSite {
        site,
        render_root: OwnedRuntimeTemplateNode::RuntimeSlotContributionSource {
            source: RuntimeSlotContributionSourceId(0),
            span: None,
        },
        span: None,
    }];

    assert!(
        !super::owned_runtime_template_node_guarantees_output(
            &wrapper,
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            Some(slot_sites.as_slice())
        ),
        "a routed contribution stays conditional even when its wrapper has a resolved site"
    );
}

#[test]
fn nested_slot_application_body_uses_wrapper_output_proof() {
    let string_table = StringTable::new();
    let (expressions, const_values, anchor) = expression_fixture();
    let handoff =
        slot_application_with_false_conditional_source(plain_text_node("<shell>"), anchor);
    let template = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::RuntimeSlotApplication(Box::new(handoff)),
    };
    let child = OwnedRuntimeTemplateNode::ChildTemplate {
        template: Box::new(template.clone()),
        span: None,
    };

    assert!(super::owned_runtime_template_node_guarantees_output(
        &child,
        expressions.view(anchor).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
    assert!(super::runtime_template_handoff_guarantees_output(
        &template,
        expressions.view(anchor).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
}

#[test]
fn nested_slot_application_expression_uses_wrapper_dynamic_output_proof() {
    let mut string_table = StringTable::new();
    let (mut expressions, mut const_values, anchor) = expression_fixture();
    let dynamic_text = expressions
        .string_slice(
            &mut const_values,
            string_table.intern("dynamic wrapper text"),
            None,
            ValueMode::default(),
        )
        .unwrap()
        .id();
    let wrapper = OwnedRuntimeTemplateNode::DynamicExpression {
        expression: dynamic_text,
        span: None,
    };
    let expression = runtime_slot_application_handoff_expr(
        &mut expressions,
        slot_application_with_false_conditional_source(wrapper, anchor),
        None,
        ValueMode::default(),
    );

    assert!(super::dynamic_expression_guarantees_output(
        expressions.view(expression).unwrap(),
        &const_values,
        &string_table,
        None,
    ));
}

#[test]
fn nested_slot_application_with_output_free_wrapper_stays_unproven() {
    let string_table = StringTable::new();
    let (mut expressions, const_values, anchor) = expression_fixture();

    for wrapper in [
        plain_text_node(""),
        plain_text_node(" \n\t"),
        OwnedRuntimeTemplateNode::Slot { span: None },
    ] {
        let handoff = slot_application_with_false_conditional_source(wrapper, anchor);
        let template = OwnedRuntimeTemplateHandoff {
            body: OwnedRuntimeTemplateBody::RuntimeSlotApplication(Box::new(handoff.clone())),
        };
        let child = OwnedRuntimeTemplateNode::ChildTemplate {
            template: Box::new(template.clone()),
            span: None,
        };
        let expression = runtime_slot_application_handoff_expr(
            &mut expressions,
            handoff,
            None,
            ValueMode::default(),
        );

        assert!(!super::owned_runtime_template_node_guarantees_output(
            &child,
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ));
        assert!(!super::runtime_template_handoff_guarantees_output(
            &template,
            expressions.view(anchor).unwrap(),
            &const_values,
            &string_table,
            None,
        ));
        assert!(!super::dynamic_expression_guarantees_output(
            expressions.view(expression).unwrap(),
            &const_values,
            &string_table,
            None,
        ));
    }
}
