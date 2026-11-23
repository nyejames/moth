//! Published declaring-module materialisation contexts available to one compilation boundary.
//!
//! WHAT: an identity-keyed registry of the generic templates and const function bodies already
//!       completed providers exposed, plus the resolved template one generated request will
//!       materialise from.
//! WHY:  materialising a concrete generic, or folding an imported const function, needs the
//!       *declaring* module's validated context, which may live in an earlier project module or a
//!       completed source package. The build system populates this registry as it publishes each
//!       provider, then lends it to the compiler for the duration of one module transaction. The
//!       compiler resolves templates from it and never reaches into a build-system store.

use crate::compiler_frontend::ast::generic_functions::{
    ModuleMaterialisationContext, ModuleMaterialisationPreparation,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::semantic_identity::{GeneratedDeclarationIdentity, OriginFunctionId};

use rustc_hash::FxHashMap;
use std::sync::Arc;

/// One published declaring-module context and the template index it owns for a declaration.
#[derive(Clone)]
pub(crate) struct PublishedMaterialisation {
    pub(crate) context: Arc<ModuleMaterialisationContext>,
    pub(crate) template_index: usize,
    /// Whether this context was published from a different identity domain.
    ///
    /// Same-boundary publications normally share the requester's path/string domain and can use
    /// the retained context directly. If the provider was published after the requester forked,
    /// the current boundary pair is supplied and materialisation rebases from that pair instead.
    /// Completed source-package seeds and any other cross-boundary publication re-intern by
    /// spelling through [`ModuleMaterialisationContext::rebased_for_requester`] for every request.
    pub(crate) rebase_required: bool,
    /// Completed package that published this row; `None` for the boundary's own rows.
    pub(crate) package_index: Option<usize>,
}

/// One published const function body row and the declaring context that owns it.
#[derive(Clone)]
pub(crate) struct PublishedConstFunction {
    pub(crate) context: Arc<ModuleMaterialisationContext>,
    pub(crate) row_index: usize,
    /// Whether the context was published from a different identity domain.
    ///
    /// A provider that retains its own const identity pair always rebases the selected closure
    /// through a synthetic declaring-domain mount. This marker is consulted only when that pair is
    /// absent: a cross-boundary seed then falls back to the generic spelling-based rebase, while a
    /// same-boundary publication can use its retained rows directly.
    pub(crate) rebase_required: bool,
    /// Completed package that published this row; `None` for the boundary's own rows.
    pub(crate) package_index: Option<usize>,
}

/// Immutable index of every materialisation row completed source packages expose.
///
/// WHAT: owns the same two identity maps as a boundary registry, built once by the completed
///       source-package registry and updated one package at a time.
/// WHY:  each canonical package needs the rows of every earlier package, but copying and
///       rehashing those rows per boundary grows quadratically. A boundary registry borrows this
///       shared index and keeps only its own publications in the local maps.
#[derive(Clone, Default)]
pub(crate) struct CompletedMaterialisationIndex {
    published: FxHashMap<GeneratedDeclarationIdentity, PublishedMaterialisation>,
    const_functions: FxHashMap<OriginFunctionId, PublishedConstFunction>,
}

impl CompletedMaterialisationIndex {
    pub(crate) fn reserve(&mut self, materialisations: usize, const_functions: usize) {
        self.published.reserve(materialisations);
        self.const_functions.reserve(const_functions);
    }

    /// Record one generic materialisation row a completed package published.
    ///
    /// A completed package finished before every later boundary forked, so its retained identity
    /// domain is always foreign to a requester and the row always needs the spelling-based
    /// rebase. The marker is not a parameter because no caller can legitimately clear it.
    pub(crate) fn insert_materialisation(
        &mut self,
        identity: GeneratedDeclarationIdentity,
        context: Arc<ModuleMaterialisationContext>,
        template_index: usize,
        package_index: usize,
    ) {
        self.published
            .entry(identity)
            .or_insert(PublishedMaterialisation {
                context,
                template_index,
                rebase_required: true,
                package_index: Some(package_index),
            });
    }

    /// Record one const function row a completed package published. See
    /// [`Self::insert_materialisation`] for why the rebase marker is always set.
    pub(crate) fn insert_const_function(
        &mut self,
        origin: OriginFunctionId,
        context: Arc<ModuleMaterialisationContext>,
        row_index: usize,
        package_index: usize,
    ) {
        self.const_functions
            .entry(origin)
            .or_insert(PublishedConstFunction {
                context,
                row_index,
                rebase_required: true,
                package_index: Some(package_index),
            });
    }

    fn materialisation(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<&PublishedMaterialisation> {
        self.published.get(identity)
    }

    fn const_function(&self, origin: &OriginFunctionId) -> Option<&PublishedConstFunction> {
        self.const_functions.get(origin)
    }

    /// Completed package that published one generic row.
    pub(crate) fn materialisation_owner_index(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<usize> {
        self.published
            .get(identity)
            .and_then(|row| row.package_index)
    }

    /// Completed package that published one const row.
    pub(crate) fn const_function_owner_index(&self, origin: &OriginFunctionId) -> Option<usize> {
        self.const_functions
            .get(origin)
            .and_then(|row| row.package_index)
    }
}

/// Every declaring-module materialisation row published in one compilation boundary.
///
/// Completed-package rows live in one shared [`CompletedMaterialisationIndex`]; only the
/// boundary's own rows occupy the local maps. Entries are shared by `Arc`, so lookups keep
/// resolving as the build system's artefact storage grows behind the registry.
#[derive(Default)]
pub(crate) struct ProviderMaterialisationRegistry {
    published: FxHashMap<GeneratedDeclarationIdentity, PublishedMaterialisation>,
    const_functions: FxHashMap<OriginFunctionId, PublishedConstFunction>,
    completed: Option<Arc<CompletedMaterialisationIndex>>,
}

impl ProviderMaterialisationRegistry {
    /// Share the immutable index of every completed package row.
    pub(crate) fn borrow_completed_index(&mut self, index: Arc<CompletedMaterialisationIndex>) {
        self.completed = Some(index);
    }

    /// Resolve one declaring template, preferring this boundary's own publication.
    fn published_template(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<&PublishedMaterialisation> {
        self.published.get(identity).or_else(|| {
            self.completed
                .as_deref()
                .and_then(|completed| completed.materialisation(identity))
        })
    }

    /// Resolve one const function body row, preferring this boundary's own publication.
    pub(crate) fn const_function(
        &self,
        origin: &OriginFunctionId,
    ) -> Option<&PublishedConstFunction> {
        self.const_functions.get(origin).or_else(|| {
            self.completed
                .as_deref()
                .and_then(|completed| completed.const_function(origin))
        })
    }

    fn completed_materialisation(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<&PublishedMaterialisation> {
        self.completed
            .as_deref()
            .and_then(|completed| completed.materialisation(identity))
    }

    fn completed_const_function(
        &self,
        origin: &OriginFunctionId,
    ) -> Option<&PublishedConstFunction> {
        self.completed
            .as_deref()
            .and_then(|completed| completed.const_function(origin))
    }

    #[cfg(test)]
    pub(crate) fn local_materialisation_count(&self) -> usize {
        self.published.len() + self.const_functions.len()
    }

    #[cfg(test)]
    pub(crate) fn resolves_materialisation(&self, identity: &GeneratedDeclarationIdentity) -> bool {
        self.published_template(identity).is_some()
    }

    #[cfg(test)]
    pub(crate) fn borrowed_completed_entry_count(&self) -> usize {
        self.completed.as_deref().map_or(0, |completed| {
            completed.published.len() + completed.const_functions.len()
        })
    }

    /// Diagnose a declaring-context collision without mutating the registry.
    ///
    /// Re-publishing the same declaring context and template row is idempotent.
    fn preflight_publish(
        &self,
        identity: &GeneratedDeclarationIdentity,
        context: &Arc<ModuleMaterialisationContext>,
        template_index: usize,
    ) -> Result<(), InfrastructureFailure> {
        if let Some(existing) = self.published_template(identity) {
            if Arc::ptr_eq(&existing.context, context) && existing.template_index == template_index
            {
                return Ok(());
            }

            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Generated declaration identity {:?} was published by multiple declaring contexts in one compilation boundary",
                    identity
                ),
            ));
        }
        Ok(())
    }

    /// Record every declaring row from one context, or leave the registry unchanged.
    pub(crate) fn publish_context(
        &mut self,
        context: &Arc<ModuleMaterialisationContext>,
    ) -> Result<(), InfrastructureFailure> {
        self.publish_context_with_rebase_required(context, false)
    }

    /// Record every declaring row from one context with an explicit identity-domain marker.
    ///
    /// Deferred check-only compilation seeds canonical rows after their boundary has installed
    /// retained tables, but its requester forks were prepared before canonical publication. Those
    /// rows therefore need the same spelling-based rebase as a completed package seed.
    pub(crate) fn publish_context_with_rebase_required(
        &mut self,
        context: &Arc<ModuleMaterialisationContext>,
        rebase_required: bool,
    ) -> Result<(), InfrastructureFailure> {
        let rows: Vec<_> = context.declaration_rows().collect();
        let const_rows: Vec<_> = context
            .const_function_rows()
            .map(|(origin, row_index)| (origin.clone(), row_index))
            .collect();
        let mut pending = FxHashMap::default();
        for (identity, template_index) in &rows {
            self.preflight_publish(identity, context, *template_index)?;
            if let Some(&existing_index) = pending.get(identity) {
                if existing_index != *template_index {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Generated declaration identity {:?} was published by multiple rows in one declaring context",
                            identity
                        ),
                    ));
                }
            } else {
                pending.insert(*identity, *template_index);
            }
        }
        let mut pending_consts = FxHashMap::default();
        for (origin, row_index) in &const_rows {
            self.preflight_publish_const(origin, context, *row_index)?;
            if let Some(&existing_index) = pending_consts.get(origin) {
                if existing_index != *row_index {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "Const function origin {:?} was published by multiple rows in one declaring context",
                            origin
                        ),
                    ));
                }
            } else {
                pending_consts.insert(origin.clone(), *row_index);
            }
        }
        // Every row passed preflight above, so record them without re-running those checks.
        // Preflight resolved completed-package rows too, so a row that already resolves through the
        // borrowed index is idempotent here and stays out of the local maps; copying it would
        // duplicate every completed row inside each boundary.
        for (identity, template_index) in rows {
            if self.completed_materialisation(identity).is_none() {
                self.published
                    .entry(identity.clone())
                    .or_insert(PublishedMaterialisation {
                        context: Arc::clone(context),
                        template_index,
                        rebase_required,
                        package_index: None,
                    });
            }
        }
        for (origin, row_index) in const_rows {
            if self.completed_const_function(&origin).is_none() {
                self.const_functions
                    .entry(origin)
                    .or_insert(PublishedConstFunction {
                        context: Arc::clone(context),
                        row_index,
                        rebase_required,
                        package_index: None,
                    });
            }
        }
        Ok(())
    }

    /// Diagnose a const function row collision without mutating the registry.
    ///
    /// Re-publishing the same declaring context and row is idempotent.
    fn preflight_publish_const(
        &self,
        origin: &OriginFunctionId,
        context: &Arc<ModuleMaterialisationContext>,
        row_index: usize,
    ) -> Result<(), InfrastructureFailure> {
        if let Some(existing) = self.const_function(origin)
            && !(Arc::ptr_eq(&existing.context, context) && existing.row_index == row_index)
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Const function origin {:?} was published by multiple declaring contexts in one compilation boundary",
                    origin
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/provider_materialisations_tests.rs"]
mod tests;

pub(crate) enum DeclaringMaterialisation<'a> {
    Published {
        context: &'a ModuleMaterialisationContext,
        template_index: usize,
        /// Whether the declaring context was published from a different identity domain and
        /// must re-intern its retained paths by spelling into the requester fork.
        rebase_required: bool,
    },
    /// The transaction retains complete declaring modules for requester-owned evidence methods.
    /// Selection may use its own module or an original caller module carried through sidecars.
    Prepared(&'a ModuleMaterialisationPreparation),
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
            rebase_required: published.rebase_required,
        });
    }

    requester_context
        .declaring_context_for_identity(identity)
        .is_some()
        .then_some(DeclaringMaterialisation::Prepared(requester_context))
}
