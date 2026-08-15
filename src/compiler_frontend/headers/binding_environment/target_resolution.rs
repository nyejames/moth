//! Header-stage dependency target resolution.
//!
//! WHAT: resolves a parsed `@path/to/symbol` dependency into a concrete source symbol or external
//! package symbol.
//! WHY: keeping path-to-symbol resolution separate avoids duplicating the same exact-match,
//! suffix-match, and external-package lookup sequence across dependency callers.
//! MUST NOT: register visible names, enforce file-local collision policy, or validate export
//! flags (those belong in the orchestration layer and `visible_names.rs`).

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::{ExternalPackageRegistry, ExternalSymbolId};
use crate::compiler_frontend::headers::binding_environment::diagnostics;
use crate::compiler_frontend::headers::module_symbols::PublicExportEntry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashSet;

/// Resolved target of a single dependency path.
///
/// WHY: explicit enums make the resolution path visible in type names and match arms.
pub(crate) enum ResolvedDependencyTarget {
    Source {
        symbol_path: InternedPath,
        access: SourceDependencyAccess,
    },
    External {
        symbol_id: ExternalSymbolId,
    },
}

/// Visibility surface that allowed a source dependency.
///
/// WHY: receiver methods travel with imported receiver types, but the set of methods that may
/// travel depends on how the type was imported. Internal dependencies keep the module-local behavior,
/// direct source dependencies use source-file exports, and dependencies through public surfaces use the
/// explicit public surface that resolved the type.
#[derive(Clone, Debug)]
pub(crate) enum SourceDependencyAccess {
    Internal,
    DirectSourceExport,
    PublicExport {
        exported_entries: FxHashSet<PublicExportEntry>,
    },
}

/// Input bundle for resolving one dependency target.
///
/// WHY: avoids threading many state references as separate function parameters.
pub(crate) struct DependencyTargetResolutionInput<'a> {
    pub(crate) dependency_path: &'a InternedPath,
    pub(crate) location: &'a SourceLocation,
    pub(crate) module_file_paths: &'a FxHashSet<InternedPath>,
    pub(crate) dependency_bindable_symbol_paths: &'a FxHashSet<InternedPath>,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) string_table: &'a mut StringTable,
}

/// Result of resolving a dependency path against virtual external-package metadata.
///
/// WHY: direct external package dependencies need this lookup before source public export enforcement,
/// while ordinary source dependencies must continue through the public export path first.
pub(crate) enum ExternalPackageSymbolLookup {
    Found {
        symbol_id: ExternalSymbolId,
    },
    PackageFoundSymbolMissing {
        package_path: StringId,
        symbol_name: StringId,
    },
    NoMatch,
}

/// Input bundle for external-package-only symbol lookup.
///
/// This deliberately does not include source files or source symbols, so callers cannot use it
/// to bypass source-backed package or module-root public export checks.
pub(crate) struct ExternalPackageSymbolResolutionInput<'a> {
    pub(crate) dependency_path: &'a InternedPath,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) string_table: &'a mut StringTable,
}

/// Resolve `@package/path/symbol` against virtual external package metadata only.
pub(crate) fn resolve_external_package_symbol(
    input: ExternalPackageSymbolResolutionInput<'_>,
) -> ExternalPackageSymbolLookup {
    match resolve_virtual_package_dependency(
        input.dependency_path,
        input.external_package_registry,
        input.string_table,
    ) {
        VirtualPackageMatch::Found { symbol_id, .. } => {
            ExternalPackageSymbolLookup::Found { symbol_id }
        }
        VirtualPackageMatch::PackageFoundSymbolMissing {
            package_path,
            symbol_name,
        } => ExternalPackageSymbolLookup::PackageFoundSymbolMissing {
            package_path,
            symbol_name,
        },
        VirtualPackageMatch::NoMatch => ExternalPackageSymbolLookup::NoMatch,
    }
}

