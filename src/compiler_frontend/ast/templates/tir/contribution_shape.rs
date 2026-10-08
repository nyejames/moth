//! TIR-native child-contribution classification.
//!
//! `ContributionShape` classifies a single TIR contribution node as a
//! potential child-template contribution, capturing whether it represents
//! child output and whether it opts out of parent `$children(..)` wrappers.
//! Control-flow-root classification stays alongside these contribution facts
//! so slot composition and runtime sites share the same decision.

use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// Classification of a contribution's relationship to child-template wrapping.
///
/// Non-child contributions cannot opt out of parent wrappers. That combination
/// is unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContributionShape {
    Child { skips_parent_child_wrappers: bool },
    Other,
}

impl ContributionShape {
    pub(crate) fn is_child_template_contribution(self) -> bool {
        matches!(self, Self::Child { .. })
    }

    pub(crate) fn skips_parent_child_wrappers(self) -> bool {
        matches!(
            self,
            Self::Child {
                skips_parent_child_wrappers: true
            }
        )
    }
}

/// Classifies a TIR contribution node for child-contribution purposes.
pub(crate) fn classify_tir_contribution_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Result<ContributionShape, CompilerError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR contribution classification: contribution node ID was not present in the store.",
        )
    })?;

    let shape = match &node.kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template = store.get_template(reference.root).ok_or_else(|| {
                CompilerError::compiler_error(
                    "TIR contribution classification: child template ID was not present in the store.",
                )
            })?;

            ContributionShape::Child {
                skips_parent_child_wrappers: template.style.skip_parent_child_wrappers,
            }
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let referenced_template = store.get_template(*template).ok_or_else(|| {
                CompilerError::compiler_error(
                    "TIR contribution classification: insert contribution template ID was not present in the store.",
                )
            })?;

            ContributionShape::Child {
                skips_parent_child_wrappers: referenced_template.style.skip_parent_child_wrappers,
            }
        }

        _ => ContributionShape::Other,
    };

    Ok(shape)
}

/// Returns whether a contribution root makes output conditional on template control flow.
///
/// Direct conditional and loop roots are conditional. A child-template reference is
/// conditional when its owned subtree contains control flow. Other contribution
/// shapes retain their ordinary wrapper behavior.
pub(crate) fn tir_node_is_control_flow_root(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Result<bool, CompilerError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR contribution control-flow classification: contribution node ID was not present in the store.",
        )
    })?;

    match &node.kind {
        TemplateIrNodeKind::Conditional { .. } | TemplateIrNodeKind::Loop { .. } => Ok(true),
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template = store.get_template(reference.root).ok_or_else(|| {
                CompilerError::compiler_error(
                    "TIR contribution control-flow classification: child template ID was not present in the store.",
                )
            })?;

            Ok(store
                .control_flow_node_id_in_subtree(template.root)?
                .is_some())
        }
        _ => Ok(false),
    }
}
