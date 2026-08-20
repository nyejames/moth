//! Published declaring-module materialisation contexts available to one compilation boundary.
//!
//! WHAT: an identity-keyed registry of the generic templates already completed providers exposed,
//!       plus the resolved template one generated request will materialise from.
//! WHY:  materialising a concrete generic needs the *declaring* module's validated template, which
//!       may live in an earlier project module or a completed source package. The build system
//!       populates this registry as it publishes each provider, then lends it to the compiler for
//!       the duration of one module transaction. The compiler resolves templates from it and never
//!       reaches into a build-system store.

use crate::compiler_frontend::ast::generic_functions::{
    ModuleMaterialisationContext, ModuleMaterialisationPreparation,
};
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;

use rustc_hash::FxHashMap;
use std::sync::Arc;

/// One published declaring-module context and the template index it owns for a declaration.
#[derive(Clone)]
pub(crate) struct PublishedMaterialisation {
    pub(crate) context: Arc<ModuleMaterialisationContext>,
    pub(crate) template_index: usize,
}

/// Every declaring-module materialisation template published in one compilation boundary.
///
/// Entries are shared by `Arc`, so the registry keeps resolving correctly as the build system's
/// artefact storage grows behind it.
#[derive(Default)]
pub(crate) struct ProviderMaterialisationRegistry {
    published: FxHashMap<GeneratedDeclarationIdentity, PublishedMaterialisation>,
}

impl ProviderMaterialisationRegistry {
    /// Record one published declaring context for a generic declaration it owns.
    ///
    /// WHY: one identity resolves to exactly one declaring context. `GeneratedDeclarationIdentity`
    ///      carries its owning package and module origin, and every producer proves uniqueness
    ///      before publishing here: the module store rejects an identity published by two
    ///      materialisation contexts, the package registry rejects one published by two packages,
    ///      and `ProjectFrontendCompilation::new` rejects one published by both lanes. An insert
    ///      that replaced an existing entry would mean one of those proofs had already failed.
    pub(crate) fn publish(
        &mut self,
        identity: GeneratedDeclarationIdentity,
        context: Arc<ModuleMaterialisationContext>,
        template_index: usize,
    ) {
        self.published.insert(
            identity,
            PublishedMaterialisation {
                context,
                template_index,
            },
        );
    }

    fn published_template(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<&PublishedMaterialisation> {
        self.published.get(identity)
    }
}

/// Where one generated request finds the template it must materialise.
///
/// A request usually resolves to a completed provider, but a module may also instantiate its own
/// generic before that module has been published. That requester-local case stays a compiler-local
/// case rather than making the build system fake a completed provider.
pub(crate) enum DeclaringMaterialisation<'a> {
    Published {
        context: &'a ModuleMaterialisationContext,
        template_index: usize,
    },
    Preparing(&'a ModuleMaterialisationPreparation),
}

/// Resolve the declaring template for one generated request.
pub(crate) fn declaring_materialisation<'a>(
    registry: &'a ProviderMaterialisationRegistry,
    identity: &GeneratedDeclarationIdentity,
    requester_context: &'a ModuleMaterialisationPreparation,
) -> Option<DeclaringMaterialisation<'a>> {
    if let Some(published) = registry.published_template(identity) {
        return Some(DeclaringMaterialisation::Published {
            context: published.context.as_ref(),
            template_index: published.template_index,
        });
    }

    requester_context
        .template_for_identity(identity)
        .is_some()
        .then_some(DeclaringMaterialisation::Preparing(requester_context))
}
