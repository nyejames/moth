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
    /// separator is `/`. A relative name must not contain a backslash at all. A rooted name,
    /// whether absolute or root-relative such as `/docs/intro.mtf`, may spell only its prefix
    /// and root natively; everything after them must still use `/` and must not begin with a
    /// drive prefix once the root is stripped. Rootedness is `Path::has_root`, so a root without
    /// a drive prefix is a rooted identity on every host and follows the same common-ancestor
    /// rule as an absolute name. A name with no portable basis, such as a drive prefix without a
    /// root or a bare root or UNC share with no file component, is rejected as input. Filesystem
    /// inputs carry canonical native paths instead.
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

/// Give every unit its request-relative identity under the request's longest common ancestor
/// directory.
///
/// File-list units and rooted in-memory display paths share this rule. Distinct directories under
/// that ancestor yield distinct module origins, so same-named resources in different documents
/// cannot collide. One input's relative path is its file name, which keeps the entry-root empty
/// module path correct for a single-input request. An input that eliminates the shared ancestor
/// is diagnosed against the basis, so the failure names the input that removed the shared basis.
fn assign_common_ancestor_relative_paths(
    units: &mut [MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    if units.is_empty() {
        return Ok(());
    }

    let ancestor =
        match common_ancestor_directory(units.iter().map(|unit| unit.source_path.as_path())) {
            Ok(ancestor) => ancestor,
            Err(conflicting_path) => {
                return Err(no_common_ancestor_failure(
                    &units[0].source_path,
                    conflicting_path,
                    string_table,
                    path_fork,
                ));
            }
        };

    for unit in units.iter_mut() {
        unit.relative_path = Some(relative_path_under(&ancestor, &unit.source_path).to_path_buf());
    }

    Ok(())
}

/// Give every in-memory unit the request's portable display identity.
///
/// Relative display paths are portable identities directly. Rooted display paths fall back to
/// the common-ancestor rule on the display paths. Rootedness is `Path::has_root`, not
/// `Path::is_absolute`: Windows parses `/docs/x.mtf` as rooted but not absolute, and it must
/// classify exactly as Unix does. A request mixing rooted and relative display paths has no
/// shared portable basis, so it is diagnosed instead of minting colliding empty module origins.
fn assign_in_memory_relative_paths(
    units: &mut [MothTemplateSourceUnit],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), PreparedFailure> {
    if units.is_empty() {
        return Ok(());
    }

    let basis_is_rooted = units[0].source_path.has_root();
    if let Some(mixed_index) = units
        .iter()
        .position(|unit| unit.source_path.has_root() != basis_is_rooted)
    {
        return Err(no_common_ancestor_failure(
            &units[0].source_path,
            &units[mixed_index].source_path,
            string_table,
            path_fork,
        ));
    }

    if !basis_is_rooted {
        for unit in units.iter_mut() {
            unit.relative_path = Some(unit.source_path.clone());
        }

        return Ok(());
    }

    // A rooted display path is no portable identity by itself; the display paths' common
    // ancestor directory plays the same role a request root plays for file inputs.
    assign_common_ancestor_relative_paths(units, string_table, path_fork)?;
    reject_derived_drive_relative_paths(units)
}

/// Reject in-memory identities whose derived relative path is drive-relative on this host.
///
/// Stripping a root can expose a drive-looking first component: `\C:guide.mtf` yields the
/// relative spelling `C:guide.mtf`, which Windows parses as a drive-relative path. Module origin
/// derivation would then reject its `C:` parent internally, so the original input name is
/// rejected here instead.
fn reject_derived_drive_relative_paths(
    units: &[MothTemplateSourceUnit],
) -> Result<(), PreparedFailure> {
    for unit in units {
        let Some(relative_path) = unit.relative_path.as_deref() else {
            continue;
        };
        let has_drive_prefix = matches!(
            relative_path.components().next(),
            Some(Component::Prefix(_))
        );
        if !has_drive_prefix {
            continue;
        }

        return Err(InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            &unit.source_path,
            format!(
                "Moth template source name {:?} yields the relative name {relative_path:?} with \
                 a drive prefix; an in-memory name must keep a portable relative suffix.",
                unit.source_path
            ),
        )
        .into());
    }

    Ok(())
}

