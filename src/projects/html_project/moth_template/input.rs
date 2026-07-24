//! Input normalization for the direct Moth template API.
//!
//! WHAT: turns file, directory, file-list, and in-memory requests into ordered source units.
//! WHY: compile orchestration should receive deterministic, duplicate-checked inputs without
//! owning filesystem traversal policy.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::moth_template::scope::{
    MothTemplatePathScope, MothTemplateScopeConstant,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MothTemplateCompileRequest {
    pub(crate) input: MothTemplateInput,
    pub(crate) default_module_constants: Vec<MothTemplateScopeConstant>,
    pub(crate) module_constants_by_path: Vec<MothTemplatePathScope>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MothTemplateInput {
    File(PathBuf),
    Directory { path: PathBuf, recursive: bool },
    Files(Vec<PathBuf>),
    Sources(Vec<MothTemplateSource>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MothTemplateSource {
    pub(crate) display_path: PathBuf,
    pub(crate) source_text: String,
}

pub(super) struct MothTemplateSourceUnit {
    pub(super) source_path: PathBuf,
    pub(super) relative_path: Option<PathBuf>,
    pub(super) source_text: String,
}

impl MothTemplateCompileRequest {
    pub(super) fn collect_sources(
        self,
        string_table: &mut StringTable,
    ) -> Result<Vec<MothTemplateSourceUnit>, CompilerMessages> {
        self.validate_no_caller_scope_constants(string_table)?;

        let units = match self.input {
            MothTemplateInput::File(path) => vec![read_file_unit(path, None, string_table)?],

            MothTemplateInput::Directory { path, recursive } => {
                collect_directory_units(path, recursive, string_table)?
            }

            MothTemplateInput::Files(paths) => {
                let mut units = Vec::with_capacity(paths.len());
                for path in paths {
                    units.push(read_file_unit(path, None, string_table)?);
                }
                units
            }

            MothTemplateInput::Sources(sources) => sources
                .into_iter()
                .map(|source| MothTemplateSourceUnit {
                    source_path: normalize_path_for_identity(&source.display_path),
                    relative_path: None,
                    source_text: source.source_text,
                })
                .collect(),
        };

        reject_duplicate_source_paths(&units, string_table)?;

        Ok(units)
    }

    fn validate_no_caller_scope_constants(
        &self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        if let Some(scope) = self
            .module_constants_by_path
            .iter()
            .find(|scope| !scope.constants.is_empty())
        {
            let messages = unsupported_scope_constant_messages(&scope.source_path, string_table);
            return Err(messages);
        }

        if !self.default_module_constants.is_empty() {
            let location_path = match &self.input {
                MothTemplateInput::File(path) => path,
                MothTemplateInput::Directory { path, .. } => path,
                MothTemplateInput::Files(paths) => paths
                    .first()
                    .map(PathBuf::as_path)
                    .unwrap_or_else(|| Path::new("<moth-template>")),
                MothTemplateInput::Sources(sources) => sources
                    .first()
                    .map(|source| source.display_path.as_path())
                    .unwrap_or_else(|| Path::new("<moth-template>")),
            };
            let messages = unsupported_scope_constant_messages(location_path, string_table);
            return Err(messages);
        }

        Ok(())
    }
}

fn collect_directory_units(
    directory: PathBuf,
    recursive: bool,
    string_table: &mut StringTable,
) -> Result<Vec<MothTemplateSourceUnit>, CompilerMessages> {
    let root = canonicalize_path(&directory, string_table)?;
    let mut paths = Vec::new();
    collect_moth_template_paths_in_directory(&root, recursive, &mut paths, string_table)?;

    paths.sort_by(|left, right| {
        normalized_relative_path(&root, left).cmp(&normalized_relative_path(&root, right))
    });

    let mut units = Vec::with_capacity(paths.len());
    for path in paths {
        let relative_path = normalized_relative_path(&root, &path);
        units.push(read_file_unit(path, Some(relative_path), string_table)?);
    }

    Ok(units)
}

fn collect_moth_template_paths_in_directory(
    directory: &Path,
    recursive: bool,
    paths: &mut Vec<PathBuf>,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    let entries = read_directory_sorted(directory, string_table)?;

    for entry in entries {
        let path = entry.path();
        if path.is_dir() && recursive {
            collect_moth_template_paths_in_directory(&path, recursive, paths, string_table)?;
        } else if path.is_file() && has_moth_template_extension(&path) {
            paths.push(canonicalize_path(&path, string_table)?);
        }
    }

    Ok(())
}

fn read_directory_sorted(
    directory: &Path,
    string_table: &mut StringTable,
) -> Result<Vec<fs::DirEntry>, CompilerMessages> {
    let read_dir = fs::read_dir(directory).map_err(|error| {
        CompilerMessages::file_error(
            directory,
            format!(
                "Failed to read Moth template directory '{}': {error}",
                directory.display()
            ),
            &string_table.clone(),
        )
    })?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|error| {
            CompilerMessages::file_error(
                directory,
                format!(
                    "Failed to inspect Moth template directory entry in '{}': {error}",
                    directory.display()
                ),
                &string_table.clone(),
            )
        })?;
        entries.push(entry);
    }

    entries.sort_by_key(|entry| normalize_path_for_identity(&entry.path()));
    Ok(entries)
}

fn read_file_unit(
    path: PathBuf,
    relative_path: Option<PathBuf>,
    string_table: &mut StringTable,
) -> Result<MothTemplateSourceUnit, CompilerMessages> {
    let source_path = canonicalize_path(&path, string_table)?;
    let source_text = fs::read_to_string(&source_path).map_err(|error| {
        CompilerMessages::file_error(
            &source_path,
            format!(
                "Failed to read Moth template source '{}': {error}",
                source_path.display()
            ),
            &string_table.clone(),
        )
    })?;

    Ok(MothTemplateSourceUnit {
        source_path,
        relative_path,
        source_text,
    })
}

fn canonicalize_path(
    path: &Path,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerMessages> {
    fs::canonicalize(path).map_err(|error| {
        CompilerMessages::file_error(
            path,
            format!(
                "Failed to resolve Moth template path '{}': {error}",
                path.display()
            ),
            &string_table.clone(),
        )
    })
}

fn reject_duplicate_source_paths(
    units: &[MothTemplateSourceUnit],
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    let mut first_locations: HashMap<PathBuf, SourceLocation> = HashMap::new();
    let mut diagnostics = Vec::new();

    for unit in units {
        let normalized = normalize_path_for_identity(&unit.source_path);
        let location = SourceLocation::from_path(&unit.source_path, string_table);

        if let Some(first_location) = first_locations.get(&normalized) {
            let path = match InternedPath::try_from_filesystem_path(&normalized, string_table) {
                Ok(interned) => interned,
                Err(NonUtf8PathComponent { path: bad_path }) => {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::file_error(
                            &bad_path,
                            format!(
                                "Moth template source path {bad_path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                            ),
                            string_table,
                        ),
                        string_table,
                    ));
                }
            };
            diagnostics.push(CompilerDiagnostic::duplicate_moth_template_input_path(
                path,
                first_location.clone(),
                location,
            ));
        } else {
            first_locations.insert(normalized, location);
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(CompilerMessages::from_diagnostics(
            diagnostics,
            string_table.clone(),
        ))
    }
}

fn unsupported_scope_constant_messages(
    location_path: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let path = match InternedPath::try_from_filesystem_path(location_path, string_table) {
        Ok(interned) => interned,
        Err(NonUtf8PathComponent { path: bad_path }) => {
            return CompilerMessages::from_error_ref(
                CompilerError::file_error(
                    &bad_path,
                    format!(
                        "Moth template scope path {bad_path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                    ),
                    string_table,
                ),
                string_table,
            );
        }
    };
    let location = SourceLocation::new(
        path.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let diagnostic = CompilerDiagnostic::invalid_moth_template_api_scope_item(path, location);

    CompilerMessages::from_diagnostics(vec![diagnostic], string_table.clone())
}

fn normalized_relative_path(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(normalize_path_for_identity)
        .unwrap_or_else(|_| normalize_path_for_identity(path))
}

fn normalize_path_for_identity(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        let component_text = component.as_os_str().to_string_lossy().replace('\\', "/");
        normalized.push(component_text);
    }

    normalized
}

fn has_moth_template_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(SourceFileKind::from_extension)
        == Some(SourceFileKind::MothTemplate)
}
