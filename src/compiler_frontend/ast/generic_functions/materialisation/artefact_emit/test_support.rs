//! Test-only fixtures for frozen materialisation contexts.
//!
//! WHAT: constructs body-less materialisation artefacts and checks the retained template rows
//!       used by materialisation regression tests.
//! WHY: these fixtures need the context's private stable fields, so they live below the owning
//!      artefact module instead of widening production visibility or adding test APIs there.

use super::super::frozen_syntax::StableBodySyntax;
use super::super::semantic_closure::StableSemanticClosure;
use super::super::stable_types::{GenericTemplateArtefact, StableFunctionSignature};
use super::super::visibility::StableFileVisibility;
use super::ModuleMaterialisationContext;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use rustc_hash::FxHashMap;

impl ModuleMaterialisationContext {
    /// Build a test-only context with one artefact per identity and no real body payload.
    pub(crate) fn from_identities_for_test(identities: Vec<GeneratedDeclarationIdentity>) -> Self {
        let frozen_identity_handle = FrozenIdentityHandle::new();
        let artefacts = identities
            .into_iter()
            .map(|declaration_identity| GenericTemplateArtefact {
                declaration_identity,
                generic_parameter_owner: None,
                receiver: None,
                receiver_nominal_identity: None,
                function_path: Box::new([]),
                source_file: Box::new([]),
                declaration_span: None,
                body: StableBodySyntax {
                    declaration_path: Box::new([]),
                    donor_file_id: crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
                    frozen_identity_handle: frozen_identity_handle.clone(),
                    pool: Box::new([]),
                    tokens: Box::new([]),
                    path_syntax: PathSyntaxTable::default(),
                    resolved_file_references: Box::new([]),
                },
                signature: StableFunctionSignature {
                    parameters: Box::new([]),
                    returns: Box::new([]),
                },
                generic_parameters: Box::new([]),
                visibility: StableFileVisibility::default(),
                declarations: Box::new([]),
                local_declarations: Box::new([]),
                callables: Box::new([]),
                nominals: Box::new([]),
                nominal_blueprints: FxHashMap::default(),
            })
            .collect::<Box<[_]>>();
        Self {
            declaration_closure: Box::new([]),
            evidence: Box::new([]),
            semantic_closure: StableSemanticClosure::default(),
            artefacts,
            module_origin: None,
            frozen_identity_handle,
        }
    }

    pub(crate) fn contains_template(&self, identity: &GeneratedDeclarationIdentity) -> bool {
        self.artefacts
            .iter()
            .any(|artefact| &artefact.declaration_identity == identity)
    }
}
