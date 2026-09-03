//! Stage 3 dependency ordering for parsed Moth headers.
//!
//! WHAT: resolves retained local declaration-ordering hints into a dependency graph over top-level
//! headers, topologically sorts that graph, then appends `StartFunction` headers in source order.
//! Finalizes the header-owned `ModuleSymbols` package: declarations are built from the sorted
//! headers and appended with builtin declarations.
//!
//! ## Stage contract
//!
//! Stage 3 resolves retained hints into top-level declaration dependency edges.
//! Hints include type-surface and constant-initializer references.
//! Executable function/start body references remain excluded.
//!
//! **`start` excluded from the graph.** `StartFunction` headers are not graph participants — they
//! have no dependents and cannot be imported. They are appended after the sorted top-level
//! headers so AST emission sees all top-level declarations before processing the start body.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticBag};
use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::module_symbols::{ModuleSymbols, PublicExportEntry};
use crate::compiler_frontend::headers::parse_file_headers::LocalDeclarationOrderingHintOrigin;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, Header, HeaderKind, LocalDeclarationOrderingHint, TopLevelConstFragment,
};
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::{
    ResolvedFileReferenceOutcome, ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::header_log;
use rustc_hash::{FxHashMap, FxHashSet};

/// Dependency-sorted module headers with a finalized symbol package.
///
/// WHAT: the output of `resolve_module_dependencies` — sorted headers and a `ModuleSymbols`
/// whose `declarations` Vec is in dependency order and includes builtin declarations.
/// WHY: all downstream stages (AST construction, build orchestration) consume this single bundle
/// without re-sorting or re-packaging the top-level symbol data.
#[derive(Debug)]
pub(crate) struct SortedHeaders {
    pub(crate) headers: Vec<Header>,
    pub(crate) source_build_config_contracts:
        Vec<crate::compiler_frontend::declaration_syntax::build_config_contract::SourceBuildConfigContract>,
    pub(crate) top_level_const_fragments: Vec<TopLevelConstFragment>,
    pub(crate) entry_runtime_fragment_count: usize,
    pub(crate) const_fragment_count: usize,
    pub(crate) has_non_trivial_root_body: bool,
    pub(crate) module_symbols: ModuleSymbols,
    pub(crate) binding_environment: HeaderBindingEnvironment,
}

/// Stage 0 resolved content-source targets for declaration ordering.
///
/// WHAT: maps one prepared, non-dependency content-class path occurrence (the referencing `FileId`
/// plus the row's `PathSyntaxId` handle) to the exact graph key of that occurrence's resolved
/// content source's synthetic `content` constant. Occurrences whose Stage 0 outcome is not a
/// settled content target (missing, invalid, or unsupported) are absent, so ordering defers them.
/// WHY: an authored `@`-path is module-root-relative while header graph keys are logical
/// (entry-root-relative) paths, so Stage 3 resolves content edges through the canonical resolved
/// targets instead of re-deriving the authored spelling. The index is preparation and Stage 0
/// owned data; it involves no expression parsing.
pub(in crate::compiler_frontend) struct ContentSourceTargets {
    targets: FxHashMap<(FileId, PathSyntaxId), InternedPath>,
}

impl ContentSourceTargets {
    pub(in crate::compiler_frontend) fn empty() -> Self {
        Self {
            targets: FxHashMap::default(),
        }
    }

    /// Build the index from Stage 0's resolved table and the module source identities.
    pub(in crate::compiler_frontend) fn from_resolved_references(
        resolved_references: &ResolvedFileReferenceTable,
        source_files: &SourceFileTable,
        string_table: &mut StringTable,
    ) -> Self {
        let mut targets = FxHashMap::default();

        for reference in resolved_references.iter() {
            let ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
                source,
            }) = &reference.outcome
            else {
                continue;
            };
            let Some(identity) = source_files.get(*source) else {
                continue;
            };

            targets.insert(
                (reference.source_file, reference.path_syntax),
                content_constant_path(&identity.logical_path, string_table),
            );
        }

        Self { targets }
    }

    /// The resolved content constant graph key for one content hint occurrence.
    fn content_header_path(
        &self,
        hint: &LocalDeclarationOrderingHint,
        referencing_file: Option<FileId>,
    ) -> Option<&InternedPath> {
        self.targets.get(&(referencing_file?, hint.occurrence()?))
    }
}

