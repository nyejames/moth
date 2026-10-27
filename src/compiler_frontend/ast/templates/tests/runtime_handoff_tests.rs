//! Neutral handoff graph-edge enumeration and immutable expression sharing.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource, OwnedRuntimeSlotSite,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn dynamic(expression: ExprId) -> OwnedRuntimeTemplateNode {
    OwnedRuntimeTemplateNode::DynamicExpression {
        expression,
        span: None,
    }
}

#[test]
fn retained_handoff_edges_include_nested_values_selectors_headers_and_slot_roots() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let original = expressions
        .bool(&mut values, true, None, ValueMode::ImmutableOwned)
        .expect("test bool is valid")
        .id();
    let replacement = expressions
        .bool(&mut values, false, None, ValueMode::ImmutableOwned)
        .expect("test bool is valid")
        .id();
    let slot_application = OwnedRuntimeSlotApplicationHandoff {
        wrapper: dynamic(original),
        contribution_sources: vec![OwnedRuntimeSlotContributionSource {
            source: RuntimeSlotContributionSourceId(0),
            render_root: dynamic(original),
            renders_wrapper_unconditionally: false,
        }],
        slot_sites: vec![OwnedRuntimeSlotSite {
            site: RuntimeSlotSiteId(0),
            render_root: dynamic(original),
            span: None,
        }],
    };
    let mut handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: vec![
                OwnedRuntimeTemplateNode::Conditional {
                    selector: Box::new(TemplateBranchSelector::Bool(original)),
                    body: Box::new(dynamic(original)),
                    span: None,
                },
                OwnedRuntimeTemplateNode::Loop {
                    header: TemplateLoopHeader::Conditional {
                        condition: original,
                    },
                    body: Box::new(dynamic(original)),
                    aggregate_wrapper: Some(Box::new(dynamic(original))),
                    span: None,
                },
                OwnedRuntimeTemplateNode::ChildTemplate {
                    template: Box::new(OwnedRuntimeTemplateHandoff {
                        body: OwnedRuntimeTemplateBody::RuntimeSlotApplication(Box::new(
                            slot_application,
                        )),
                    }),
                    span: None,
                },
            ],
            span: None,
        }),
    };

    let mut observed = Vec::new();
    handoff.visit_expression_ids(&mut |id| observed.push(id));
    assert_eq!(
        observed,
        vec![original; 8],
        "all authored expression edges are retained, including repeated shared IDs"
    );
    handoff.visit_expression_ids_mut(&mut |id| *id = replacement);
    observed.clear();
    handoff.visit_expression_ids(&mut |id| observed.push(id));
    assert_eq!(
        observed,
        vec![replacement; 8],
        "the consuming remapper reaches every retained edge"
    );
    assert_eq!(
        expressions.len(),
        2,
        "remapping handoff roots does not copy expression rows"
    );
}
