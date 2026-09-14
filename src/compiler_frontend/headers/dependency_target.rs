//! Header-owned dependency target classification and checked decoding.
//!
//! WHAT: classifies one retained dependency path as extensionless source or an explicit-extension
//!       provider, then decodes a validated prefix, remaining suffix and extension for later
//!       stages.
//! WHY: header preparation, Stage 0 and interface binding must share one owner for this fact.
//!      Independent prefix slices and extension lookups can disagree on malformed retained state.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
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

/// One provider target checked while a dependency shell is prepared.
///
/// WHAT: owns the provider prefix, suffix components and extension identity produced by the
///      retained-path validation pass.
/// WHY: Stage 0 and interface binding can consume these facts directly instead of decoding the
///      compact prefix count and reconstructing the provider prefix from the path again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CheckedExternalProviderTarget {
    prefix: PathId,
    remaining_components: Vec<StringId>,
    extension: StringId,
    raw_prefix: String,
}

impl CheckedExternalProviderTarget {
    pub(crate) fn prefix_path_id(&self) -> PathId {
        self.prefix
    }

    pub(crate) fn remaining_components(&self) -> &[StringId] {
        &self.remaining_components
    }
    pub(crate) fn extension_id(&self) -> StringId {
        self.extension
    }

    pub(crate) fn raw_prefix(&self) -> &str {
        &self.raw_prefix
    }

    pub(crate) fn extension_spelling<'a>(
        &self,
        string_table: &'a StringTable,
    ) -> Result<&'a str, CompilerError> {
        string_table.try_resolve(self.extension).ok_or_else(|| {
            CompilerError::compiler_error(
                "checked provider target has an invalid extension string id",
            )
        })
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.extension = remap.get(self.extension);
        for component in &mut self.remaining_components {
            *component = remap.get(*component);
        }
    }

    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.prefix = remap.get(self.prefix);
    }

    /// Validate that the owned fact still describes the retained path after ID remapping.
    pub(crate) fn validate(
        &self,
        path: PathId,
        target: &DependencyTargetKind,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        let DependencyTargetKind::ExternalProvider {
            prefix_component_count,
            extension,
        } = target
        else {
            return Err(CompilerError::compiler_error(
                "checked provider target is attached to a source dependency",
            ));
        };
        if self.extension_id() != *extension {
            return Err(CompilerError::compiler_error(
                "checked provider target extension disagrees with its classification",
            ));
        }

        let path_depth = path_fork.try_depth(path).ok_or_else(|| {
            CompilerError::compiler_error(
                "checked provider target path is outside its owning path fork",
            )
        })?;
        let prefix_depth = path_fork.try_depth(self.prefix).ok_or_else(|| {
            CompilerError::compiler_error(
                "checked provider target prefix is outside its owning path fork",
            )
        })?;
        let prefix_len = usize::try_from(*prefix_component_count).map_err(|_| {
            CompilerError::compiler_error("retained provider prefix count does not fit usize")
        })?;
        if prefix_len == 0 {
            return Err(CompilerError::compiler_error(
                "retained provider target has a zero prefix component count",
            ));
        }

        let mut path_components = Vec::new();
        path_fork.resolve_components(path, &mut path_components);
        let mut prefix_components = Vec::new();
        path_fork.resolve_components(self.prefix, &mut prefix_components);
        if path_components
            .iter()
            .chain(prefix_components.iter())
            .any(|component| string_table.try_resolve(*component).is_none())
        {
            return Err(CompilerError::compiler_error(
                "checked provider target contains an invalid path component string id",
            ));
        }
        if prefix_components.len() != prefix_len
            || path_components.len() != prefix_len + self.remaining_components.len()
            || path_components.get(..prefix_len) != Some(prefix_components.as_slice())
            || path_components.get(prefix_len..) != Some(self.remaining_components.as_slice())
        {
            return Err(CompilerError::compiler_error(
                "checked provider target does not match its retained path",
            ));
        }
        if path_depth as usize != path_components.len()
            || prefix_depth as usize != prefix_components.len()
        {
            return Err(CompilerError::compiler_error(
                "checked provider target path depth disagrees with its retained components",
            ));
        }
        let mut raw_prefix_scratch = Vec::new();
        let expected_raw_prefix =
            path_fork.render_portable(self.prefix, string_table, &mut raw_prefix_scratch);
        if expected_raw_prefix != self.raw_prefix {
            return Err(CompilerError::compiler_error(
                "checked provider target raw prefix disagrees with its path identity",
            ));
        }
        let extension_spelling = self.extension_spelling(string_table)?;
        let prefix_component = string_table.resolve(
            *prefix_components
                .last()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "checked provider target prefix has no final component",
                    )
                })?,
        );
        if explicit_non_source_extension(prefix_component) != Some(extension_spelling) {
            return Err(CompilerError::compiler_error(
                "checked provider target prefix does not end with its classified extension",
            ));
        }

        Ok(())
    }
}