/// Tracks which modules are temporarily marked (in the current DFS stack)
/// and which have been permanently visited.
struct DependencyTracker {
    temp_mark: FxHashSet<InternedPath>,
    visited: FxHashSet<InternedPath>,
    stack: Vec<InternedPath>,
    visit_count: usize,
}

impl DependencyTracker {
    fn new(capacity: usize) -> Self {
        DependencyTracker {
            temp_mark: FxHashSet::with_capacity_and_hasher(capacity, Default::default()),
            visited: FxHashSet::with_capacity_and_hasher(capacity, Default::default()),
            stack: Vec::with_capacity(capacity),
            visit_count: 0,
        }
    }

    fn enter(&mut self, path: InternedPath) {
        self.temp_mark.insert(path.to_owned());
        self.stack.push(path);
    }

    fn is_in_current_stack(&self, path: &InternedPath) -> bool {
        self.temp_mark.contains(path)
    }

    fn abandon(&mut self, path: &InternedPath) {
        self.temp_mark.remove(path);

        if self.stack.last() == Some(path) {
            self.stack.pop();
        } else {
            self.stack.retain(|stack_path| stack_path != path);
        }
    }

    fn finish(&mut self, path: &InternedPath) {
        self.abandon(path);
        self.visited.insert(path.to_owned());
    }
}

/// Header dependency graph plus the path-resolution rules that make graph edges sortable.
///
/// WHAT: owns the graph keys, source-order indexes, and resolved edge construction used by DFS.
/// WHY: dependency sorting needs one place where exact header membership, source-backed package public
/// fallback, same-file hint deferral, and stable edge ordering are applied consistently.
struct DependencyGraph<'a> {
    headers_by_path: FxHashMap<InternedPath, Header>,
    source_order_by_path: FxHashMap<InternedPath, usize>,
    ordered_paths: Vec<InternedPath>,
    source_package_public_exports: &'a FxHashMap<String, FxHashSet<PublicExportEntry>>,
    provider_interface_paths: &'a FxHashMap<InternedPath, OriginDeclarationId>,
    content_source_targets: &'a ContentSourceTargets,
}

impl<'a> DependencyGraph<'a> {
    fn from_headers(
        headers: Vec<Header>,
        source_package_public_exports: &'a FxHashMap<String, FxHashSet<PublicExportEntry>>,
        provider_interface_paths: &'a FxHashMap<InternedPath, OriginDeclarationId>,
        content_source_targets: &'a ContentSourceTargets,
        _string_table: &StringTable,
    ) -> Self {
        let mut headers_by_path: FxHashMap<InternedPath, Header> =
            FxHashMap::with_capacity_and_hasher(headers.len(), Default::default());
        let mut source_order_by_path: FxHashMap<InternedPath, usize> =
            FxHashMap::with_capacity_and_hasher(headers.len(), Default::default());
        let mut ordered_paths: Vec<InternedPath> = Vec::with_capacity(headers.len());

        for header in headers {
            header_log!(header);

            let path = header.tokens.src_path.to_owned();
            source_order_by_path.insert(path.to_owned(), ordered_paths.len());
            ordered_paths.push(path.to_owned());
            headers_by_path.insert(path, header);
        }

        Self {
            headers_by_path,
            source_order_by_path,
            ordered_paths,
            source_package_public_exports,
            provider_interface_paths,
            content_source_targets,
        }
    }

    fn len(&self) -> usize {
        self.headers_by_path.len()
    }

