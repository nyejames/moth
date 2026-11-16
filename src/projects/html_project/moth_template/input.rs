//! Input normalization for the direct Moth template API.
//!
//! WHAT: turns file, directory, file-list, and in-memory requests into ordered source units.
//! WHY: compile orchestration should receive deterministic, duplicate-checked inputs without
//! owning filesystem traversal policy.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{imports, token_lexical};
use crate::compiler_frontend::compiler_messages::{PreparedFailure, SourceSpanCapacityResource};
use crate::compiler_frontend::symbols::path_interner::{
    NonUtf8PathComponent, PathId, PathInternError, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
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
    /// The caller's in-memory logical source name.
    ///
    /// This is a portable identity, not a file to open: components must be UTF-8 and the only
    /// separator is `/`. A relative name must not contain a backslash at all. An absolute name
    /// may spell only its prefix and root natively; everything after them must still use `/`.
    /// Filesystem inputs carry canonical native paths instead.
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
                let mut units = Vec::with_capacity(sources.len());
                for source in sources {
                    validate_in_memory_source_name(&source.display_path)?;
                    units.push(MothTemplateSourceUnit {
                        source_path: source.display_path,
                        relative_path: None,
                        source_text: source.source_text,
                    });
                }

                assign_in_memory_relative_paths(&mut units, string_table, path_fork)?;
                units
            }
        };

        validate_source_path_identities(&units, string_table, path_fork)?;
        reject_duplicate_source_paths(&units, string_table, path_fork)?;

        Ok(units)
    }
}

fn collect_directory_units(
    directory: PathBuf,
    recursive: bool,
) -> Result<Vec<MothTemplateSourceUnit>, InfrastructureFailure> {
    let root = canonicalize_path(&directory)?;
    let mut paths = Vec::new();
    collect_moth_template_paths_in_directory(&root, recursive, &mut paths)?;

    paths.sort_by(|left, right| {
        relative_path_under(&root, left).cmp(relative_path_under(&root, right))
    });

    let mut units = Vec::with_capacity(paths.len());
    for path in paths {
        let relative_path = relative_path_under(&root, &path).to_path_buf();
        units.push(read_file_unit(path, Some(relative_path))?);
    }

    Ok(units)
}

/// Give every file-list unit its request-relative identity relative to the files' longest common
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

    let Some(ancestor) =
        common_ancestor_directory(units.iter().map(|unit| unit.source_path.as_path()))
    else {
        return Err(no_common_ancestor_failure(
            &units[0].source_path,
            &units[1].source_path,
            string_table,
            path_fork,
        ));
    };

    for unit in units.iter_mut() {
        unit.relative_path = Some(relative_path_under(&ancestor, &unit.source_path).to_path_buf());
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
        return Err(no_common_ancestor_failure(
            &units[0].source_path,
            &units[mixed_index].source_path,
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
    let Some(ancestor) =
        common_ancestor_directory(units.iter().map(|unit| unit.source_path.as_path()))
    else {
        return Err(no_common_ancestor_failure(
            &units[0].source_path,
            &units[1].source_path,
            string_table,
            path_fork,
        ));
    };

    for unit in units.iter_mut() {
        unit.relative_path = Some(relative_path_under(&ancestor, &unit.source_path).to_path_buf());
    }

    Ok(())
}

/// The longest shared directory of the given paths, or `None` when they share none.
///
/// Each path's file component is excluded: two sibling files share their parent directory,
/// while files whose leading components differ (for example files on separate Windows drives)
/// share no ancestor directory at all.
fn common_ancestor_directory<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Option<PathBuf> {
    let mut component_prefixes = paths.into_iter().map(|path| {
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
    let diagnostic = imports::moth_template_inputs_share_no_common_ancestor(
        first_interned,
        second_interned,
        None,
    );

    PreparedFailure::capture_diagnosed(
        DiagnosticBag::from(diagnostic),
        string_table.take_for_handoff(),
        None,
        Some(Arc::new(path_fork.shared_table())),
    )
}

/// Validate one user-supplied in-memory source name against the portable-name contract.
///
/// In-memory display paths are logical identities, not files to open. Portable names separate
/// components with `/`: a relative name must not contain a backslash at all, and an absolute name
/// may spell only its prefix and root natively. A backslash anywhere else would change the
/// identity once host parsing or downstream URL conversion reads it as a separator. UTF-8
/// validation belongs to `validate_source_path_identities`, which sees every input path.
fn validate_in_memory_source_name(display_path: &Path) -> Result<(), PreparedFailure> {
    // A non-UTF-8 spelling cannot be compared as portable text; the shared identity validation
    // reports it through the typed source-path failure before duplicate handling.
    let Some(spelling) = display_path.to_str() else {
        return Ok(());
    };

    // Prefix and root spellings are structural host syntax only for an absolute name. A
    // root-relative Windows name has no prefix and stays a relative in-memory identity.
    let structural_prefix_len = if display_path.is_absolute() {
        display_path
            .components()
            .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
            .map(|component| component.as_os_str().len())
            .sum::<usize>()
    } else {
        0
    };

    if spelling[structural_prefix_len..].contains('\\') {
        return Err(InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            display_path,
            format!(
                "Moth template source name {display_path:?} contains a backslash; in-memory names \
                 separate components with `/`."
            ),
        )
        .into());
    }

    Ok(())
}

/// Validate every source path through the typed UTF-8 identity lane.
///
/// Interning here means a unique non-UTF-8 name fails with the MTF-owned failure and native
/// `host_path` instead of surfacing later as a source-database logical-path failure.
fn validate_source_path_identities(
    units: &[MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    for unit in units {
        intern_filesystem_path_identity(&unit.source_path, path_fork, string_table)?;
    }

    Ok(())
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
                InfrastructureFailure::filesystem(
                    CompilerStage::SourceLoading,
                    &bad_path,
                    format!(
                        "Moth template source path {bad_path:?} contains a non-UTF-8 component; Moth \
                         identity requires UTF-8 paths."
                    ),
                )
                .into()
            }
            PathInternError::TableFull => PreparedFailure::capture_diagnosed(DiagnosticBag::from(token_lexical::source_span_capacity(
                0,
                u32::MAX,
                SourceSpanCapacityResource::LogicalPath,
                None,
            )), string_table.take_for_handoff(), None, Some(Arc::new(path_fork.shared_table()))),
            PathInternError::BaseMismatch { .. } => InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly,
                "logical path merge base is not a structural prefix of the destination table",
            )
            .into(),
        })
}

