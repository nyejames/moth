//! One diagnostic as the render boundary reads it.
//!
//! WHAT: answers the questions every renderer asks of a diagnostic (severity, descriptor, primary
//! position, resolved labels and payload) from one value per render entry.
//! WHY: pilot families render from their compact record while every other family renders from the
//! legacy value. Routing both through one seam keeps the terminal, terse and dev-server renderers
//! independent of that choice, so P4.6 can delete the legacy path without touching them.

use super::context::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, ResolvedDiagnosticLabel,
    compact_label_message_text, resolve_label_render_facts_from_root, resolved_label_fact,
};
use super::payload::{RenderedPayload, render_payload};
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::{
    CompactRender, compact_for_render,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticDescriptor, DiagnosticLabelStyle, DiagnosticSeverity,
};
use std::path::Path;

/// A diagnostic prepared for one render entry.
///
/// WHAT: holds the legacy value and, for a pilot family, its compact record. The compact record is
/// built once here, so every read in the same render entry sees the same record.
/// WHY: compacting per read would repeat the work and let two reads of one diagnostic disagree.
pub(crate) struct DiagnosticSubject<'d> {
    diagnostic: &'d CompilerDiagnostic,
    compact: Option<CompactRender>,
}

impl<'d> DiagnosticSubject<'d> {
    /// Prepares `diagnostic` for rendering. A pilot family compacts here; every other family keeps
    /// the legacy value.
    pub(crate) fn new(
        diagnostic: &'d CompilerDiagnostic,
        context: DiagnosticRenderContext<'_>,
    ) -> Self {
        Self {
            diagnostic,
            compact: compact_for_render(diagnostic, context),
        }
    }

    pub(crate) fn severity(&self) -> DiagnosticSeverity {
        match &self.compact {
            Some(compact) => compact.view().severity(),
            None => self.diagnostic.severity,
        }
    }

    pub(crate) fn descriptor(&self) -> DiagnosticDescriptor {
        match &self.compact {
            Some(compact) => compact.view().family().pilot_descriptor(),
            None => self.diagnostic.kind.descriptor(),
        }
    }

    /// The primary source position. A compact primary resolves through its site; a legacy one
    /// resolves through the requester context.
    pub(crate) fn primary_position<'r>(
        &'r self,
        context: DiagnosticRenderContext<'r>,
    ) -> Option<DiagnosticPrimaryPosition<'r>> {
        match &self.compact {
            Some(compact) => context.site_position(compact.view().primary_site()),
            None => context.primary_position(self.diagnostic),
        }
    }

    /// The secondary labels that have a resolvable span or a message, in declaration order.
    pub(crate) fn resolved_labels(
        &self,
        context: DiagnosticRenderContext<'_>,
        root: &Path,
    ) -> Vec<ResolvedDiagnosticLabel> {
        let Some(compact) = &self.compact else {
            return resolve_label_render_facts_from_root(self.diagnostic, context, root);
        };
        let view = compact.view();
        let types = view.types();
        view.durable_labels()
            .filter_map(|label| {
                resolved_label_fact(
                    DiagnosticLabelStyle::Secondary,
                    label.spanned,
                    context.site_position(label.site),
                    compact_label_message_text(label.message, types, context),
                    root,
                )
            })
            .collect()
    }

    /// The primary message and guidance. A pilot family renders through its compact renderer.
    pub(crate) fn payload(&self, context: DiagnosticRenderContext<'_>) -> RenderedPayload {
        match &self.compact {
            Some(compact) => {
                let view = compact.view();
                let render = view.family().render.expect("pilot rows declare a renderer");
                render(&view, context)
            }
            None => render_payload(&self.diagnostic.payload, context),
        }
    }
}