/// Resolve an `@path/to/symbol` to its concrete target.
///
/// WHAT: performs source-symbol resolution (exact match and suffix match with optional `.moth`
/// extension), file→symbol inference, and virtual-package lookup.
///
/// Returns `DirectSourceExport` for all source targets because this function does not know whether
/// the caller will later prove a more specific internal or public export access surface.
pub(crate) fn resolve_dependency_target(
    input: DependencyTargetResolutionInput<'_>,
) -> Result<ResolvedDependencyTarget, Box<CompilerDiagnostic>> {
    // Resolve as a source symbol dependency first.
    match resolve_dependency_target_path(
        input.dependency_path,
        input.dependency_bindable_symbol_paths,
        input.string_table,
    ) {
        DependencyPathMatch::Resolved(symbol_path) => Ok(ResolvedDependencyTarget::Source {
            symbol_path,
            access: SourceDependencyAccess::DirectSourceExport,
        }),
        DependencyPathMatch::Ambiguous => Err(Box::new(diagnostics::ambiguous_dependency_target(
            input.dependency_path,
            input.location.clone(),
        ))),
        DependencyPathMatch::Missing => {
            // File→symbol inference: if the path matches a source file but not a symbol,
            // try appending the path's last component to the file path as the symbol name.
            if let DependencyPathMatch::Resolved(ref file_path) = resolve_dependency_target_path(
                input.dependency_path,
                input.module_file_paths,
                input.string_table,
            ) && let Some(inferred_name) = input.dependency_path.name()
            {
                let inferred_path = file_path.append(inferred_name);
                match resolve_dependency_target_path(
                    &inferred_path,
                    input.dependency_bindable_symbol_paths,
                    input.string_table,
                ) {
                    DependencyPathMatch::Resolved(symbol_path) => {
                        return Ok(ResolvedDependencyTarget::Source {
                            symbol_path,
                            access: SourceDependencyAccess::DirectSourceExport,
                        });
                    }
                    DependencyPathMatch::Ambiguous => {
                        return Err(Box::new(diagnostics::ambiguous_dependency_target(
                            &inferred_path,
                            input.location.clone(),
                        )));
                    }
                    DependencyPathMatch::Missing => {
                        // The file exists but the inferred symbol does not.
                        // Fall through to standard error handling.
                    }
                }
            }

            // Try to resolve as a virtual package dependency.
            match resolve_external_package_symbol(ExternalPackageSymbolResolutionInput {
                dependency_path: input.dependency_path,
                external_package_registry: input.external_package_registry,
                string_table: input.string_table,
            }) {
                ExternalPackageSymbolLookup::Found { symbol_id } => {
                    return Ok(ResolvedDependencyTarget::External { symbol_id });
                }
                ExternalPackageSymbolLookup::PackageFoundSymbolMissing {
                    package_path,
                    symbol_name,
                } => {
                    return Err(Box::new(diagnostics::missing_package_symbol(
                        symbol_name,
                        package_path,
                        input.location.clone(),
                    )));
                }
                ExternalPackageSymbolLookup::NoMatch => {}
            }

            // If the path matches a module file but not a symbol, report a bare-file dependency error.
            if let DependencyPathMatch::Resolved(_) | DependencyPathMatch::Ambiguous =
                resolve_dependency_target_path(
                    input.dependency_path,
                    input.module_file_paths,
                    input.string_table,
                )
            {
                return Err(Box::new(diagnostics::bare_file_dependency(
                    input.dependency_path,
                    input.location.clone(),
                )));
            }

            Err(Box::new(diagnostics::missing_dependency_target(
                input.dependency_path,
                input.location.clone(),
            )))
        }
    }
}

/// Internal result of matching one dependency path against a candidate set.
enum DependencyPathMatch {
    Missing,
    Ambiguous,
    Resolved(InternedPath),
}

