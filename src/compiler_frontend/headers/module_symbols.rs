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
use crate::compiler_frontend::ast::statements::functions::{
    FunctionReturn, FunctionSignature, ReturnSlot,
};
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::declaration_syntax::type_syntax::parsed_ref_to_data_type;

use crate::compiler_frontend::headers::parse_file_headers::{
    FileImport, FileRole, Header, HeaderKind,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;

/// Resolved target of a module-root public export entry.
///
/// WHAT: a public export exposes either a source declaration or an external package symbol
/// through a module-root public surface.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PublicExportTarget {
    Source(InternedPath),
    External(crate::compiler_frontend::external_packages::ExternalSymbolId),
}

impl PublicExportTarget {
    /// Whether this target is the given source declaration path.
    ///
    /// WHAT: a `Source` target matches the canonical declaration path of an authored source
    ///       symbol; an `External` package target never matches a source path.
    /// WHY: the header-built public export maps are the single owner of which source declarations
    ///      a module-root or source-package public surface exposes. The AST public-surface
    ///      validator and the stable source-nominal origin index share this one predicate
    ///      instead of duplicating the source/external match arms, so nameability and origin
    ///      indexing cannot drift on what a public export targets.
    pub(crate) fn is_source_path(&self, path: &InternedPath) -> bool {
        matches!(self, PublicExportTarget::Source(exported_path) if exported_path == path)
    }
}

/// One exported symbol in a module-root public surface.
///
/// WHAT: records the name that external importers use and the resolved target.
/// WHY: the public API name can differ from the canonical declaration path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PublicExportEntry {
    pub export_name: StringId,
    pub target: PublicExportTarget,
}

