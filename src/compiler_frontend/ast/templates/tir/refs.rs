//! Durable module-local TIR references.
//!
//! WHAT: stores the root, phase, and value-carried context needed to resolve a
//! template value inside one module-scoped [`TemplateIrStore`].
//! WHY: every TIR reference is local to the AST module that owns its store, so
//! no store qualification is needed to resolve it.

use std::fmt;

pub(crate) use super::ids::TemplateIrId;
use super::overlays::TemplateViewContext;
use super::view::TemplateTirPhase;

/// Durable reference to a finalized parser-emitted TIR root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateTirReference {
    pub(crate) root: TemplateIrId,
    pub(crate) phase: TemplateTirPhase,
    pub(crate) context: TemplateViewContext,
}

/// Module-local identity for a child-template occurrence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateTirChildReference {
    pub(crate) root: TemplateIrId,
    pub(crate) phase: TemplateTirPhase,
    pub(crate) context: TemplateViewContext,
}

impl TemplateTirChildReference {
    pub(crate) fn new(
        root: TemplateIrId,
        phase: TemplateTirPhase,
        context: TemplateViewContext,
    ) -> Self {
        Self {
            root,
            phase,
            context,
        }
    }

    pub(crate) fn with_root(self, root: TemplateIrId) -> Self {
        Self { root, ..self }
    }

    pub(crate) fn with_context(self, context: TemplateViewContext) -> Self {
        Self { context, ..self }
    }
}

/// Effective identity for a wrapper template in a wrapper set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TemplateWrapperReference {
    pub(crate) root: TemplateIrId,
    pub(crate) phase: TemplateTirPhase,
    pub(crate) context: TemplateViewContext,
}

impl TemplateWrapperReference {
    pub(crate) fn new(
        root: TemplateIrId,
        phase: TemplateTirPhase,
        context: TemplateViewContext,
    ) -> Self {
        Self {
            root,
            phase,
            context,
        }
    }

    /// Converts an unchanged wrapper reference into a structural child reference.
    pub(crate) fn into_structural_child_reference(self) -> TemplateTirChildReference {
        TemplateTirChildReference::new(self.root, self.phase, self.context)
    }

    /// Converts a completed slot application into its exact composed child reference.
    ///
    /// Slot resolution belongs to the application that consumed it, not to the derived wrapper
    /// root. A parsed wrapper cannot authorise structural overlays merely because composition
    /// advances the derived root to `Composed`. Expression overlays also remain owned by the
    /// surrounding structural view rather than becoming wrapper authority.
    pub(crate) fn into_composed_child_reference(
        self,
        derived_root: TemplateIrId,
    ) -> TemplateTirChildReference {
        let wrapper_context = self
            .phase
            .is_at_least(TemplateTirPhase::Composed)
            .then_some(self.context.wrapper_context)
            .flatten();

        TemplateTirChildReference::new(
            derived_root,
            TemplateTirPhase::Composed,
            TemplateViewContext {
                expression_overlay: None,
                slot_resolution: None,
                wrapper_context,
            },
        )
    }
}

impl fmt::Display for TemplateWrapperReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TemplateWrapperReference({}, phase={:?}, context={:?})",
            self.root, self.phase, self.context
        )
    }
}