    fn ordered_paths(&self) -> &[InternedPath] {
        &self.ordered_paths
    }

    fn header_for_path(&self, path: &InternedPath) -> Option<&Header> {
        self.headers_by_path.get(path)
    }

    fn resolve_requested_path(
        &self,
        requested_path: &InternedPath,
        string_table: &StringTable,
    ) -> Option<ResolvedGraphPath> {
        if self.headers_by_path.contains_key(requested_path) {
            return Some(ResolvedGraphPath::Header(requested_path.to_owned()));
        }
        if self.provider_interface_paths.contains_key(requested_path) {
            return Some(ResolvedGraphPath::ProviderInterface(
                requested_path.to_owned(),
            ));
        }

        // Source-backed package public dependencies can use a public prefix path that differs from the
        // concrete root-file header path. Accept those public edges without treating them as
        // graph nodes.
        if self.is_source_package_public_export_path(requested_path, string_table) {
            return Some(ResolvedGraphPath::SourcePackagePublicExport(
                requested_path.to_owned(),
            ));
        }

        None
    }

    fn source_order_for_requested_path(
        &self,
        requested_path: &InternedPath,
        string_table: &StringTable,
    ) -> Option<usize> {
        let resolved_path = match self.resolve_requested_path(requested_path, string_table)? {
            ResolvedGraphPath::Header(path) => path,
            ResolvedGraphPath::ProviderInterface(_)
            | ResolvedGraphPath::SourcePackagePublicExport(_) => return None,
        };

        self.source_order_by_path.get(&resolved_path).copied()
    }

    fn sorted_dependency_edges_for_header(
        &self,
        header: &Header,
        string_table: &StringTable,
    ) -> Vec<ResolvedDependencyEdge> {
        // Hints retain every authored occurrence for diagnostics and Stage 0 handoff, but graph
        // traversal needs one edge per resolved target. In particular, repeated file-value paths
        // can point at one synthetic `content` header even though their PathSyntaxIds differ.
        let mut seen_targets = FxHashSet::default();
        let mut edges = Vec::with_capacity(header.local_ordering_hints.len());
        for hint in &header.local_ordering_hints {
            let edge = self.resolve_dependency_edge(header, hint, string_table);
            let keep = edge
                .resolved_path
                .as_ref()
                .is_none_or(|resolved_path| seen_targets.insert(resolved_path.clone()));
            if keep {
                edges.push(edge);
            }
        }

        edges.sort_by(|left, right| {
            let left_order = self.source_order_for_edge(left);
            let right_order = self.source_order_for_edge(right);

            left_order.cmp(&right_order).then_with(|| {
                left.requested_path
                    .to_portable_string(string_table)
                    .cmp(&right.requested_path.to_portable_string(string_table))
            })
        });

        edges
    }

