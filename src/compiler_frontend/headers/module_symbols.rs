//! Header-owned module symbol package.
//!
//! WHAT: defines `ModuleSymbols`, the header-owned symbol metadata package built during header
//! parsing. Dependency sorting fills its complete sorted declaration placeholder list.
//! WHY: top-level symbol discovery is owned by the header stage. `ModuleSymbols` carries that
//! knowledge forward so dependency sorting and AST construction consume it directly without
//! re-iterating headers or running a separate manifest-building pass.
//!
//! ## Ownership split
//!
//! Header parsing owns:
//! - Top-level symbol discovery and metadata collection
//! - Builtin/reserved symbol registration
//!
//! Dependency sorting owns:
//! - Reconstruction of `declarations` in topologically sorted header order
//!
//! AST consumes:
//! - Header-built file visibility (via `FileVisibility`)
//!
//! AST owns:
//! - Type/constant/signature resolution
//! - Receiver-method catalog construction
//! - Body lowering and template normalization

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, type_id_hint_for_diagnostic_type,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::declaration_syntax::type_syntax::parsed_ref_to_data_type;

use crate::compiler_frontend::headers::parse_file_headers::{
    FileRole, Header, HeaderKind, RetainedDependencyClause,
};
use crate::compiler_frontend::headers::types::DependencySelection;
use crate::compiler_frontend::source::{SourceDatabase, SourceId, SourceSlot, SourceSpan};
use crate::compiler_frontend::symbols::identity::DependencySelectionId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use rustc_hash::{FxHashMap, FxHashSet};

/// Resolved target of a module-root public export entry.
///
/// WHAT: a public export exposes a source declaration, a provider-shell-keyed selection, or an
/// external package symbol through a module-root public surface. Provider selections retain their
/// completed-interface join identity until a binding consumer resolves them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PublicExportTarget {
    SourceDeclaration {
        path: PathId,
    },
    ProviderSelection {
        /// Retained selection identity used to join this re-export to the completed provider
        /// interface without re-comparing authored path spellings.
        selection: DependencySelectionId,
        /// Name selected from the provider surface before this target crosses the public-export
        /// boundary.
        source_name: StringId,
        /// Authored provider-plus-selection path retained for source diagnostics and receiver
        /// method rejection before provider projection completes.
        diagnostic_path: PathId,
    },
    External(crate::compiler_frontend::external_packages::ExternalSymbolId),
}

impl PublicExportTarget {
    pub(crate) fn source_path(&self) -> Option<&PathId> {
        match self {
            Self::SourceDeclaration { path } => Some(path),
            Self::External(_) => None,
            Self::ProviderSelection { .. } => None,
        }
    }

    /// Whether this target is the given source declaration path.
    ///
    /// WHAT: only a retained source declaration matches a source path; provider selections keep
    ///       their provider-shell identity until a binding consumer joins them to an interface.
    ///       An `External` package target never matches a source path.
    /// WHY: the header-built public export maps are the single owner of which source declarations
    ///      a module-root or source-package public surface exposes. The AST public-surface
    ///      validator and the stable source-nominal origin index share this one predicate
    ///      instead of duplicating the source/external match arms, so nameability and origin
    ///      indexing cannot drift on what a public export targets. Provider selections are
    ///      intentionally excluded because their diagnostic path is not semantic source identity.
    pub(crate) fn is_source_path(&self, path: &PathId) -> bool {
        self.source_path()
            .is_some_and(|exported_path| exported_path == path)
    }
}

/// One exported symbol in a module-root public surface.
///
/// WHAT: records the name that external consumers use and the resolved target.
/// WHY: the public API name can differ from the canonical declaration path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PublicExportEntry {
    pub export_name: StringId,
    pub target: PublicExportTarget,
}

/// Prepared entry-root boundary identity used by dependency resolution.
///
/// WHAT: carries the logical dependency prefix, module-root identity and actual prepared root file
///       together for one entry-root boundary.
/// WHY: namespace bindings must use the Stage 0-selected root file instead of reconstructing a
///      filename from the dependency prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleRootBoundary {
    pub(crate) dependency_prefix: PathId,
    pub(crate) module_root: PathId,
    pub(crate) root_file: PathId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GenericDeclarationKind {
    Function,
    Struct,
    Choice,
}