/// Prepared entry-root boundary identity used by header import resolution.
///
/// WHAT: carries the logical import prefix, module-root identity and actual prepared root file
///       together for one entry-root boundary.
/// WHY: namespace imports must use the Stage 0-selected root file instead of reconstructing a
///      filename from the import prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleRootBoundary {
    pub(crate) import_prefix: InternedPath,
    pub(crate) module_root: InternedPath,
    pub(crate) root_file: InternedPath,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GenericDeclarationKind {
    Function,
    Struct,
    Choice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GenericDeclarationMetadata {
    pub(crate) kind: GenericDeclarationKind,
    pub(crate) parameters: GenericParameterList,
    pub(crate) declaration_location: SourceLocation,
}

/// Header-owned module symbol package.
///
/// WHAT: carries top-level declaration placeholders, per-file import/export metadata, and builtin
/// type data needed by all AST passes.
///
/// WHY: header parsing discovers top-level symbols once; dependency sorting finalises the
/// `declarations` order; AST receives this as a complete, pre-built package and does not
/// re-iterate headers to discover symbols.
///
/// ## Field lifetimes
///
/// - All order-independent maps are populated by `prepare_header_syntax` and stay unchanged
///   thereafter.
/// - `builtin_declarations` is populated by `prepare_header_syntax` and consumed (appended into
///   `declarations`) by `resolve_module_dependencies`.
/// - `declarations` is empty after `prepare_header_syntax` and filled by
///   `resolve_module_dependencies`.
#[derive(Debug)]
pub(crate) struct ModuleSymbols {
    // Declarations in sorted-header order.
    // Empty until resolve_module_dependencies completes; do not read before sorting.
    pub(crate) declarations: Vec<Declaration>,

    // Staging: builtin declarations collected during header parsing.
    // Consumed by resolve_module_dependencies (appended to declarations). Empty after sorting.
    pub(crate) builtin_declarations: Vec<Declaration>,

    // Order-independent maps built during header parsing.
    pub(crate) canonical_source_by_symbol_path: FxHashMap<InternedPath, InternedPath>,
    pub(crate) module_file_paths: FxHashSet<InternedPath>,
    // Per-file metadata is recorded for every prepared file, including import-only root files that
    // produce no declaration headers.
    pub(crate) file_roles_by_source: FxHashMap<InternedPath, FileRole>,
    pub(crate) canonical_os_path_by_source: FxHashMap<InternedPath, PathBuf>,
    pub(crate) file_imports_by_source: FxHashMap<InternedPath, Vec<FileImport>>,
    // Source declarations eligible for import-environment source surfaces. Private root-file
    // declarations are intentionally absent because they are reachable only inside the root
    // file that authored them.
    pub(crate) importable_source_symbol_paths: FxHashSet<InternedPath>,
    pub(crate) declared_paths_by_file: FxHashMap<InternedPath, FxHashSet<InternedPath>>,
    pub(crate) declared_names_by_file: FxHashMap<InternedPath, FxHashSet<StringId>>,
    // Source constants detected during header symbol collection.
    // WHY: Moth template's implicit body scope is header-stage visibility over source constants only.
    // AST later decides whether those constants actually fold to plain values or const records.
    pub(crate) constant_paths: FxHashSet<InternedPath>,
    pub(crate) type_alias_paths: FxHashSet<InternedPath>,
    pub(crate) nominal_type_paths: FxHashSet<InternedPath>,
    pub(crate) trait_paths: FxHashSet<InternedPath>,
    pub(crate) generic_declarations_by_path: FxHashMap<InternedPath, GenericDeclarationMetadata>,

    // Builtin data merged during header parsing.
    pub(crate) builtin_visible_symbol_paths: FxHashSet<InternedPath>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<InternedPath, Vec<Declaration>>,
    pub(crate) struct_source_by_path: FxHashMap<InternedPath, InternedPath>,

    // Receiver-method paths detected during header parsing.
    // WHAT: every function whose first parameter is named `this` is recorded here so import
    //       preparation can route receiver methods to the receiver-catalog visibility path
    //       instead of treating them as free-function value members.
    // WHY: header stage needs to distinguish receiver methods from ordinary functions for
    //      namespace-record shape and grouped-import routing without re-resolving signatures.
    pub(crate) receiver_method_paths: FxHashSet<InternedPath>,
    // Best-effort receiver type name from the parsed signature.
    // WHY: importing a struct auto-imports same-surface methods for that struct only. The header
    //      stage has not resolved semantic receiver types yet, but the parsed receiver name is
    //      enough to avoid importing unrelated methods from the same source file.
    pub(crate) receiver_method_receiver_names: FxHashMap<InternedPath, StringId>,

    // Public export data: maps source-backed package import prefixes to exported root-file entries.
    // Each entry records the export name (which may differ from the target path name via alias)
    // and the resolved target (source symbol path or external symbol id).
    pub(crate) source_package_public_exports: FxHashMap<String, FxHashSet<PublicExportEntry>>,
    // Maps source-backed package import prefix to the actual logical root source file.
    // WHY: namespace imports need the prepared root file itself, not a synthetic path spelling,
    // because source-backed package roots usually live under configured folders such as `lib/`.
    pub(crate) source_package_root_files: FxHashMap<String, InternedPath>,
    // Maps source file logical path to its package prefix, if the file belongs to a source-backed package.
    pub(crate) file_package_membership: FxHashMap<InternedPath, String>,
    // Module root membership for entry-root files (not source-backed packages).
    // Maps file path (logical or canonical) to its module root path.
    pub(crate) file_module_membership: FxHashMap<InternedPath, InternedPath>,
    // Public exports for module roots, keyed by module root path.
    pub(crate) module_root_public_exports: FxHashMap<InternedPath, FxHashSet<PublicExportEntry>>,
    // Prepared entry-root boundary identities, sorted by import prefix longest first.
    // Used for intercepting cross-module imports before file resolution and for resolving the
    // actual prepared root file for namespace imports.
    pub(crate) module_root_boundaries: Vec<ModuleRootBoundary>,
}

impl ModuleSymbols {
    pub(crate) fn empty() -> Self {
        Self {
            declarations: Vec::new(),
            builtin_declarations: Vec::new(),
            canonical_source_by_symbol_path: FxHashMap::default(),
            module_file_paths: FxHashSet::default(),
            file_roles_by_source: FxHashMap::default(),
            canonical_os_path_by_source: FxHashMap::default(),
            file_imports_by_source: FxHashMap::default(),
            importable_source_symbol_paths: FxHashSet::default(),
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

    /// Build the complete sorted declaration placeholder list from topologically ordered headers
    /// and append staged builtin declarations.
    ///
    /// WHAT: iterates the already-sorted headers to create `Declaration` placeholders for every
    /// top-level header kind (except ConstTemplate), then appends the builtin declarations that
    /// were staged during header parsing.
    ///
    /// WHY: declarations must be in the same topological order as the sorted headers so that
    /// all AST passes see dependencies before dependents. The order-independent maps were already
    /// built during `prepare_header_syntax`; only this Vec requires sorted input.
    pub(crate) fn build_sorted_declarations(
        &mut self,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
    ) {
        self.declarations.clear();

        for header in sorted_headers {
            if let Some(declaration) = declaration_from_header(header, string_table) {
                self.declarations.push(declaration);
            }
        }

        // Append staged builtin declarations after all user-defined declarations.
        self.declarations.append(&mut self.builtin_declarations);
    }
}

fn declaration_from_header(header: &Header, string_table: &mut StringTable) -> Option<Declaration> {
    match &header.kind {
        HeaderKind::Function { .. } => Some(Declaration {
            id: header.tokens.src_path.to_owned(),
            value: {
                let data_type = DataType::Function(Box::new(None), FunctionSignature::default());
                Expression::new(
                    ExpressionKind::NoValue,
                    header.name_location.to_owned(),
                    type_id_hint_for_diagnostic_type(&data_type),
                    data_type,
                    ValueMode::ImmutableReference,
                )
            },
        }),
        HeaderKind::Constant { declaration, .. } => Some(constant_declaration_placeholder(
            &header.tokens.src_path,
            declaration,
            &header.name_location,
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
                    header.name_location.to_owned(),
                    type_id_hint_for_diagnostic_type(&data_type),
                    data_type,
                    ValueMode::ImmutableReference,
                )
            },
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
                    header.name_location.to_owned(),
                    type_id_hint_for_diagnostic_type(&data_type),
                    data_type,
                    ValueMode::ImmutableReference,
                )
            },
        }),
        HeaderKind::StartFunction => {
            // The implicit start function is a compiler-owned synthetic declaration scoped under
            // the entry source file.
            let start_name = header
                .source_file
                .join_str(IMPLICIT_START_FUNC_NAME, string_table);
            Some(Declaration {
                id: start_name.to_owned(),
                value: {
                    let data_type = DataType::Function(
                        Box::new(None),
                        FunctionSignature {
                            parameters: vec![],
                            returns: vec![ReturnSlot::success(FunctionReturn::Value(
                                DataType::collection(DataType::StringSlice),
                            ))],
                        },
                    );
                    Expression::new(
                        ExpressionKind::NoValue,
                        header.name_location.to_owned(),
                        type_id_hint_for_diagnostic_type(&data_type),
                        data_type,
                        ValueMode::ImmutableReference,
                    )
                },
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
    path: &InternedPath,
    declaration: &DeclarationSyntax,
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
) -> Declaration {
    Declaration {
        id: path.to_owned(),
        value: {
            let data_type = parsed_ref_to_data_type(&declaration.semantic_type());
            Expression::new(
                ExpressionKind::NoValue,
                location.to_owned(),
                type_id_hint_for_diagnostic_type(&data_type),
                data_type,
                declaration.value_mode(),
            )
        },
    }
}

/// Register a symbol into the declared-path and declared-name tables.
/// Importable source symbols are also recorded for direct import resolution.
pub(crate) fn register_declared_symbol(
    module_symbols: &mut ModuleSymbols,
    symbol_path: &InternedPath,
    source_file: &InternedPath,
    is_importable_source_symbol: bool,
) {
    if is_importable_source_symbol {
        module_symbols
            .importable_source_symbol_paths
            .insert(symbol_path.to_owned());
    }
    module_symbols
        .declared_paths_by_file
        .entry(source_file.to_owned())
        .or_default()
        .insert(symbol_path.to_owned());
    if let Some(name) = symbol_path.name() {
        module_symbols
            .declared_names_by_file
            .entry(source_file.to_owned())
            .or_default()
            .insert(name);
    }
}
