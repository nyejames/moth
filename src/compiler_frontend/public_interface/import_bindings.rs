//! Build-resolved source-provider inputs consumed by header interface binding.
//!
//! WHAT: associates one retained import shell in one consumer source file with the immutable
//! public semantic interface selected by Stage 0.
//! WHY: the compiler binds names and semantic facts, while the build system owns graph and
//! namespace resolution. This narrow borrowed input keeps build-local `ModuleId` values out of
//! the compiler and prevents header binding from probing the filesystem or opening provider
//! syntax.

use super::model::PublicSemanticInterface;
use crate::compiler_frontend::headers::parse_file_headers::FileImport;
use crate::compiler_frontend::symbols::interned_path::InternedPath;

pub(crate) struct SourceProviderImport<'a> {
    pub(crate) importer_source: InternedPath,
    pub(crate) imported_path: InternedPath,
    pub(crate) from_grouped: bool,
    pub(crate) interface: &'a PublicSemanticInterface,
}

#[derive(Default)]
pub(crate) struct SourceProviderImportSet<'a> {
    imports: Vec<SourceProviderImport<'a>>,
}

impl<'a> SourceProviderImportSet<'a> {
    pub(crate) fn new(imports: Vec<SourceProviderImport<'a>>) -> Self {
        Self { imports }
    }

    pub(crate) fn resolve(
        &self,
        importer_source: &InternedPath,
        import: &FileImport,
    ) -> Option<&'a PublicSemanticInterface> {
        self.imports
            .iter()
            .find(|binding| {
                binding.importer_source == *importer_source
                    && binding.imported_path == import.provider.path
                    && binding.from_grouped == import.from_grouped
            })
            .map(|binding| binding.interface)
    }
}
