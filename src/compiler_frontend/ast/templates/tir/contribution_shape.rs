//! TIR-native child-contribution classification.
//!
//! `ContributionShape` classifies a single TIR contribution node as a
//! potential child-template contribution, capturing whether it represents
//! child output and whether it opts out of parent `$children(..)` wrappers.

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