fn collect_moth_template_paths_in_directory(
    directory: &Path,
    recursive: bool,
    paths: &mut Vec<PathBuf>,
) -> Result<(), InfrastructureFailure> {
    let entries = read_directory_paths(directory)?;

    for path in entries {
        if path.is_dir() {
            // `_` directories are excluded from the project source tree, so recursive template
            // collection must not reintroduce their files.
            if recursive
                && !crate::build_system::create_project_modules::path_has_hidden_component_below(
                    &path, directory,
                )
            {
                collect_moth_template_paths_in_directory(&path, recursive, paths)?;
            }
        } else if path.is_file() && has_moth_template_extension(&path) {
            paths.push(canonicalize_path(&path)?);
        }
    }

    Ok(())
}

/// Read a directory's entries as native paths, ordered per directory.
///
/// Native per-directory sorting fixes recursive traversal and operational failure order because
/// `read_dir` order is host-dependent. Compiled-output ordering is still owned once by the final
/// relative-path sort in `collect_directory_units`.
fn read_directory_paths(directory: &Path) -> Result<Vec<PathBuf>, InfrastructureFailure> {
    let read_dir = fs::read_dir(directory).map_err(|error| {
        InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            directory,
            format!(
                "Failed to read Moth template directory '{}': {error}",
                portable_path_text(directory)
            ),
        )
    })?;

    let mut paths = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|error| {
            InfrastructureFailure::filesystem(
                CompilerStage::SourceLoading,
                directory,
                format!(
                    "Failed to inspect Moth template directory entry in '{}': {error}",
                    portable_path_text(directory)
                ),
            )
        })?;
        paths.push(entry.path());
    }

    paths.sort();
    Ok(paths)
}

fn read_file_unit(
    path: PathBuf,
    relative_path: Option<PathBuf>,
) -> Result<MothTemplateSourceUnit, InfrastructureFailure> {
    let source_path = canonicalize_path(&path)?;
    let source_text = fs::read_to_string(&source_path).map_err(|error| {
        InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            &source_path,
            format!(
                "Failed to read Moth template source '{}': {error}",
                portable_path_text(&source_path)
            ),
        )
    })?;

    Ok(MothTemplateSourceUnit {
        source_path,
        relative_path,
        source_text,
    })
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, InfrastructureFailure> {
    fs::canonicalize(path).map_err(|error| {
        InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            path,
            format!(
                "Failed to resolve Moth template path '{}': {error}",
                portable_path_text(path)
            ),
        )
    })
}

fn reject_duplicate_source_paths(
    units: &[MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    let mut seen_paths: HashSet<&Path> = HashSet::new();
    let mut diagnostics = DiagnosticBag::new();
    let mut has_duplicates = false;

    for unit in units {
        if !seen_paths.insert(unit.source_path.as_path()) {
            let path = intern_filesystem_path_identity(&unit.source_path, path_fork, string_table)?;
            diagnostics.push(imports::duplicate_moth_template_input_path(
                path, None, None,
            ));
            has_duplicates = true;
        }
    }

    if !has_duplicates {
        Ok(())
    } else {
        Err(PreparedFailure::capture_diagnosed(
            diagnostics,
            string_table.take_for_handoff(),
            None,
            Some(Arc::new(path_fork.shared_table())),
        ))
    }
}

/// The path's native spelling relative to `root` when it is under that root.
///
/// The canonical native components are retained so a literal backslash on Unix stays inside one
/// component. Logical consumers convert these components to a portable `/` spelling at their own
/// identity boundary.
fn relative_path_under<'a>(root: &Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

fn has_moth_template_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(SourceFileKind::from_extension)
        == Some(SourceFileKind::MothTemplate)
}
