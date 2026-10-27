//! One diagnostic as the render boundary reads it.
//!
//! WHAT: answers the questions every renderer asks of a diagnostic (severity, descriptor, primary
//! position, resolved labels and payload) from one frozen report view.
//! WHY: the terminal, terse and dev-server renderers read every record through this seam, so they
//! share one reading of each record.

use super::context::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, ResolvedDiagnosticLabel,
    compact_label_message_text, resolved_label_fact,
};
use super::payload::RenderedPayload;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{DiagnosticDescriptor, DiagnosticSeverity};
use std::path::Path;

/// One diagnostic prepared for a render entry.
///
/// WHAT: wraps the frozen view of one record. Every read in the same render entry goes through
/// this value, so severity, descriptor, position, labels and payload all come from one record.
/// WHY: each renderer reads the record the same way; no read bypasses the frozen view.
pub(crate) struct DiagnosticSubject<'a> {
    view: DiagnosticView<'a>,
}

impl<'a> DiagnosticSubject<'a> {
    /// Wraps the view of one record from a frozen report.
    pub(crate) fn new(view: DiagnosticView<'a>) -> Self {
        Self { view }
    }

    pub(crate) fn severity(&self) -> DiagnosticSeverity {
        self.view.severity()
    }

    pub(crate) fn descriptor(&self) -> DiagnosticDescriptor {
        self.view.family().descriptor()
    }

    /// The primary source position, resolved through the primary site's identity handle.
    pub(crate) fn primary_position<'r>(
        &'r self,
        context: DiagnosticRenderContext<'r>,
    ) -> Option<DiagnosticPrimaryPosition<'r>> {
        context.site_position(self.view.primary_site())
    }

    /// The secondary labels that have a resolvable span or a message, in declaration order.
    pub(crate) fn resolved_labels(
        &self,
        context: DiagnosticRenderContext<'_>,
        root: &Path,
    ) -> Vec<ResolvedDiagnosticLabel> {
        let types = self.view.types();
        self.view
            .durable_labels()
            .filter_map(|label| {
                resolved_label_fact(
                    label.spanned,
                    context.site_position(label.site),
                    compact_label_message_text(label.message, types, context),
                    root,
                )
            })
            .collect()
    }

    /// The primary message and guidance. The record's family row renders them.
    pub(crate) fn payload(&self, context: DiagnosticRenderContext<'_>) -> RenderedPayload {
        let render = self
            .view
            .family()
            .render
            .expect("matched family rows declare a renderer");
        render(&self.view, context)
    }
}
