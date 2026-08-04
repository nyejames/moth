//! Transient indexed views over one completed public interface.
//!
//! WHAT: builds direct lookup indexes over the compact vector-backed fields of one
//! [`PublicSemanticInterface`] for one closure or binding operation, then drops with that
//! operation.
//! WHY: repeated linear searches over declarations, callable summaries, evidence and export
//! bindings make closure and re-export binding quadratic in the number of provider facts. The
//! completed interface stays vector-backed, deterministic and free of durable lookup maps; the
//! view is never stored inside it.

use super::model::{
    PublicBindingExport, PublicDeclarationRecord, PublicDiagnosticLocation, PublicEvidenceRecord,
    PublicSemanticInterface,
};
use crate::compiler_frontend::canonical_type_identity::CanonicalEvidenceIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginFunctionId};

use rustc_hash::FxHashMap;

fn view_error(detail: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(format!("public semantic interface view: {}", detail.into()))
}

/// One operation-scoped record view over an interface used by recursive closure.
///
/// WHAT: indexes only the declaration, concrete summary and reusable evidence records closure
///       needs, with duplicate-key validation while it builds.
/// WHY: closure walks many interfaces; a narrow record view keeps lookups direct without
///      carrying binding-only export maps into the closure operation.
pub(crate) struct ClosureRecordView<'a> {
    interface: &'a PublicSemanticInterface,
    declaration_by_origin: FxHashMap<OriginDeclarationId, usize>,
    summary_by_origin: FxHashMap<OriginFunctionId, usize>,
    evidence_by_identity: FxHashMap<CanonicalEvidenceIdentity, usize>,
}

impl<'a> ClosureRecordView<'a> {
    pub(crate) fn build(interface: &'a PublicSemanticInterface) -> Result<Self, CompilerError> {
        let mut view = Self {
            interface,
            declaration_by_origin: FxHashMap::default(),
            summary_by_origin: FxHashMap::default(),
            evidence_by_identity: FxHashMap::default(),
        };

        for (index, declaration) in interface.declarations.iter().enumerate() {
            Self::insert_unique(
                interface,
                &mut view.declaration_by_origin,
                declaration.origin.clone(),
                index,
                "declaration origin",
                || format!("{:?}", declaration.origin),
            )?;
        }
        for (index, summary) in interface.concrete_call_summaries.iter().enumerate() {
            Self::insert_unique(
                interface,
                &mut view.summary_by_origin,
                summary.origin.clone(),
                index,
                "concrete summary origin",
                || format!("{:?}", summary.origin),
            )?;
        }
        for (index, evidence) in interface.reusable_evidence.iter().enumerate() {
            Self::insert_unique(
                interface,
                &mut view.evidence_by_identity,
                evidence.identity.clone(),
                index,
                "evidence identity",
                || format!("{:?}", evidence.identity),
            )?;
        }

        Ok(view)
    }

    fn insert_unique<K: std::hash::Hash + Eq>(
        interface: &PublicSemanticInterface,
        index: &mut FxHashMap<K, usize>,
        key: K,
        record_index: usize,
        key_class: &str,
        render_key: impl FnOnce() -> String,
    ) -> Result<(), CompilerError> {
        if index.insert(key, record_index).is_some() {
            return Err(view_error(format!(
                "duplicate {key_class} {:?} in interface {:?}",
                render_key(),
                interface.module_origin
            )));
        }

        Ok(())
    }

    pub(crate) fn declaration(
        &self,
        origin: &OriginDeclarationId,
    ) -> Option<&PublicDeclarationRecord> {
        self.declaration_by_origin
            .get(origin)
            .map(|index| &self.interface.declarations[*index])
    }

    pub(crate) fn concrete_call_summary(
        &self,
        origin: &OriginFunctionId,
    ) -> Option<&PublicCallSummary> {
        self.summary_by_origin
            .get(origin)
            .map(|index| &self.interface.concrete_call_summaries[*index].summary)
    }

    pub(crate) fn evidence(
        &self,
        identity: &CanonicalEvidenceIdentity,
    ) -> Option<&PublicEvidenceRecord> {
        self.evidence_by_identity
            .get(identity)
            .map(|index| &self.interface.reusable_evidence[*index])
    }