/// The decoded provider target owned by a retained dependency path.
///
/// This conversion happens while the path fork and local string table are still available. The
/// resulting record is remapped with the rest of the retained shell and remains valid after the
/// worker-local stores are merged.
pub(crate) fn checked_provider_target(
    path: PathId,
    target: &DependencyTargetKind,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<Option<CheckedExternalProviderTarget>, CompilerError> {
    let DependencyTargetKind::ExternalProvider {
        prefix_component_count,
        extension,
    } = target
    else {
        return Ok(None);
    };

    let (prefix, remaining_components, _) = validate_external_provider_parts(
        path,
        *prefix_component_count,
        *extension,
        path_fork,
        string_table,
    )?;
    let raw_prefix = path_fork.render_portable(prefix, string_table, &mut Vec::new());
    Ok(Some(CheckedExternalProviderTarget {
        prefix,
        remaining_components,
        extension: *extension,
        raw_prefix,
    }))
}

/// One decoded explicit-extension provider target.
///
/// WHAT: exposes the validated prefix as a `PathId` in the same fork plus the
///       remaining provider-specific components and the interned extension whose
///       spelling matches the prefix's last component.
/// WHY: the legacy decoder remains available to focused malformed-state tests and compatibility
///      construction, while prepared shells carry [`CheckedExternalProviderTarget`] directly.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DecodedExternalProviderTarget<'a> {
    prefix: PathId,
    remaining_components: Vec<StringId>,
    extension_spelling: &'a str,
}

#[cfg(test)]
impl<'a> DecodedExternalProviderTarget<'a> {
    pub(crate) fn prefix_path_id(&self) -> PathId {
        self.prefix
    }

    pub(crate) fn remaining_components(&self) -> &[StringId] {
        &self.remaining_components
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
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> DependencyTargetKind {
    let mut components = Vec::new();
    path_fork.resolve_components(path, &mut components);
    let mut provider_extension = None;
    for (index, component) in components.iter().enumerate() {
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
#[cfg(test)]
pub(crate) fn decode_dependency_target<'a>(
    path: PathId,
    target: &DependencyTargetKind,
    path_fork: &PathInternerFork,
    string_table: &'a StringTable,
) -> Result<Option<DecodedExternalProviderTarget<'a>>, CompilerError> {
    match target {
        DependencyTargetKind::Source => Ok(None),
        DependencyTargetKind::ExternalProvider {
            prefix_component_count,
            extension,
        } => {
            validate_external_provider_parts(
                path,
                *prefix_component_count,
                *extension,
                path_fork,
                string_table,
            )
            .map(|(prefix, remaining_components, extension_spelling)| {
                Some(DecodedExternalProviderTarget {
                    prefix,
                    remaining_components,
                    extension_spelling,
                })
            })
        }
    }
}

fn validate_external_provider_parts<'a>(
    path: PathId,
    prefix_component_count: u32,
    extension: StringId,
    path_fork: &PathInternerFork,
    string_table: &'a StringTable,
) -> Result<(PathId, Vec<StringId>, &'a str), CompilerError> {
    if prefix_component_count == 0 {
        return Err(CompilerError::compiler_error(
            "retained provider target has a zero prefix component count",
        ));
    }

    let prefix_len = usize::try_from(prefix_component_count).map_err(|_| {
        CompilerError::compiler_error("retained provider prefix count does not fit usize")
    })?;
    let depth = path_fork.try_depth(path).ok_or_else(|| {
        CompilerError::compiler_error("retained provider path is outside its owning path fork")
    })? as usize;
    if prefix_len > depth {
        return Err(CompilerError::compiler_error(
            "retained provider prefix count is outside the path",
        ));
    }

    let mut components = Vec::new();
    path_fork.resolve_components(path, &mut components);
    if components
        .iter()
        .any(|component| string_table.try_resolve(*component).is_none())
    {
        return Err(CompilerError::compiler_error(
            "retained provider path contains an invalid component string id",
        ));
    }
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

    let mut prefix = path;
    for _ in 0..(depth - prefix_len) {
        prefix = path_fork.try_parent(prefix).ok_or_else(|| {
            CompilerError::compiler_error("retained provider path is missing a parent link")
        })?;
    }

    Ok((
        prefix,
        components[prefix_len..].to_vec(),
        extension_spelling,
    ))
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