    fn resolve_dependency_edge(
        &self,
        header: &Header,
        hint: &LocalDeclarationOrderingHint,
        string_table: &StringTable,
    ) -> ResolvedDependencyEdge {
        // Stage 3 turns a retained local declaration-ordering hint into a sortable graph edge.
        // It consumes the typed hint rather than reconstructing the requested path from
        // RetainedDependencyClause or source syntax. Content hints resolve their exact graph key
        // through the Stage 0 resolved-reference table instead of re-deriving the authored
        // spelling, so a module-relative authored occurrence still targets the canonical
        // content constant.
        let location = header.name_location.to_owned();
        let requested_path = if hint.origin() == LocalDeclarationOrderingHintOrigin::ContentSource {
            let Some(resolved_path) = self
                .content_source_targets
                .content_header_path(hint, header.tokens.file_id)
            else {
                return ResolvedDependencyEdge {
                    requested_path: hint.path().to_owned(),
                    resolved_path: None,
                    location,
                    source_order: None,
                    kind: DependencyEdgeKind::MissingContentSource,
                };
            };

            resolved_path
        } else {
            hint.path()
        };

        let source_order = self.source_order_for_requested_path(requested_path, string_table);

        match self.resolve_requested_path(requested_path, string_table) {
            Some(ResolvedGraphPath::Header(resolved_path)) => ResolvedDependencyEdge {
                requested_path: requested_path.to_owned(),
                resolved_path: Some(resolved_path),
                location,
                source_order,
                kind: DependencyEdgeKind::GraphHeader,
            },
            Some(ResolvedGraphPath::SourcePackagePublicExport(resolved_path)) => {
                ResolvedDependencyEdge {
                    requested_path: requested_path.to_owned(),
                    resolved_path: Some(resolved_path),
                    location,
                    source_order,
                    kind: DependencyEdgeKind::SourcePackagePublicExport,
                }
            }
            Some(ResolvedGraphPath::ProviderInterface(resolved_path)) => ResolvedDependencyEdge {
                requested_path: requested_path.to_owned(),
                resolved_path: Some(resolved_path),
                location,
                source_order,
                kind: DependencyEdgeKind::ProviderInterface,
            },
            None if self.is_same_file_symbol_hint(requested_path, &header.source_file) => {
                ResolvedDependencyEdge {
                    requested_path: requested_path.to_owned(),
                    resolved_path: None,
                    location,
                    source_order,
                    kind: DependencyEdgeKind::SameFileSymbolHint,
                }
            }
            None if hint.origin() == LocalDeclarationOrderingHintOrigin::ContentSource => {
                ResolvedDependencyEdge {
                    requested_path: requested_path.to_owned(),
                    resolved_path: None,
                    location,
                    source_order,
                    kind: DependencyEdgeKind::MissingContentSource,
                }
            }
            None => ResolvedDependencyEdge {
                requested_path: requested_path.to_owned(),
                resolved_path: None,
                location,
                source_order,
                kind: DependencyEdgeKind::Missing,
            },
        }
    }

    fn source_order_for_edge(&self, edge: &ResolvedDependencyEdge) -> usize {
        edge.source_order.unwrap_or(usize::MAX)
    }

    fn is_source_package_public_export_path(
        &self,
        path: &InternedPath,
        string_table: &StringTable,
    ) -> bool {
        let components = path.as_components();
        if components.is_empty() {
            return false;
        }

        let first_component = string_table.resolve(components[0]);
        let Some(export_name) = path.name() else {
            return false;
        };

        if let Some(entries) = self.source_package_public_exports.get(first_component) {
            for entry in entries {
                if entry.export_name == export_name {
                    return true;
                }
            }
        }

        false
    }

    fn is_same_file_symbol_hint(&self, path: &InternedPath, source_file: &InternedPath) -> bool {
        path.parent().as_ref() == Some(source_file)
    }
}

struct ResolvedDependencyEdge {
    requested_path: InternedPath,
    resolved_path: Option<InternedPath>,
    location: SourceLocation,
    source_order: Option<usize>,
    kind: DependencyEdgeKind,
}

enum DependencyEdgeKind {
    GraphHeader,
    ProviderInterface,
    SourcePackagePublicExport,
    SameFileSymbolHint,
    /// A content-source hint whose synthetic content constant never became a graph header.
    ///
    /// WHY: Stage 0 retains the user-facing diagnostic for a missing, invalid, or unsupported
    /// content target; ordering must defer to that lane instead of raising a second error.
    MissingContentSource,
    Missing,
}

enum ResolvedGraphPath {
    Header(InternedPath),
    ProviderInterface(InternedPath),
    SourcePackagePublicExport(InternedPath),
}

/// Boxed diagnostic result shared by the DFS visit and edge-recursion boundaries.
///
/// WHAT: one file-local alias for the boxed `CompilerDiagnostic` error variant returned by
/// `visit_node` and `visit_dependency_edge`.
/// WHY: recursive visits propagate one diagnostic through nested edges, while the outer resolver
/// owns plain diagnostic accumulation. The single unbox stays at that `DiagnosticBag` boundary.
type VisitResult = Result<(), Box<CompilerDiagnostic>>;