/// Match a requested path against a set of candidate paths.
///
/// WHAT: first tries exact component match (with optional source-file extensions), then tries
/// suffix match with the same source-file extension rules.
/// WHY: `@path/to/symbol` may match `@path/to/symbol.moth` or a generated content asset symbol
/// such as `@path/to/file.mtf/content` or `@path/to/file.md/content` while user dependency syntax stays
/// extensionless.
fn resolve_dependency_target_path(
    requested_path: &InternedPath,
    candidates: &FxHashSet<InternedPath>,
    string_table: &StringTable,
) -> DependencyPathMatch {
    let exact_matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| exact_path_matches_candidate(candidate, requested_path, string_table))
        .cloned()
        .collect();

    match exact_matches.len() {
        1 => {
            if let Some(path) = exact_matches.into_iter().next() {
                return DependencyPathMatch::Resolved(path);
            }
            return DependencyPathMatch::Missing;
        }
        2.. => return DependencyPathMatch::Ambiguous,
        _ => {}
    }

    let matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            candidate.ends_with(requested_path)
                || suffix_matches_with_optional_source_extension(
                    candidate,
                    requested_path,
                    string_table,
                )
        })
        .cloned()
        .collect();

    match matches.len() {
        0 => DependencyPathMatch::Missing,
        1 => matches
            .into_iter()
            .next()
            .map(DependencyPathMatch::Resolved)
            .unwrap_or(DependencyPathMatch::Missing),
        _ => DependencyPathMatch::Ambiguous,
    }
}

/// Result of attempting to resolve a dependency path as a virtual package symbol.
enum VirtualPackageMatch {
    Found {
        symbol_id: ExternalSymbolId,
    },
    PackageFoundSymbolMissing {
        package_path: StringId,
        symbol_name: StringId,
    },
    NoMatch,
}

/// Attempts to resolve a dependency path as a virtual package symbol.
///
/// WHAT: checks whether the dependency path matches `package/path/symbol` where `package/path`
/// is a known virtual package in the builder-provided registry.
/// WHY: virtual package dependencies share the same `@`-prefixed path syntax as file dependencies,
/// so they are distinguished at resolution time rather than tokenization time.
fn resolve_virtual_package_dependency(
    requested_path: &InternedPath,
    registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> VirtualPackageMatch {
    let Some(package_match) =
        registry.longest_package_prefix_for_dependency(requested_path, string_table)
    else {
        return VirtualPackageMatch::NoMatch;
    };

    let package_path = string_table.intern(&package_match.package_path);

    // The remaining components are the symbol path within the package.
    // For now, we only support a single symbol name after the package path.
    let symbol_components =
        &requested_path.as_components()[package_match.matched_component_count..];
    let symbol_name = symbol_components
        .last()
        .copied()
        .unwrap_or_else(|| string_table.intern("<unknown>"));

    if symbol_components.len() != 1 {
        // Multi-component symbol paths within packages are not supported yet.
        return VirtualPackageMatch::PackageFoundSymbolMissing {
            package_path,
            symbol_name,
        };
    }

    let symbol_name_str = string_table.resolve(symbol_name);
    if let Some(symbol_id) =
        registry.resolve_package_symbol(&package_match.package_path, symbol_name_str)
    {
        return VirtualPackageMatch::Found { symbol_id };
    }

    // Package exists but symbol doesn't — stop searching shorter prefixes
    // so we report the missing symbol accurately.
    VirtualPackageMatch::PackageFoundSymbolMissing {
        package_path,
        symbol_name,
    }
}

fn exact_path_matches_candidate(
    candidate: &InternedPath,
    requested: &InternedPath,
    string_table: &StringTable,
) -> bool {
    source_components_match(
        candidate.as_components(),
        requested.as_components(),
        string_table,
    )
}

pub(super) fn suffix_matches_with_optional_source_extension(
    candidate: &InternedPath,
    requested: &InternedPath,
    string_table: &StringTable,
) -> bool {
    if requested.len() > candidate.len() {
        return false;
    }

    let candidate_components = candidate.as_components();
    let requested_components = requested.as_components();
    let start_index = candidate_components.len() - requested_components.len();

    source_components_match(
        &candidate_components[start_index..],
        requested_components,
        string_table,
    )
}

fn source_components_match(
    candidate: &[StringId],
    requested: &[StringId],
    string_table: &StringTable,
) -> bool {
    components_match_with_optional_moth_extension(candidate, requested, string_table)
        || components_match_with_optional_content_file_extension(candidate, requested, string_table)
}

fn components_match_with_optional_moth_extension(
    candidate_components: &[crate::compiler_frontend::symbols::string_interning::StringId],
    requested_components: &[crate::compiler_frontend::symbols::string_interning::StringId],
    string_table: &StringTable,
) -> bool {
    if candidate_components.len() != requested_components.len() {
        return false;
    }

    candidate_components
        .iter()
        .zip(requested_components.iter())
        .all(|(candidate_component, requested_component)| {
            if candidate_component == requested_component {
                return true;
            }

            let candidate_str = string_table.resolve(*candidate_component);
            let requested_str = string_table.resolve(*requested_component);

            candidate_str.strip_suffix(SourceFileKind::Moth.extension_suffix())
                == Some(requested_str)
                || requested_str.strip_suffix(SourceFileKind::Moth.extension_suffix())
                    == Some(candidate_str)
        })
}

fn components_match_with_optional_content_file_extension(
    candidate_components: &[StringId],
    requested_components: &[StringId],
    string_table: &StringTable,
) -> bool {
    if candidate_components.len() != requested_components.len() {
        return false;
    }

    candidate_components
        .iter()
        .zip(requested_components.iter())
        .all(|(candidate_component, requested_component)| {
            if candidate_component == requested_component {
                return true;
            }

            let candidate_str = string_table.resolve(*candidate_component);
            let requested_str = string_table.resolve(*requested_component);

            candidate_str.strip_suffix(SourceFileKind::MothTemplate.extension_suffix())
                == Some(requested_str)
                || candidate_str.strip_suffix(SourceFileKind::PlainMarkdown.extension_suffix())
                    == Some(requested_str)
        })
}

// --------------------------
//  Namespace dependency resolution
// --------------------------

/// Resolved target of a namespace dependency (`@path` without direct selections).
///
/// WHAT: a namespace dependency resolves to either a source file surface or an external package
/// surface, producing a shallow field-access-only record in the depending file.
pub(crate) enum ResolvedNamespaceTarget {
    SourceFile(InternedPath),
    ExternalPackage { package_path: StringId },
}

/// Input bundle for resolving one namespace dependency target.
pub(crate) struct NamespaceTargetResolutionInput<'a> {
    pub(crate) dependency_path: &'a InternedPath,
    pub(crate) module_file_paths: &'a FxHashSet<InternedPath>,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) string_table: &'a mut StringTable,
}

