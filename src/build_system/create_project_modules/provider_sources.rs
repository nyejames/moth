//! Stage 0 implementations of provider-owned source registration.
//!
//! WHAT: two registrar owners share one provider contract. `BoundaryProviderSources` uses the
//!       already-registered source slot in a directory project or source-package boundary.
//!       `ProviderOwnedSourceStaging` registers sources discovered by the single-file traversal,
//!       because that lane builds its final source table only after the walk completes.
//! WHY: provider diagnostics need exact spans in the provider-owned file, and the compiler cannot
//!      render a span whose source text was never loaded into the boundary source database.
//!
//! Both owners retain the text the provider already read, so no provider-owned file is read twice.

use crate::build_system::create_project_modules::source_loading::SelectedSourceTextMap;
use crate::builder_surface::external_import_providers::provider_sources::{
    ProviderSourceHandle, ProviderSourceRegistrar, ProviderSourceRegistrationError,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabaseBuilder, SourceId, SourceKind, SourceSpanBuilders,
};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// Provider-owned sources registered against one indexed boundary source table.
///
/// The directory and source-package lanes register every candidate before discovery, so the
/// provider lookup finds an existing slot and the registrar only supplies text and spans.
pub(crate) struct BoundaryProviderSources<'a, 'sources> {
    span_builders: &'a mut SourceSpanBuilders<'sources>,
    selected_source_texts: &'a mut SelectedSourceTextMap,
    live_builders: FxHashMap<SourceId, ExtendedSpanBuilder>,
}

impl<'a, 'sources> BoundaryProviderSources<'a, 'sources> {
    pub(crate) fn new(
        span_builders: &'a mut SourceSpanBuilders<'sources>,
        selected_source_texts: &'a mut SelectedSourceTextMap,
    ) -> Self {
        Self {
            span_builders,
            selected_source_texts,
            live_builders: FxHashMap::default(),
        }
    }

    fn source_id_for_path(
        &self,
        canonical_path: &Path,
    ) -> Result<SourceId, ProviderSourceRegistrationError> {
        self.span_builders
            .sources()
            .get_by_canonical_path(canonical_path)
            .map(|slot| slot.id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "provider-owned source {} is absent from the boundary source table",
                        canonical_path.display()
                    ),
                )
                .into()
            })
    }

    fn builder_for_source(&mut self, source: SourceId) -> &mut ExtendedSpanBuilder {
        if !self.live_builders.contains_key(&source) {
            let builder = self.span_builders.take_span_builder(source);
            self.live_builders.insert(source, builder);
        }
        self.live_builders
            .get_mut(&source)
            .expect("provider source builder was inserted above")
    }
}

impl ProviderSourceRegistrar for BoundaryProviderSources<'_, '_> {
    fn register_provider_source<'a>(
        &'a mut self,
        canonical_path: &Path,
        _logical_source_path: &str,
        source: String,
    ) -> Result<ProviderSourceHandle<'a>, ProviderSourceRegistrationError> {
        if source.len() >= u32::MAX as usize {
            return Err(ProviderSourceRegistrationError::SourceTooLarge);
        }
        let source_id = self.source_id_for_path(canonical_path)?;
        let source_length = source.len();
        self.selected_source_texts
            .retain_provider_text(canonical_path, source);
        let span_builder = self.builder_for_source(source_id);
        Ok(ProviderSourceHandle::new(
            source_id,
            source_length,
            span_builder,
        ))
    }

    fn finish(&mut self) {
        for (source, builder) in std::mem::take(&mut self.live_builders) {
            self.span_builders.retain_span_builder(source, builder);
        }
    }
}

/// Provider-owned sources discovered by the synthetic single-file traversal.
///
/// The traversal has no indexed provider-owned candidate slots. Registration therefore takes a
/// provisional identity from the reserved provider range; finalization registers canonical rows
/// and rebinds each provisional identity by canonical path.
pub(crate) struct ProviderOwnedSourceStaging {
    entries: Vec<StagedProviderSource>,
    entry_by_path: FxHashMap<PathBuf, usize>,
}