/// Dense module-local identity assigned when Stage 3 finalises declaration order.
///
/// This identity never crosses a module interface. Compile-time-only aliases and traits receive
/// IDs alongside value declarations so every ordered AST pass consumes the same Stage 3 order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DeclarationId(u32);

impl DeclarationId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index as u32)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// One identity-bearing declaration record in final Stage 3 order.
#[derive(Clone, Debug)]
pub(crate) struct OrderedSemanticDeclaration {
    pub(crate) declaration_id: DeclarationId,
    pub(crate) header_index: usize,
    pub(crate) path: PathId,
    pub(crate) kind: OrderedSemanticDeclarationKind,
    pub(crate) declaration: Option<Declaration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrderedSemanticDeclarationKind {
    TypeAlias,
    Struct,
    Choice,
    Constant,
    Trait,
    Function,
}

impl OrderedSemanticDeclarationKind {
    pub(crate) fn owns_value_row(self) -> bool {
        !matches!(self, Self::TypeAlias | Self::Trait)
    }
}

/// Header-owned module symbol package.
///
/// WHAT: carries top-level declaration placeholders, per-file dependency/export metadata, and builtin
/// type data needed by all AST passes.
///
/// WHY: header parsing discovers top-level symbols once; dependency sorting finalises their
/// identities, kinds, rows and order; AST receives this as a complete package and does not
/// rediscover those facts from paths.
///
/// ## Field lifetimes
///
/// - All order-independent maps are populated by `prepare_header_syntax` and stay unchanged
///   thereafter.
/// - `builtin_declarations` is populated by `prepare_header_syntax` and consumed (appended into
///   `compiler_owned_declarations`) by `resolve_module_dependencies`.
/// - `ordered_semantic_declarations` and `compiler_owned_declarations` are empty after
///   `prepare_header_syntax` and filled by `resolve_module_dependencies`.
#[derive(Debug, Clone)]
pub(crate) struct ModuleSymbols {
    /// Identity-bearing declaration-like headers in final Stage 3 order.
    ///
    /// This includes compile-time-only aliases and traits that do not own an AST `Declaration`
    /// row. Stage 3 assigns their shared dense module-local `DeclarationId` namespace so AST can
    /// consume final identity and order without reconstructing either fact from paths.
    pub(crate) ordered_semantic_declarations: Vec<OrderedSemanticDeclaration>,

    /// Synthetic start and builtin declarations appended after authored semantic slots.
    pub(crate) compiler_owned_declarations: Vec<Declaration>,

    // Staging: builtin declarations collected during header parsing.
    // Consumed by resolve_module_dependencies and appended to compiler-owned rows after sorting.
    pub(crate) builtin_declarations: Vec<Declaration>,

    // Order-independent maps built during header parsing.
    pub(crate) canonical_source_by_symbol_path: FxHashMap<PathId, PathId>,
    /// Exact authored declaration-name spans keyed by canonical symbol path.
    ///
    /// WHAT: carries source-owned declaration anchors into binding collision diagnostics.
    /// WHY: declaration provenance is a byte range and remains source-qualified until a
    ///      diagnostic crosses the module boundary.
    pub(crate) declaration_spans_by_symbol_path: FxHashMap<PathId, SourceSpan>,
    pub(crate) module_file_paths: FxHashSet<PathId>,
    pub(crate) file_roles_by_source: FxHashMap<PathId, FileRole>,
    pub(crate) source_ids_by_source: FxHashMap<PathId, SourceId>,
    pub(crate) file_dependency_clauses_by_source:
        FxHashMap<PathId, Vec<RetainedDependencyClause>>,
    // One flat selection table per prepared source file. Clause ranges index this table.
    pub(crate) dependency_selections_by_source: FxHashMap<PathId, Vec<DependencySelection>>,
    // Source declarations eligible for dependency-binding surfaces. Private root-file
    // declarations are intentionally absent because they are reachable only inside the root
    // file that authored them.
    pub(crate) dependency_bindable_source_symbol_paths: FxHashSet<PathId>,
    pub(crate) declared_paths_by_file: FxHashMap<PathId, FxHashSet<PathId>>,
    pub(crate) declared_names_by_file: FxHashMap<PathId, FxHashSet<StringId>>,
    // Source constants detected during header symbol collection.
    // WHY: Moth template's implicit body scope is header-stage visibility over source constants only.
    // AST later decides whether those constants actually fold to plain values or const records.
    pub(crate) constant_paths: FxHashSet<PathId>,
    pub(crate) type_alias_paths: FxHashSet<PathId>,
    pub(crate) nominal_type_paths: FxHashSet<PathId>,
    pub(crate) trait_paths: FxHashSet<PathId>,
    pub(crate) generic_declarations_by_path: FxHashMap<PathId, GenericDeclarationKind>,