/// Topologically sort headers and finalize the header-owned module symbol package.
///
/// WHAT: resolves retained hints, sorts top-level declaration headers (non-start) by the resulting
/// dependency edges, then appends `StartFunction` headers in source order. Builds the
/// `declarations` Vec in sorted order and appends builtin declarations.
///
/// WHY: `StartFunction` is excluded from the dependency graph — it is build-system-only and
/// cannot be imported by other headers. All other headers are sorted by edges resolved from their
/// retained type-surface and constant-initializer hints so AST sees dependencies first.
pub(in crate::compiler_frontend) fn resolve_module_dependencies(
    parsed: BoundModuleHeaders,
    content_source_targets: &ContentSourceTargets,
    string_table: &mut StringTable,
) -> Result<SortedHeaders, DiagnosticBag> {
    let BoundModuleHeaders {
        headers,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        mut module_symbols,
        binding_environment,
        ..
    } = parsed;

    // Partition: StartFunction headers are appended last, not sorted.
    // WHY: start is build-system-only and has no dependents. Module-root declarations remain graph
    // participants because other modules can depend on their public constants and type surfaces;
    // header dependency visibility, not dependency sorting, owns whether those declarations are visible.
    let mut start_headers: Vec<Header> = Vec::new();
    let top_level_headers: Vec<Header> = headers
        .into_iter()
        .filter_map(|h| {
            if matches!(h.kind, HeaderKind::StartFunction) {
                start_headers.push(h);
                None
            } else {
                Some(h)
            }
        })
        .collect();
    let dependency_header_count = top_level_headers.len();
    let dependency_edge_count = top_level_headers
        .iter()
        .map(|header| header.local_ordering_hints.len())
        .sum();

    let (mut sorted, dependency_visit_count) = {
        let graph = DependencyGraph::from_headers(
            top_level_headers,
            &module_symbols.source_package_public_exports,
            &binding_environment.imported_declarations_by_local_path,
            content_source_targets,
            string_table,
        );
        let mut diagnostic_bag = DiagnosticBag::new();

        // Resolve retained hints and topologically sort the resulting dependency edges.
        let mut tracker = DependencyTracker::new(graph.len());
        let mut sorted: Vec<Header> = Vec::with_capacity(graph.len());

        for path in graph.ordered_paths() {
            if !tracker.visited.contains(path) {
                let diagnostic_location = graph
                    .header_for_path(path)
                    .map(|header| header.name_location.to_owned())
                    .unwrap_or_default();

                if let Err(error) = visit_node(
                    path,
                    &graph,
                    &mut tracker,
                    &mut sorted,
                    string_table,
                    diagnostic_location,
                ) {
                    diagnostic_bag.push(*error);
                }
            }
        }

        if diagnostic_bag.has_errors() {
            return Err(diagnostic_bag);
        }

        (sorted, tracker.visit_count)
    };

    // Append start headers after the sorted top-level headers.
    // WHY: start sees all declarations and cannot be imported, so it must come last.
    sorted.extend(start_headers);

    // Build the complete sorted declaration placeholder list from the topologically
    // ordered headers and append builtins.
    // WHY: declarations must be in sorted order so AST passes see dependencies before dependents.
    module_symbols.build_sorted_declarations(&sorted, string_table);

    add_frontend_counter(
        FrontendCounter::DependencyHeaderCount,
        dependency_header_count,
    );
    add_frontend_counter(FrontendCounter::DependencyEdgeCount, dependency_edge_count);
    add_frontend_counter(
        FrontendCounter::DependencyVisitCount,
        dependency_visit_count,
    );

    Ok(SortedHeaders {
        headers: sorted,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        module_symbols,
        binding_environment,
    })
}