    /// The completed interface this view indexes.
    pub(crate) fn interface(&self) -> &'a PublicSemanticInterface {
        self.interface
    }
}

/// One operation-scoped binding view over a completed provider interface.
///
/// WHAT: indexes only the facts header binding needs: public export names, binding-export
///       names, declaration origins, concrete summary origins and export diagnostic provenance.
///       Keys borrow the interface's own stable strings instead of allocating owned copies.
/// WHY: header binding resolves many shells against one provider; a narrow borrowed view keeps
///      those lookups direct without duplicating the broader closure view or owned string keys.
#[derive(Debug)]
pub(crate) struct ProviderBindingView<'a> {
    interface: &'a PublicSemanticInterface,
    export_by_name: FxHashMap<&'a str, usize>,
    binding_export_by_name: FxHashMap<&'a str, usize>,
    declaration_by_origin: FxHashMap<OriginDeclarationId, usize>,
    summary_by_origin: FxHashMap<OriginFunctionId, usize>,
    provenance_by_name: FxHashMap<&'a str, usize>,
}

impl<'a> ProviderBindingView<'a> {
    pub(crate) fn build(interface: &'a PublicSemanticInterface) -> Result<Self, CompilerError> {
        let mut view = Self {
            interface,
            export_by_name: FxHashMap::default(),
            binding_export_by_name: FxHashMap::default(),
            declaration_by_origin: FxHashMap::default(),
            summary_by_origin: FxHashMap::default(),
            provenance_by_name: FxHashMap::default(),
        };

        for (index, binding) in interface.export_bindings.iter().enumerate() {
            Self::insert_unique(
                &mut view.export_by_name,
                binding.public_name(),
                index,
                "export name",
            )?;
        }
        for (index, binding) in interface.binding_exports.iter().enumerate() {
            Self::insert_unique(
                &mut view.binding_export_by_name,
                binding.public_name.as_str(),
                index,
                "binding export name",
            )?;
        }
        for (index, declaration) in interface.declarations.iter().enumerate() {
            Self::insert_unique(
                &mut view.declaration_by_origin,
                declaration.origin.clone(),
                index,
                "declaration origin",
            )?;
        }
        for (index, summary) in interface.concrete_call_summaries.iter().enumerate() {
            Self::insert_unique(
                &mut view.summary_by_origin,
                summary.origin.clone(),
                index,
                "concrete summary origin",
            )?;
        }
        for (index, entry) in interface.export_diagnostic_provenance.iter().enumerate() {
            Self::insert_unique(
                &mut view.provenance_by_name,
                entry.public_name.as_str(),
                index,
                "export diagnostic provenance",
            )?;
        }

        Ok(view)
    }

    fn insert_unique<K: std::hash::Hash + Eq>(
        index: &mut FxHashMap<K, usize>,
        key: K,
        record_index: usize,
        key_class: &str,
    ) -> Result<(), CompilerError> {
        if index.insert(key, record_index).is_some() {
            return Err(view_error(format!(
                "duplicate {key_class} in provider binding view"
            )));
        }

        Ok(())
    }

    pub(crate) fn exported_origin(&self, public_name: &str) -> Option<&OriginDeclarationId> {
        self.export_by_name
            .get(public_name)
            .map(|index| self.interface.export_bindings[*index].origin())
    }

    pub(crate) fn binding_export(&self, public_name: &str) -> Option<&PublicBindingExport> {
        self.binding_export_by_name
            .get(public_name)
            .map(|index| &self.interface.binding_exports[*index])
    }

    pub(crate) fn declaration(
        &self,
        origin: &OriginDeclarationId,
    ) -> Option<&PublicDeclarationRecord> {
        self.declaration_by_origin
            .get(origin)
            .map(|index| &self.interface.declarations[*index])
    }

    pub(crate) fn concrete_call_summary(
        &self,
        origin: &OriginFunctionId,
    ) -> Option<&PublicCallSummary> {
        self.summary_by_origin
            .get(origin)
            .map(|index| &self.interface.concrete_call_summaries[*index].summary)
    }

    pub(crate) fn export_diagnostic_provenance(
        &self,
        public_name: &str,
    ) -> Option<&PublicDiagnosticLocation> {
        self.provenance_by_name
            .get(public_name)
            .map(|index| &self.interface.export_diagnostic_provenance[*index].location)
    }
}