    // Builtin data merged during header parsing.
    pub(crate) builtin_visible_symbol_paths: FxHashSet<PathId>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<PathId, Vec<Declaration>>,
    pub(crate) struct_source_by_path: FxHashMap<PathId, PathId>,

    // Receiver-method paths detected during header parsing.
    // WHAT: every function whose first parameter is named `this` is recorded here so binding
    //       preparation can route receiver methods to the receiver-catalog visibility path
    //       instead of treating them as free-function value members.
    // WHY: header stage needs to distinguish receiver methods from ordinary functions for
    //      namespace-record shape and direct-selection dependency routing without re-resolving signatures.
    pub(crate) receiver_method_paths: FxHashSet<PathId>,
    // Best-effort receiver type name from the parsed signature.
    // WHY: binding a struct also binds same-surface methods for that struct only. The header
    //      stage has not resolved semantic receiver types yet, but the parsed receiver name is
    //      enough to avoid binding unrelated methods from the same source file.
    pub(crate) receiver_method_receiver_names: FxHashMap<PathId, StringId>,

    // Public export data: maps source-backed package prefixes to exported root-file entries.
    // Each entry records the export name (which may differ from the target path name via alias)
    // and one of a source declaration, a provider-shell-keyed selection, or an external symbol.
    // Provider-selection diagnostic paths are authored context only, never source identity.
    pub(crate) source_package_public_exports: FxHashMap<String, FxHashSet<PublicExportEntry>>,
    // Maps each source-backed package prefix to the actual logical root source file.
    // WHY: namespace bindings need the prepared root file itself, not a synthetic path spelling,
    // because source-backed package roots usually live under configured folders such as `lib/`.
    pub(crate) source_package_root_files: FxHashMap<String, PathId>,
    // Maps source file logical path to its package prefix, if the file belongs to a source-backed package.
    pub(crate) file_package_membership: FxHashMap<PathId, String>,
    // Module root membership for entry-root files (not source-backed packages).
    // Maps file path (logical or canonical) to its module root path.
    pub(crate) file_module_membership: FxHashMap<PathId, PathId>,
    // Public exports for module roots, keyed by module root path.
    pub(crate) module_root_public_exports: FxHashMap<PathId, FxHashSet<PublicExportEntry>>,
    // Prepared entry-root boundary identities, sorted by dependency prefix longest first.
    // Used for intercepting cross-module dependencies before file resolution and for resolving the
    // actual prepared root file for namespace bindings.
    pub(crate) module_root_boundaries: Vec<ModuleRootBoundary>,
}

impl ModuleSymbols {
    /// Resolve one clause range through the selection table owned by its source file.
    pub(crate) fn selections_for_clause<'a>(
        &'a self,
        source_file: &PathId,
        clause: &RetainedDependencyClause,
    ) -> Result<&'a [DependencySelection], CompilerError> {
        let selections = self
            .dependency_selections_by_source
            .get(source_file)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        clause.selections(selections)
    }

    /// Join a module source's logical path to its build-lifetime registration slot.
    ///
    /// WHY: neighbouring tables stay keyed by `PathId`; canonical OS path and kind live
    ///      only on `SourceSlot`. Callers with no registered identity observe `None`.
    pub(crate) fn source_record<'a>(
        &'a self,
        source_file: &PathId,
        source_files: &'a SourceDatabase,
    ) -> Option<&'a SourceSlot> {
        let source_id = *self.source_ids_by_source.get(source_file)?;
        source_files.get(source_id)
    }

    pub(crate) fn empty() -> Self {
        Self {
            ordered_semantic_declarations: Vec::new(),
            compiler_owned_declarations: Vec::new(),
            builtin_declarations: Vec::new(),
            canonical_source_by_symbol_path: FxHashMap::default(),
            declaration_spans_by_symbol_path: FxHashMap::default(),
            module_file_paths: FxHashSet::default(),
            file_roles_by_source: FxHashMap::default(),
            source_ids_by_source: FxHashMap::default(),
            file_dependency_clauses_by_source: FxHashMap::default(),
            dependency_selections_by_source: FxHashMap::default(),
            dependency_bindable_source_symbol_paths: FxHashSet::default(),
            declared_paths_by_file: FxHashMap::default(),
            declared_names_by_file: FxHashMap::default(),
            constant_paths: FxHashSet::default(),
            builtin_visible_symbol_paths: FxHashSet::default(),
            builtin_struct_ast_nodes: Vec::new(),
            resolved_struct_fields_by_path: FxHashMap::default(),
            struct_source_by_path: FxHashMap::default(),
            receiver_method_paths: FxHashSet::default(),
            receiver_method_receiver_names: FxHashMap::default(),
            type_alias_paths: FxHashSet::default(),
            nominal_type_paths: FxHashSet::default(),
            trait_paths: FxHashSet::default(),
            generic_declarations_by_path: FxHashMap::default(),
            source_package_public_exports: FxHashMap::default(),
            source_package_root_files: FxHashMap::default(),
            file_package_membership: FxHashMap::default(),
            file_module_membership: FxHashMap::default(),
            module_root_public_exports: FxHashMap::default(),
            module_root_boundaries: Vec::new(),
        }
    }

    /// Build final identity-bearing semantic records from topologically ordered headers.
    ///
    /// WHAT: assigns one dense ID and typed optional value row to each semantic header, separates
    /// synthetic start rows, then appends the builtins staged during header parsing.
    ///
    /// WHY: declarations must be in the same topological order as the sorted headers so that
    /// all AST passes see dependencies before dependents. The order-independent maps were already
    /// built during `prepare_header_syntax`; only these ordered records require sorted input.
    pub(crate) fn build_sorted_declarations(
        &mut self,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) {
        self.ordered_semantic_declarations.clear();
        self.compiler_owned_declarations.clear();

        for (header_index, header) in sorted_headers.iter().enumerate() {
            if let Some(kind) = ordered_semantic_declaration_kind(&header.kind) {
                self.ordered_semantic_declarations
                    .push(OrderedSemanticDeclaration {
                        declaration_id: DeclarationId::from_index(
                            self.ordered_semantic_declarations.len(),
                        ),
                        header_index,
                        path: header.tokens.src_path.clone(),
                        kind,
                        declaration: declaration_from_header(header, string_table, path_fork),
                    });
            } else if let Some(declaration) = declaration_from_header(header, string_table, path_fork) {
                self.compiler_owned_declarations.push(declaration);
            }
        }

        // Append staged builtin declarations after all user-defined declarations.
        self.compiler_owned_declarations
            .append(&mut self.builtin_declarations);
    }
}