struct StagedProviderSource {
    canonical_path: PathBuf,
    source_id: SourceId,
    text: Option<String>,
    span_builder: ExtendedSpanBuilder,
}

/// Provisional identities occupy the upper half of the compact identity domain.
///
/// The traversal mints its own source identities from the lower range. A reserved high base keeps
/// the two provisional namespaces disjoint until finalization rebinds provider identities by path.
pub(crate) const PROVIDER_SOURCE_ID_BASE: usize = 1 << 31;

impl ProviderOwnedSourceStaging {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            entry_by_path: FxHashMap::default(),
        }
    }

    /// Canonical registration rows for the final source table.
    pub(crate) fn registration_rows(&self) -> impl Iterator<Item = (&Path, SourceKind)> + '_ {
        self.entries
            .iter()
            .map(|entry| (entry.canonical_path.as_path(), SourceKind::ProviderOwned))
    }

    /// Provisional identity and canonical path pairs for final identity rebinding.
    pub(crate) fn provisional_identities(&self) -> impl Iterator<Item = (SourceId, &Path)> + '_ {
        self.entries
            .iter()
            .map(|entry| (entry.source_id, entry.canonical_path.as_path()))
    }

    /// Retain staged text and span builders in the final source owner, then finalize each table.
    ///
    /// Provider span producers have all returned before this boundary, so no staged builder can
    /// receive another span afterwards.
    pub(crate) fn retain_into(
        &mut self,
        source_files: &mut SourceDatabaseBuilder,
    ) -> Result<(), InfrastructureFailure> {
        for entry in &mut self.entries {
            let final_source_id = source_files
                .sources()
                .get_by_canonical_path(&entry.canonical_path)
                .map(|slot| slot.id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "provider-owned source {} is absent from the finalized source table",
                            entry.canonical_path.display()
                        ),
                    )
                })?;
            let text = entry.text.take().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "provider-owned source {} lost its retained text",
                        entry.canonical_path.display()
                    ),
                )
            })?;
            let text_error = source_files
                .sources_mut()
                .retain_text(final_source_id, text)
                .err();
            if let Some(error) = text_error {
                return Err(match error {
                    crate::compiler_frontend::source::SourceDatabaseError::Infrastructure(
                        failure,
                    ) => failure,
                    other => InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "provider-owned source {} could not retain its text: {other:?}",
                            entry.canonical_path.display()
                        ),
                    ),
                });
            }
            let span_builder =
                std::mem::replace(&mut entry.span_builder, ExtendedSpanBuilder::new());
            source_files.retain_span_builder(final_source_id, span_builder);
            source_files.finish_source(final_source_id)?;
        }
        Ok(())
    }
}

impl ProviderSourceRegistrar for ProviderOwnedSourceStaging {
    fn register_provider_source<'a>(
        &'a mut self,
        canonical_path: &Path,
        _logical_source_path: &str,
        source: String,
    ) -> Result<ProviderSourceHandle<'a>, ProviderSourceRegistrationError> {
        if source.len() >= u32::MAX as usize {
            return Err(ProviderSourceRegistrationError::SourceTooLarge);
        }
        let source_length = source.len();
        let entry_index = match self.entry_by_path.get(canonical_path) {
            Some(index) => *index,
            None => {
                let offset = self.entries.len();
                let source_id = PROVIDER_SOURCE_ID_BASE
                    .checked_add(offset)
                    .and_then(SourceId::try_from_index)
                    .ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::ProjectBuildAssembly,
                            "provider-owned source identity range is exhausted",
                        )
                    })?;
                let index = self.entries.len();
                self.entries.push(StagedProviderSource {
                    canonical_path: canonical_path.to_path_buf(),
                    source_id,
                    text: Some(source),
                    span_builder: ExtendedSpanBuilder::new(),
                });
                self.entry_by_path
                    .insert(canonical_path.to_path_buf(), index);
                index
            }
        };
        let entry = &mut self.entries[entry_index];
        Ok(ProviderSourceHandle::new(
            entry.source_id,
            source_length,
            &mut entry.span_builder,
        ))
    }

    fn finish(&mut self) {}
}
