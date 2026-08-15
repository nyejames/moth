//! Header-owned dependency target classification and checked decoding.
//!
//! WHAT: classifies one retained dependency path as extensionless source or an explicit-extension
//!       provider, then decodes a validated prefix, remaining suffix and extension for later
//!       stages.
//! WHY: header preparation, Stage 0 and interface binding must share one owner for this fact.
//!      Independent prefix slices and extension lookups can disagree on malformed retained state.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use std::path::Path;

/// Structural source-versus-provider classification owned by retained header syntax.
///
/// WHAT: records whether a dependency path is extensionless source or an explicit-extension
///       provider, including the provider prefix length and interned extension.
/// WHY: Stage 0 and interface binding must consume this fact instead of rescanning components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DependencyTargetKind {
    Source,
    ExternalProvider {
        prefix_component_count: u32,
        extension: StringId,
    },
}

impl DependencyTargetKind {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if let Self::ExternalProvider { extension, .. } = self {
            *extension = remap.get(*extension);
        }
    }
}

/// One checked explicit-extension provider target.
///
/// WHAT: exposes the validated prefix, remaining provider-specific components and the
///       interned extension whose spelling matches the prefix's last component.
/// WHY: Stage 0 and binding must not reslice the raw count or reinterpret the extension ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DecodedExternalProviderTarget<'a> {
    prefix_components: &'a [StringId],
    remaining_components: &'a [StringId],
    extension_spelling: &'a str,
}

impl<'a> DecodedExternalProviderTarget<'a> {
    pub(crate) fn prefix_path(&self) -> InternedPath {
        InternedPath::from_components(self.prefix_components.to_vec())
    }

    pub(crate) fn remaining_components(&self) -> &'a [StringId] {
        self.remaining_components
    }

    pub(crate) fn extension_spelling(&self) -> &'a str {
        self.extension_spelling
    }
}

/// Classify one retained path as source or explicit-extension provider.
///
/// Compiler-recognized source extensions stay source so later resolution can emit the
/// extensionless-source diagnostic. The first other explicit extension is the provider prefix.
/// Header syntax does not consult the provider registry.
pub(crate) fn classify_dependency_target(
    path: &InternedPath,
    string_table: &mut StringTable,
) -> DependencyTargetKind {
    let mut provider_extension = None;
    for (index, component) in path.as_components().iter().enumerate() {
        let spelling = string_table.resolve(*component);
        let Some(extension) = explicit_non_source_extension(spelling) else {
            continue;
        };

        provider_extension = Some((index + 1, extension.to_owned()));
        break;
    }

    let Some((prefix_component_count, extension)) = provider_extension else {
        return DependencyTargetKind::Source;
    };
    DependencyTargetKind::ExternalProvider {
        prefix_component_count: prefix_component_count as u32,
        extension: string_table.intern(&extension),
    }
}

/// Decode a retained target into a checked provider prefix, or `None` for source.
///
/// WHAT: validates prefix bounds, looks up the retained extension and compares it with the
///       last prefix component. Source targets return `Ok(None)`.
/// WHY: malformed retained classification is compiler corruption and must fail through
///      `CompilerError` rather than looking like an ordinary non-provider dependency.
pub(crate) fn decode_dependency_target<'a>(
    path: &'a InternedPath,
    target: &DependencyTargetKind,
    string_table: &'a StringTable,
) -> Result<Option<DecodedExternalProviderTarget<'a>>, CompilerError> {
    match target {
        DependencyTargetKind::Source => Ok(None),
        DependencyTargetKind::ExternalProvider {
            prefix_component_count,
            extension,
        } => {
            decode_external_provider_target(path, *prefix_component_count, *extension, string_table)
                .map(Some)
        }
    }
}

fn decode_external_provider_target<'a>(
    path: &'a InternedPath,
    prefix_component_count: u32,
    extension: StringId,
    string_table: &'a StringTable,
) -> Result<DecodedExternalProviderTarget<'a>, CompilerError> {
    if prefix_component_count == 0 {
        return Err(CompilerError::compiler_error(
            "retained provider target has a zero prefix component count",
        ));
    }

    let prefix_len = usize::try_from(prefix_component_count).map_err(|_| {
        CompilerError::compiler_error("retained provider prefix count does not fit usize")
    })?;
    let components = path.as_components();
    if prefix_len > components.len() {
        return Err(CompilerError::compiler_error(
            "retained provider prefix count is outside the path",
        ));
    }

    let Some(extension_spelling) = string_table.try_resolve(extension) else {
        return Err(CompilerError::compiler_error(
            "retained provider target has an invalid extension string id",
        ));
    };

    let prefix_component = string_table.resolve(components[prefix_len - 1]);
    let Some(component_extension) = explicit_non_source_extension(prefix_component) else {
        return Err(CompilerError::compiler_error(
            "retained provider prefix does not end with an explicit non-source extension",
        ));
    };
    if component_extension != extension_spelling {
        return Err(CompilerError::compiler_error(
            "retained provider extension does not match the prefix component",
        ));
    }

    Ok(DecodedExternalProviderTarget {
        prefix_components: &components[..prefix_len],
        remaining_components: &components[prefix_len..],
        extension_spelling,
    })
}

fn explicit_non_source_extension(component: &str) -> Option<&str> {
    let extension = Path::new(component)
        .extension()
        .and_then(|extension| extension.to_str())?;
    if SourceFileKind::from_extension(extension).is_some() {
        return None;
    }
    Some(extension)
}

#[cfg(test)]
#[path = "tests/dependency_target_tests.rs"]
mod dependency_target_tests;