fn ordered_semantic_declaration_kind(
    header_kind: &HeaderKind,
) -> Option<OrderedSemanticDeclarationKind> {
    match header_kind {
        HeaderKind::TypeAlias { .. } => Some(OrderedSemanticDeclarationKind::TypeAlias),
        HeaderKind::Struct { .. } => Some(OrderedSemanticDeclarationKind::Struct),
        HeaderKind::Choice { .. } => Some(OrderedSemanticDeclarationKind::Choice),
        HeaderKind::Constant { .. } => Some(OrderedSemanticDeclarationKind::Constant),
        HeaderKind::Trait { .. } => Some(OrderedSemanticDeclarationKind::Trait),
        HeaderKind::Function { .. } => Some(OrderedSemanticDeclarationKind::Function),
        HeaderKind::ConstTemplate { .. }
        | HeaderKind::StartFunction
        | HeaderKind::TraitConformance { .. }
        | HeaderKind::TraitIncompatibility { .. } => None,
    }
}

fn declaration_from_header(
    header: &Header,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Option<Declaration> {
    match &header.kind {
        HeaderKind::Function { .. } => Some(Declaration {
            id: header.tokens.src_path.to_owned(),
            value: {
                let data_type = DataType::Function(Box::new(None), FunctionSignature::default());
                Expression::new(
                    ExpressionKind::NoValue,
                    header.name_span,
                    type_id_hint_for_diagnostic_type(&data_type),
                    data_type,
                    ValueMode::ImmutableReference,
                )
            },
            binding_span: header.name_span,
            config_qualifier: None,
        }),
        HeaderKind::Constant { declaration, .. } => Some(constant_declaration_placeholder(
            &header.tokens.src_path,
            declaration,
            header.name_span,
        )),
        HeaderKind::Struct { .. } => Some(Declaration {
            id: header.tokens.src_path.to_owned(),
            value: {
                let data_type = DataType::runtime_struct(
                    header.tokens.src_path.to_owned(),
                    builtin_type_ids::NONE,
                );
                Expression::new(
                    ExpressionKind::NoValue,
                    header.name_span,
                    type_id_hint_for_diagnostic_type(&data_type),
                    data_type,
                    ValueMode::ImmutableReference,
                )
            },
            binding_span: header.name_span,
            config_qualifier: None,
        }),
        HeaderKind::Choice { .. } => Some(Declaration {
            id: header.tokens.src_path.to_owned(),
            value: {
                let data_type = DataType::Choices {
                    nominal_path: header.tokens.src_path.to_owned(),
                    type_id: builtin_type_ids::NONE,
                    generic_instance_key: None,
                };
                Expression::new(
                    ExpressionKind::NoValue,
                    header.name_span,
                    type_id_hint_for_diagnostic_type(&data_type),
                    data_type,
                    ValueMode::ImmutableReference,
                )
            },
            binding_span: header.name_span,
            config_qualifier: None,
        }),
        HeaderKind::StartFunction => {
            // The implicit start function is a compiler-owned synthetic declaration scoped under
            // the entry source file.
            let start_name = path_fork
                .try_intern_child(
                    header.source_file,
                    string_table.intern(IMPLICIT_START_FUNC_NAME),
                )
                .expect("path table exhausted while interning implicit start path");
            Some(Declaration {
                id: start_name.to_owned(),
                value: {
                    let data_type = DataType::Function(
                        Box::new(None),
                        FunctionSignature {
                            parameters: vec![],
                            returns: vec![ReturnSlot::success(DataType::collection(
                                DataType::StringSlice,
                            ))],
                        },
                    );
                    Expression::new(
                        ExpressionKind::NoValue,
                        header.name_span,
                        type_id_hint_for_diagnostic_type(&data_type),
                        data_type,
                        ValueMode::ImmutableReference,
                    )
                },
                binding_span: None,
                config_qualifier: None,
            })
        }
        HeaderKind::TypeAlias { .. } => None,
        HeaderKind::ConstTemplate { .. } => None,
        HeaderKind::Trait { .. }
        | HeaderKind::TraitConformance { .. }
        | HeaderKind::TraitIncompatibility { .. } => None,
    }
}

fn constant_declaration_placeholder(
    path: &PathId,
    declaration: &DeclarationSyntax,
    name_span: Option<SourceSpan>,
) -> Declaration {
    Declaration {
        id: path.to_owned(),
        value: {
            let data_type = parsed_ref_to_data_type(&declaration.semantic_type());
            Expression::new(
                ExpressionKind::NoValue,
                name_span,
                type_id_hint_for_diagnostic_type(&data_type),
                data_type,
                declaration.value_mode(),
            )
        },
        binding_span: name_span,
        config_qualifier: None,
    }
}

/// Register a symbol into the declared-path and declared-name tables.
/// Dependency-bindable source symbols are also recorded for direct clause resolution.
pub(crate) fn register_declared_symbol(
    module_symbols: &mut ModuleSymbols,
    symbol_path: &PathId,
    source_file: &PathId,
    is_dependency_bindable_source_symbol: bool,
    path_fork: &PathInternerFork,
) {
    if is_dependency_bindable_source_symbol {
        module_symbols
            .dependency_bindable_source_symbol_paths
            .insert(symbol_path.to_owned());
    }
    module_symbols
        .declared_paths_by_file
        .entry(source_file.to_owned())
        .or_default()
        .insert(symbol_path.to_owned());
    if let Some(name) = path_fork.component(*symbol_path) {
        module_symbols
            .declared_names_by_file
            .entry(source_file.to_owned())
            .or_default()
            .insert(name);
    }
}