/// The longest shared directory of the given paths, or the first path that shares none.
///
/// Each path's file component is excluded: two sibling files share their parent directory,
/// while files whose leading components differ (for example files on separate Windows drives)
/// share no ancestor directory. An input that empties the shared prefix is returned because the
/// request diagnostic must name the input that eliminated the shared basis. An empty input set
/// yields the empty path.
fn common_ancestor_directory<'a>(
    mut paths: impl Iterator<Item = &'a Path>,
) -> Result<PathBuf, &'a Path> {
    let Some(basis) = paths.next() else {
        return Ok(PathBuf::new());
    };

    let mut shared = directory_components(basis);
    for path in paths {
        let components = directory_components(path);
        let shared_length = shared
            .iter()
            .zip(components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        shared.truncate(shared_length);

        if shared.is_empty() {
            return Err(path);
        }
    }

    Ok(shared.into_iter().collect())
}

/// The directory components of one input, excluding its file component.
fn directory_components(path: &Path) -> Vec<&std::ffi::OsStr> {
    let mut components: Vec<&std::ffi::OsStr> = path
        .components()
        .map(|component| component.as_os_str())
        .collect();
    components.pop();
    components
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
/// components with `/`: a relative name must not contain a backslash at all, and a rooted name
/// may spell only its prefix and root natively. Rootedness is `Path::has_root`, so Windows treats
/// a leading `/` as the root exactly as Unix does. A backslash anywhere else would change the
/// identity once host parsing or downstream URL conversion reads it as a separator. A drive
/// prefix without a root has no portable basis and is rejected here rather than reaching the
/// relative branch and failing later as an internal logical-path error. A name with no file
/// component below its root, such as a bare UNC share, is likewise rejected as input. UTF-8
/// validation belongs to `validate_source_path_identities`, which sees every input path.
fn validate_in_memory_source_name(display_path: &Path) -> Result<(), PreparedFailure> {
    // A drive prefix without a root names a path relative to a drive's current directory, which
    // has no portable meaning. Only Windows parses a drive prefix, so this spelling is rejected
    // there and remains an ordinary relative name on other hosts.
    let has_drive_prefix = matches!(display_path.components().next(), Some(Component::Prefix(_)));
    if has_drive_prefix && !display_path.has_root() {
        return Err(InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            display_path,
            format!(
                "Moth template source name {display_path:?} has a drive prefix without a root; \
                 in-memory names must be relative or rooted."
            ),
        )
        .into());
    }

    let remainder = path_after_native_structure(display_path);

    // A non-UTF-8 spelling cannot be compared as portable text; the shared identity validation
    // reports it through the typed source-path failure before duplicate handling.
    let Some(spelling) = remainder.to_str() else {
        return Ok(());
    };

    // A name with nothing after its prefix and root has no source identity: `//server/share`
    // spells only a UNC prefix and its implicit root, while `/` and `C:\` spell only a root. The
    // implicit root occupies no bytes of the spelling, so this must be rejected before any
    // offset arithmetic and before module-root derivation.
    if spelling.is_empty() {
        return Err(InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            display_path,
            format!(
                "Moth template source name {display_path:?} has no source file component; \
                 in-memory names must name a file below their root."
            ),
        )
        .into());
    }

    // Prefix and root spellings are structural host syntax only before the portable remainder.
    // A root-relative name has a root even where the host does not call it absolute, so its root
    // still counts as structural syntax.
    if spelling.contains('\\') {
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

/// The spelling left after the native prefix and root components.
///
/// A component-length sum cannot locate this boundary: a non-verbatim UNC share reports an
/// implicit `RootDir` component that occupies no bytes of the spelling.
fn path_after_native_structure(path: &Path) -> &Path {
    let structural_components = path
        .components()
        .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        .count();

    let mut components = path.components();
    for _ in 0..structural_components {
        components.next();
    }

    components.as_path()
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
