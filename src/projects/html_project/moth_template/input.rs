//! Input normalization for the direct Moth template API.
//!
//! WHAT: turns file, directory, file-list, and in-memory requests into ordered source units.
//! WHY: compile orchestration should receive deterministic, duplicate-checked inputs without
//! owning filesystem traversal policy.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, PreparedFailure, SourceSpanCapacityResource,
};
use crate::compiler_frontend::symbols::path_interner::{
    NonUtf8PathComponent, PathId, PathInternError, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

impl MothTemplateInput {
    pub(super) fn collect_sources(
        self,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<Vec<MothTemplateSourceUnit>, PreparedFailure> {
        let units = match self {
            MothTemplateInput::File(path) => vec![read_file_unit(path, None)?],

            MothTemplateInput::Directory { path, recursive } => {
                collect_directory_units(path, recursive)?
            }

            MothTemplateInput::Files(paths) => {
                let mut units = Vec::with_capacity(paths.len());
                for path in paths {
                    units.push(read_file_unit(path, None)?);
                }
                assign_common_ancestor_relative_paths(&mut units, string_table, path_fork)?;
                units
            }
            MothTemplateInput::Sources(sources) => {
                let mut units = sources
                    .into_iter()
                    .map(|source| MothTemplateSourceUnit {
                        source_path: normalize_path_for_identity(&source.display_path),
                        relative_path: None,
                        source_text: source.source_text,
                    })
                    .collect::<Vec<_>>();

                assign_in_memory_relative_paths(&mut units, string_table, path_fork)?;
                units
            }
        };

        reject_duplicate_source_paths(&units, string_table, path_fork)?;

        Ok(units)
    }
}

fn collect_directory_units(
    directory: PathBuf,
    recursive: bool,
) -> Result<Vec<MothTemplateSourceUnit>, CompilerError> {
    let root = canonicalize_path(&directory)?;
    let mut paths = Vec::new();
    collect_moth_template_paths_in_directory(&root, recursive, &mut paths)?;

    paths.sort_by(|left, right| {
        normalized_relative_path(&root, left).cmp(&normalized_relative_path(&root, right))
    });

    let mut units = Vec::with_capacity(paths.len());
    for path in paths {
        let relative_path = normalized_relative_path(&root, &path);
        units.push(read_file_unit(path, Some(relative_path))?);
    }

    Ok(units)
}

/// Give every file-list unit its portable identity relative to the files' longest common
/// ancestor directory.
///
/// Distinct directories under that ancestor yield distinct module origins, so same-named
/// resources in different documents cannot collide. One file's relative path is its file name,
/// which keeps the entry-root empty module path correct for a single-file request.
fn assign_common_ancestor_relative_paths(
    units: &mut [MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    if units.is_empty() {
        return Ok(());
    }

    let canonical_paths = units
        .iter()
        .map(|unit| unit.source_path.clone())
        .collect::<Vec<_>>();
    let Some(ancestor) = common_ancestor_directory(&canonical_paths) else {
        return Err(no_common_ancestor_failure(
            &canonical_paths[0],
            &canonical_paths[1],
            string_table,
            path_fork,
        ));
    };

    for unit in units.iter_mut() {
        unit.relative_path = Some(normalized_relative_path(&ancestor, &unit.source_path));
    }

    Ok(())
}

/// Give every in-memory unit the request's portable display identity.
///
/// Relative display paths are portable identities directly. Absolute display paths fall back to
/// the common-ancestor rule on the display paths. A request mixing relative and absolute display
/// paths has no shared portable basis, so it is diagnosed instead of minting colliding empty
/// module origins.
fn assign_in_memory_relative_paths(
    units: &mut [MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    if units.is_empty() {
        return Ok(());
    }

    let basis_is_relative = units[0].source_path.is_relative();
    if let Some(mixed_index) = units
        .iter()
        .position(|unit| unit.source_path.is_relative() != basis_is_relative)
    {
        let first_path = units[0].source_path.clone();
        let mixed_path = units[mixed_index].source_path.clone();
        return Err(no_common_ancestor_failure(
            &first_path,
            &mixed_path,
            string_table,
            path_fork,
        ));
    }

    if basis_is_relative {
        for unit in units.iter_mut() {
            unit.relative_path = Some(unit.source_path.clone());
        }

        return Ok(());
    }

    // An absolute display path is no portable identity by itself; the display paths' common
    // ancestor directory plays the same role a request root plays for file inputs.
    let display_paths = units
        .iter()
        .map(|unit| unit.source_path.clone())
        .collect::<Vec<_>>();
    let Some(ancestor) = common_ancestor_directory(&display_paths) else {
        return Err(no_common_ancestor_failure(
            &display_paths[0],
            &display_paths[1],
            string_table,
            path_fork,
        ));
    };

    for unit in units.iter_mut() {
        unit.relative_path = Some(normalized_relative_path(&ancestor, &unit.source_path));
    }

    Ok(())
}

/// The longest shared directory of the given paths, or `None` when they share none.
///
/// Each path's file component is excluded: two sibling files share their parent directory,
/// while files whose leading components differ (for example files on separate Windows drives)
/// share no ancestor directory at all.
fn common_ancestor_directory(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut component_prefixes = paths.iter().map(|path| {
        let mut components: Vec<&std::ffi::OsStr> = path
            .components()
            .map(|component| component.as_os_str())
            .collect();
        components.pop();
        components
    });

    let mut shared = component_prefixes.next()?;
    for components in component_prefixes {
        let shared_length = shared
            .iter()
            .zip(components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        shared.truncate(shared_length);

        if shared.is_empty() {
            return None;
        }
    }

    Some(shared.into_iter().collect())
}

/// Reject a request whose inputs cannot derive portable module identities.
fn no_common_ancestor_failure(
    first_path: &Path,
    second_path: &Path,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> PreparedFailure {
    let first_interned = intern_filesystem_path_identity(first_path, path_fork, string_table);
    let second_interned = intern_filesystem_path_identity(second_path, path_fork, string_table);
    let (first_interned, second_interned) = match (first_interned, second_interned) {
        (Ok(first), Ok(second)) => (first, second),
        (Err(failure), _) | (_, Err(failure)) => {
            return failure;
        }
    };
    let diagnostic = CompilerDiagnostic::moth_template_inputs_share_no_common_ancestor(
        first_interned,
        second_interned,
        None,
    );

    PreparedFailure::capture_diagnosed(
        DiagnosticBag::from(diagnostic),
        string_table.clone(),
        None,
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    )
}
/// Intern one filesystem path for user-facing identity, failing non-UTF-8 paths like the
/// duplicate-input check does. Compact-table exhaustion is authored input rejection, so it maps
/// to the typed source capacity diagnostic.
fn intern_filesystem_path_identity(
    path: &Path,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<PathId, PreparedFailure> {
    path_fork
        .try_intern_filesystem_path(path, string_table)
        .map_err(|error| match error {
            PathInternError::NonUtf8(NonUtf8PathComponent { path: bad_path }) => {
                CompilerError::file_error(
                    &bad_path,
                    format!(
                        "Moth template source path {bad_path:?} contains a non-UTF-8 component; Moth \
                         identity requires UTF-8 paths."
                    ),
                )
                .into()
            }
            PathInternError::TableFull => PreparedFailure::capture_diagnosed(
                DiagnosticBag::from(CompilerDiagnostic::source_table_capacity(
                    SourceSpanCapacityResource::LogicalPath,
                )),
                string_table.clone(),
                None,
                true,
                Some(Arc::new(path_fork.snapshot_table())),
            ),
            PathInternError::BaseMismatch { .. } => CompilerError::compiler_error(
                "logical path merge base is not a structural prefix of the destination table",
            )
            .into(),
        })
}

fn collect_moth_template_paths_in_directory(
    directory: &Path,
    recursive: bool,
    paths: &mut Vec<PathBuf>,
) -> Result<(), CompilerError> {
    let entries = read_directory_sorted(directory)?;

    for entry in entries {
        let path = entry.path();
        if path.is_dir() && recursive {
            collect_moth_template_paths_in_directory(&path, recursive, paths)?;
        } else if path.is_file() && has_moth_template_extension(&path) {
            paths.push(canonicalize_path(&path)?);
        }
    }

    Ok(())
}

fn read_directory_sorted(directory: &Path) -> Result<Vec<fs::DirEntry>, CompilerError> {
    let read_dir = fs::read_dir(directory).map_err(|error| {
        CompilerError::file_error(
            directory,
            format!(
                "Failed to read Moth template directory '{}': {error}",
                directory.display()
            ),
        )
    })?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|error| {
            CompilerError::file_error(
                directory,
                format!(
                    "Failed to inspect Moth template directory entry in '{}': {error}",
                    directory.display()
                ),
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
) -> Result<MothTemplateSourceUnit, CompilerError> {
    let source_path = canonicalize_path(&path)?;
    let source_text = fs::read_to_string(&source_path).map_err(|error| {
        CompilerError::file_error(
            &source_path,
            format!(
                "Failed to read Moth template source '{}': {error}",
                source_path.display()
            ),
        )
    })?;

    Ok(MothTemplateSourceUnit {
        source_path,
        relative_path,
        source_text,
    })
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, CompilerError> {
    fs::canonicalize(path).map_err(|error| {
        CompilerError::file_error(
            path,
            format!(
                "Failed to resolve Moth template path '{}': {error}",
                path.display()
            ),
        )
    })
}

fn reject_duplicate_source_paths(
    units: &[MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    let mut seen_paths = HashSet::new();
    let mut diagnostics = Vec::new();

    for unit in units {
        let normalized = normalize_path_for_identity(&unit.source_path);

        if !seen_paths.insert(normalized.clone()) {
            let path = intern_filesystem_path_identity(&normalized, path_fork, string_table)?;
            diagnostics.push(CompilerDiagnostic::duplicate_moth_template_input_path(
                path, None, None,
            ));
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(PreparedFailure::capture_diagnosed(
            DiagnosticBag::from_diagnostics(diagnostics),
            string_table.clone(),
            None,
            true,
            Some(Arc::new(path_fork.snapshot_table())),
        ))
    }
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