/// DFS visit for one module node, pushing clones into `sorted`.
fn visit_node(
    node_path: &InternedPath,
    graph: &DependencyGraph<'_>,
    tracker: &mut DependencyTracker,
    sorted: &mut Vec<Header>,
    string_table: &mut StringTable,
    diagnostic_location: SourceLocation,
) -> VisitResult {
    tracker.visit_count += 1;

    let Some(resolved_graph_path) = graph.resolve_requested_path(node_path, string_table) else {
        return Err(Box::new(CompilerDiagnostic::missing_import_target(
            node_path.to_owned(),
            diagnostic_location,
        )));
    };

    let resolved_path = match resolved_graph_path {
        ResolvedGraphPath::Header(path) => path,
        ResolvedGraphPath::ProviderInterface(path)
        | ResolvedGraphPath::SourcePackagePublicExport(path) => {
            tracker.visited.insert(path);
            return Ok(());
        }
    };

    if tracker.is_in_current_stack(&resolved_path) {
        return Err(Box::new(CompilerDiagnostic::circular_dependency(
            resolved_path,
            diagnostic_location,
        )));
    }

    if !tracker.visited.contains(&resolved_path) {
        let Some(header) = graph.header_for_path(&resolved_path) else {
            return Err(Box::new(
                CompilerError::new(
                    format!(
                        "Dependency ordering resolved '{}' but it was missing from the graph.",
                        resolved_path.to_portable_string(string_table)
                    ),
                    diagnostic_location,
                    ErrorType::Compiler,
                )
                .into(),
            ));
        };

        tracker.enter(resolved_path.to_owned());

        // Recurse on the dependency edges resolved from this header's retained hints.
        // WHY: edges include type surfaces and constant initializer references.
        // Executable body references are excluded.
        let dependency_edges = graph.sorted_dependency_edges_for_header(header, string_table);
        for edge in dependency_edges {
            if let Err(error) = visit_dependency_edge(edge, graph, tracker, sorted, string_table) {
                tracker.abandon(&resolved_path);
                return Err(error);
            }
        }

        sorted.push(header.clone());
        tracker.finish(&resolved_path);
    }

    Ok(())
}

fn visit_dependency_edge(
    edge: ResolvedDependencyEdge,
    graph: &DependencyGraph<'_>,
    tracker: &mut DependencyTracker,
    sorted: &mut Vec<Header>,
    string_table: &mut StringTable,
) -> VisitResult {
    match edge.kind {
        DependencyEdgeKind::GraphHeader => {
            let Some(resolved_path) = edge.resolved_path else {
                return Err(Box::new(
                    CompilerError::new(
                        "Dependency edge was classified as a graph header without a resolved path.",
                        edge.location,
                        ErrorType::Compiler,
                    )
                    .into(),
                ));
            };

            visit_node(
                &resolved_path,
                graph,
                tracker,
                sorted,
                string_table,
                edge.location,
            )
        }

        DependencyEdgeKind::ProviderInterface | DependencyEdgeKind::SourcePackagePublicExport => {
            if let Some(resolved_path) = edge.resolved_path {
                tracker.visited.insert(resolved_path);
            }

            Ok(())
        }

        DependencyEdgeKind::SameFileSymbolHint => {
            // Same-file named-type edges are only ordering hints while header parsing is still
            // discovering the file. If the target never materializes as a header, let later type
            // resolution emit the user-facing "Unknown type" diagnostic.
            Ok(())
        }

        DependencyEdgeKind::MissingContentSource => {
            // A content target that never materialized is diagnosed through the Stage 0
            // resolved-reference lane (missing, invalid, or unsupported target), so ordering
            // defers instead of raising a second error for the same occurrence.
            Ok(())
        }

        DependencyEdgeKind::Missing => Err(Box::new(CompilerDiagnostic::missing_import_target(
            edge.requested_path,
            edge.location,
        ))),
    }
}

#[cfg(test)]
#[path = "tests/module_dependencies_tests.rs"]
mod module_dependencies_tests;
