//! Test-only fixtures for frozen materialisation contexts.
//!
//! WHAT: constructs body-less materialisation artefacts and checks the retained template rows
//!       used by materialisation regression tests.
//! WHY: these fixtures need the context's private stable fields, so they live below the owning
//!      artefact module instead of widening production visibility or adding test APIs there.

use super::super::frozen_syntax::{StableBodyOwner, StableBodySyntax};
use super::super::semantic_closure::StableSemanticClosure;
use super::super::stable_types::{
    ConstFunctionArtefact, GenericTemplateArtefact, StableFunctionReturnContract,
    StableFunctionSignature, StableReturns,
};
use super::super::visibility::StableFileVisibility;
use super::ModuleMaterialisationContext;
use crate::compiler_frontend::semantic_identity::{GeneratedDeclarationIdentity, OriginFunctionId};
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::style_directives::{ConstraintDirectiveSpans, SourceConstraintSet};
use crate::compiler_frontend::symbols::path_interner::PathId;
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::symbols::path_interner::PathTable;
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::symbols::string_interning::FrozenStringTable;
use crate::compiler_frontend::tokenizer::tokens::TestSourceTokensBuilder;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// Build one body-less stable syntax owner in the synthetic compilation-root source.
fn empty_body_for_test(frozen_identity_handle: &FrozenIdentityHandle) -> StableBodySyntax {
    let empty_source_owner =
        TestSourceTokensBuilder::new(crate::compiler_frontend::source::SourceId::COMPILATION_ROOT)
            .finish()
            .expect("canonical empty source fixture should finish");
    let empty_token_range = empty_source_owner
        .full_range()
        .expect("empty canonical source should have a checked full range");
    StableBodySyntax {
        declaration_path: PathId::ROOT,
        donor_file_id: crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
        frozen_identity_handle: frozen_identity_handle.clone(),
        source_owner: StableBodyOwner {
            source_tokens: Arc::clone(&empty_source_owner),
        },
        token_range: empty_token_range,
        token_sequence: None,
        source_path_table: None,
        source_string_table: None,
        resolved_file_references: Box::new([]),
        qualified_references: Box::new([]),
    }
}

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
                function_path: PathId::ROOT,
                source_file: PathId::ROOT,
                declaration_span: None,
                source_constraints: SourceConstraintSet::default(),
                constraint_directive_spans: ConstraintDirectiveSpans::default(),
                body: empty_body_for_test(&frozen_identity_handle),
                signature: StableFunctionSignature {
                    parameters: Box::new([]),
                    return_contract: StableFunctionReturnContract::Returns(StableReturns {
                        success: Box::new([]),
                        error: None,
                    }),
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
            concrete_call_summaries: Box::new([]),
            semantic_closure: StableSemanticClosure::default(),
            artefacts,
            const_functions: Box::new([]),
            module_origin: None,
            frozen_identity_handle,
            path_table: None,
            source_string_table: None,
            const_path_table: None,
            const_source_string_table: None,
        }
    }

    /// Build a test-only context with the given const rows and no generic artefacts.
    ///
    /// `None` marks a retained private helper row; `Some` marks an exported public row.
    pub(crate) fn from_const_origins_for_test(origins: Vec<Option<OriginFunctionId>>) -> Self {
        let mut context = Self::from_identities_for_test(Vec::new());
        let frozen_identity_handle = context.frozen_identity_handle.clone();
        context.const_functions = origins
            .into_iter()
            .map(|origin| ConstFunctionArtefact {
                origin,
                function_path: PathId::ROOT,
                body: empty_body_for_test(&frozen_identity_handle),
                signature: StableFunctionSignature {
                    parameters: Box::new([]),
                    return_contract: StableFunctionReturnContract::Returns(StableReturns {
                        success: Box::new([]),
                        error: None,
                    }),
                },
                visibility: StableFileVisibility::default(),
                declarations: Box::new([]),
                constants: Box::new([]),
                aliases: Box::new([]),
                closure: Box::new([]),
                callables: Box::new([]),
                nominals: Box::new([]),
                nominal_blueprints: FxHashMap::default(),
            })
            .collect::<Box<[_]>>();
        context
    }

    /// Build one test-only const context that retains the tables which issued its rows.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn from_const_origins_with_identity_for_test(
        origins: Vec<Option<OriginFunctionId>>,
        path_table: Arc<PathTable>,
        source_string_table: Arc<FrozenStringTable>,
    ) -> Self {
        let mut context = Self::from_const_origins_for_test(origins);
        context.const_path_table = Some(path_table);
        context.const_source_string_table = Some(source_string_table);
        context
    }

    pub(crate) fn contains_template(&self, identity: &GeneratedDeclarationIdentity) -> bool {
        self.artefacts
            .iter()
            .any(|artefact| &artefact.declaration_identity == identity)
    }
}