/// Resolve a bare `@path` dependency to its namespace target.
///
/// WHAT: first checks whether the path matches a known source file (with optional `.moth`
/// extension), then checks whether it matches a known external package exactly.
/// WHY: namespace dependencies create field-access-only records; they must resolve to a concrete
/// file or package surface, not to individual symbols.
pub(crate) fn resolve_namespace_target(
    input: NamespaceTargetResolutionInput<'_>,
) -> Option<ResolvedNamespaceTarget> {
    // 1. Try to match as a source file path.
    let file_match = resolve_dependency_target_path(
        input.dependency_path,
        input.module_file_paths,
        input.string_table,
    );

    if let DependencyPathMatch::Resolved(file_path) = file_match {
        return Some(ResolvedNamespaceTarget::SourceFile(file_path));
    }

    // 2. Try to match as an external package (exact path only).
    if let Some(package_match) = input
        .external_package_registry
        .longest_package_prefix_for_dependency(input.dependency_path, input.string_table)
        && package_match.matched_component_count == input.dependency_path.len()
    {
        let package_path = input.string_table.intern(&package_match.package_path);
        return Some(ResolvedNamespaceTarget::ExternalPackage { package_path });
    }

    None
}

/// True when any component of the dependency path ends with `.moth`.
///
/// WHAT: Moth dependencies must not include the `.moth` extension. This helper detects
/// explicit `.moth` usage so callers can emit `ExplicitMothExtension`.
pub(crate) fn has_explicit_moth_extension(path: &InternedPath, string_table: &StringTable) -> bool {
    path.as_components()
        .iter()
        .any(|&component| string_table.resolve(component).ends_with(".moth"))
}
