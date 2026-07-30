//! Header-stage data contracts.
//!
//! WHAT: shared structs/enums produced by header parsing and consumed by dependency sorting,
//! AST construction, and module symbol collection.
//! WHY: keeping these types separate from parser control flow makes the header-stage API obvious
//! and avoids making `parse_file_headers.rs` the dumping ground for every header concern.

use crate::compiler_frontend::arena::{HeaderStats, TokenStats};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantSyntax;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionSignatureSyntax, SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::import_environment::HeaderImportEnvironment;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::paths::const_paths::StructuralProviderReference;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::traits::syntax::{
    TraitConformanceSyntax, TraitDeclarationSyntax, TraitIncompatibilitySyntax,
};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use std::collections::HashSet;
use std::fmt::Display;

/// Provider-independent retained header syntax produced before provider interfaces exist.
///
/// WHAT: aggregates per-file declaration shells, import shells, order-independent local/module
/// symbol facts, root-activity/fragment metadata, and token/header statistics from all prepared
/// files in one module.
/// WHY: syntax preparation is the only phase that reads token streams and discovers module-wide
/// top-level declaration syntax. It must complete before provider interfaces are available so the
/// build system can schedule binding later without retokenizing or reparsing source.
///
/// `module_symbols` carries all order-independent top-level symbol metadata collected during
/// header parsing. `declarations` inside it is empty until dependency sorting completes.
pub struct PreparedHeaderSyntax {
    pub headers: Vec<Header>,
    pub top_level_const_fragments: Vec<TopLevelConstFragment>,
    /// Number of top-level runtime templates in the active module root.
    ///
    /// WHY: only the active module root produces runtime slots; header parsing is the single authoritative
    /// counter so builders do not need to re-scan HIR for `PushRuntimeFragment` statements.
    pub entry_runtime_fragment_count: usize,
    /// Number of top-level const fragments in the active module root.
    pub const_fragment_count: usize,
    /// Whether the active module root contains non-trivial top-level root/start code.
    ///
    /// WHY: header parsing is the first stage that can classify root activity without asking
    /// AST or a builder to rediscover it from tokens or HIR.
    pub has_non_trivial_root_body: bool,
    /// Aggregate cheap token classification for this module.
    ///
    /// WHAT: the sum of per-file `TokenStats` gathered during tokenization.
    /// WHY: provides a policy-only seed for arena capacity heuristics without re-tokenizing.
    pub token_stats: TokenStats,
    /// Aggregate cheap header classification for this module.
    ///
    /// WHAT: counts of declaration headers, their generic parameters, signature members,
    ///       choice variants, and dependency edges.
    /// WHY: provides a policy-only seed for arena capacity heuristics.
    pub header_stats: HeaderStats,
    /// Header-owned module symbol package with order-independent symbol facts.
    ///
    /// WHY: top-level symbol discovery is owned by the header preparation phase; binding
    /// mutates this with public-surface entries, then dependency sorting and AST construction
    /// consume it without a separate manifest-building step.
    pub module_symbols: ModuleSymbols,
}

/// Bound module headers produced by consuming `PreparedHeaderSyntax` through interface binding.
///
/// WHAT: owns the completed public-surface/import environment and dependency facts required by
/// dependency sorting and AST. Produced only by `bind_module_headers`, which resolves retained
/// import shells against immutable provider interfaces, canonicalizes dependency edges, and
/// completes constant initializer dependencies.
/// WHY: binding does not retokenize source or reparse declaration syntax — it consumes the
/// retained `PreparedHeaderSyntax` and adds the provider-dependent facts that cannot be known
/// before the provider graph has compiled.
pub struct BoundModuleHeaders {
    pub headers: Vec<Header>,
    pub top_level_const_fragments: Vec<TopLevelConstFragment>,
    pub entry_runtime_fragment_count: usize,
    pub const_fragment_count: usize,
    pub has_non_trivial_root_body: bool,
    pub token_stats: TokenStats,
    pub header_stats: HeaderStats,
    pub module_symbols: ModuleSymbols,
    /// Header-built per-file import visibility environment.
    ///
    /// WHY: import binding and visibility construction is owned by the header binding phase; AST
    /// consumes this directly without rebuilding import bindings or rediscovering visibility.
    pub import_environment: HeaderImportEnvironment,
}

/// Placement metadata for one compile-time top-level template in the active module root.
///
/// WHAT: records where a const fragment should be inserted relative to runtime fragments
/// in the final merged output.
/// WHY: only const fragments carry insertion metadata; runtime fragments are returned by
/// `start()` in source order and need no separate metadata.
#[derive(Clone, Debug)]
pub struct TopLevelConstFragment {
    /// Number of runtime fragments seen before this const fragment in source order.
    /// Used by the builder to insert the const string at the correct position.
    pub runtime_insertion_index: usize,
    pub header_path: InternedPath,
    pub location: SourceLocation,
}

/// Optional settings that affect module header parsing.
///
/// WHAT: bundles optional entry identity and path-resolution behavior for one parse invocation.
/// WHY: the parser is called from both production and tests, and grouping these keeps the API concise.
#[derive(Clone)]
pub struct HeaderParseOptions {
    pub entry_file_id: Option<FileId>,
    pub project_path_resolver: Option<ProjectPathResolver>,
    /// The graph-owned semantic role of the root currently being compiled.
    ///
    /// WHY: entry-path equality identifies which file is active but cannot decide whether that
    ///      root owns dormant runtime work. Support and project-facade roots are API-only even
    ///      while they are the active compilation root.
    pub active_root_role: ModuleRootRole,
}

impl Default for HeaderParseOptions {
    fn default() -> Self {
        Self {
            entry_file_id: None,
            project_path_resolver: None,
            active_root_role: ModuleRootRole::Normal,
        }
    }
}

#[derive(Clone, Debug)]
pub enum HeaderKind {
    Function {
        generic_parameters: GenericParameterList,
        signature: FunctionSignatureSyntax,
    },
    Constant {
        declaration: DeclarationSyntax,
    },
    Struct {
        generic_parameters: GenericParameterList,
        fields: Vec<SignatureMemberSyntax>,
    },
    Choice {
        generic_parameters: GenericParameterList,
        variants: Vec<ChoiceVariantSyntax>,
    },
    TypeAlias {
        target: ParsedTypeRef,
    },

    ConstTemplate {
        condition_references: Vec<InitializerReference>,
    },

    /// The active-root start function for non-header top-level statements.
    ///
    /// WHAT: captures top-level executable statements that are not declarations.
    /// WHY: only the active module root produces a start function. Ordinary source files with
    /// non-trivial top-level executable code are rejected as a rule error; imported roots discard
    /// their root body before this output is assembled.
    /// Start functions are build-system-only; they are not importable or callable from modules.
    StartFunction,

    /// Trait declaration: `TRAIT must: requirements ;`
    ///
    /// WHAT: parse-only shell for a trait declaration discovered at the header stage.
    /// WHY: trait declarations are top-level declarations that participate in normal
    ///      module symbol collection; semantic resolution happens during AST environment
    ///      construction.
    Trait {
        declaration: TraitDeclarationSyntax,
    },

    /// Trait conformance declaration: `Type must TRAIT, TRAIT`
    ///
    /// WHAT: parse-only shell for an explicit conformance declaration.
    /// WHY: conformance declarations are bodyless top-level declarations discovered at
    ///      the header stage; evidence validation happens during AST environment construction.
    TraitConformance {
        conformance: TraitConformanceSyntax,
    },

    /// Trait incompatibility declaration: `TRAIT must not TRAIT, TRAIT`
    ///
    /// WHAT: parse-only shell for a source-authored mutual incompatibility between traits.
    /// WHY: incompatibility declarations are bodyless top-level metadata discovered at the
    ///      header stage; semantic resolution and symmetric recording happen during AST
    ///      environment construction after trait registration.
    TraitIncompatibility {
        incompatibility: TraitIncompatibilitySyntax,
    },
}

/// Explicit export mode for a parsed header or file import.
///
/// WHAT: distinguishes private source-file items from public module-root API surface.
/// WHY: module-root files use one explicit `export:` block to mark public declarations and
/// grouped re-exports. All other files keep every declaration as `Private`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderExportMode {
    /// Private to the source file or importing file.
    Private,
    /// Public module API entry exposed through a module-root file.
    Public,
}

impl HeaderExportMode {
    pub fn is_public(&self) -> bool {
        matches!(self, HeaderExportMode::Public)
    }
}

/// A conservative declaration-shell ordering fact retained before provider binding.
///
/// WHAT: one referenced path captured from a declaration shell's type surface or constant
/// initializer, recorded in the import spelling or same-file spelling seen during syntax
/// preparation. It is not an already-proven graph edge.
/// WHY: Stage 2 retains these hints without knowing which imports are source graph participants
/// versus virtual or provider bindings. Stage 3 alone resolves retained local hints into
/// sortable graph edges after binding has canonicalized or dropped import-spelled hints.
/// MUST NOT: carry alias, export, or provider classification; that metadata stays on
/// `FileImport` and `StructuralProviderReference`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalDeclarationOrderingHint {
    /// The conservative referenced path: an import spelling or a same-file spelling.
    pub path: InternedPath,
}

impl LocalDeclarationOrderingHint {
    /// Wrap one conservative referenced path as a retained ordering hint.
    pub fn new(path: InternedPath) -> Self {
        Self { path }
    }

    /// The conservative referenced path this hint records.
    pub fn path(&self) -> &InternedPath {
        &self.path
    }

    /// Remap the interned path into a merged string table.
    ///
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    ///      table requires shifting the `InternedPath` so later stages resolve the hint through the
    ///      global table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.path.remap_string_ids(remap);
    }
}

#[derive(Clone, Debug)]
pub struct Header {
    pub kind: HeaderKind,
    /// The role of the source file that produced this header.
    ///
    /// WHAT: distinguishes active roots, imported roots, and normal source files.
    /// WHY: visibility and export decisions depend on file role and declaration kind,
    /// not file role alone.
    pub file_role: FileRole,
    /// Whether this header is part of the module public surface.
    ///
    /// WHAT: `Public` only for items inside a module root's `export:` block; `Private` everywhere
    /// else.
    /// WHY: import preparation builds module APIs from explicit public-surface metadata, not from
    /// file role alone.
    pub export_mode: HeaderExportMode,
    /// Conservative local declaration-ordering hints retained before provider binding.
    ///
    /// WHAT: referenced paths from this declaration shell's type surface and constant initializer,
    /// recorded in the import or same-file spelling seen during syntax preparation. These are
    /// ordering hints, not already-proven graph edges.
    /// WHY: binding canonicalizes or drops import-spelled hints using bound visibility, then
    /// Stage 3 resolves the retained local hints into sortable graph edges.
    pub local_ordering_hints: HashSet<LocalDeclarationOrderingHint>,
    pub name_location: SourceLocation,

    // Token Body (for functions / templates) and info about canonical_os_path
    pub tokens: FileTokens,

    pub source_file: InternedPath,
    /// Bare fixed-capacity constant references discovered in type annotations on this header.
    ///
    /// WHAT: value-namespace references from fixed-collection capacity annotations.
    /// WHY: dependency sorting must order referenced constants before the declaration that
    ///      uses them, even when the declaration itself is not a constant.
    pub capacity_references: Vec<InitializerReference>,
}

impl Display for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Header kind: {:#?}", self.kind)
    }
}

impl TopLevelConstFragment {
    /// Remap every interned string owned by this fragment into the merged global string table.
    ///
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    /// table requires shifting every `StringId`, `InternedPath`, and `SourceLocation` so later
    /// stages resolve names through the global table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.header_path.remap_string_ids(remap);
        self.location.remap_string_ids(remap);
    }
}

impl FileImport {
    /// Remap every interned string owned by this import into the merged global string table.
    ///
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    /// table requires shifting every `StringId`, `InternedPath`, and `SourceLocation` so later
    /// stages resolve names through the global table. The nested structural provider reference
    /// remaps exactly once here alongside the alias and clause-location metadata.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.provider.remap_string_ids(remap);
        self.authored_provider.remap_string_ids(remap);

        if let Some(alias) = &mut self.alias {
            *alias = remap.get(*alias);
        }

        self.location.remap_string_ids(remap);

        if let Some(alias_location) = &mut self.alias_location {
            alias_location.remap_string_ids(remap);
        }
    }
}

impl HeaderKind {
    /// Whether this header kind is an authored declaration that may participate in a
    /// module-root public export surface.
    ///
    /// WHAT: returns `true` exactly for the authored declaration kinds: functions, structs,
    ///       choices, transparent type aliases, compile-time constants and trait declarations.
    ///       Returns `false` for const templates, the implicit active-root start function, trait
    ///       conformance and trait incompatibility, which are not exportable declarations.
    /// WHY: the header, AST public-surface and semantic-origin stages all need the same
    ///      declaration-kind gate to decide which headers may become public export entries.
    ///      Owning the kind set on `HeaderKind` keeps one declaration-kind authority so the three
    ///      stage-local public-export predicates cannot drift. Each predicate keeps its own
    ///      file-role and export-mode policy; this method owns only the declaration-kind policy.
    pub fn is_authored_public_export_declaration(&self) -> bool {
        matches!(
            self,
            HeaderKind::Function { .. }
                | HeaderKind::Struct { .. }
                | HeaderKind::Choice { .. }
                | HeaderKind::TypeAlias { .. }
                | HeaderKind::Trait { .. }
                | HeaderKind::Constant { .. }
        )
    }

    /// Remap every interned string owned by this header kind into the merged global string table.
    ///
    /// WHAT: dispatches to nested remap methods for function signatures, declaration shells,
    ///       struct fields, choice variants, and type-alias targets.
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    ///      table requires shifting every `StringId`, `InternedPath`, and `SourceLocation` so later
    ///      stages resolve names through the global table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            HeaderKind::Function {
                generic_parameters,
                signature,
            } => {
                generic_parameters.remap_string_ids(remap);
                signature.remap_string_ids(remap);
            }

            HeaderKind::Constant { declaration, .. } => {
                declaration.remap_string_ids(remap);
            }

            HeaderKind::Struct {
                generic_parameters,
                fields,
            } => {
                generic_parameters.remap_string_ids(remap);
                for field in fields {
                    field.remap_string_ids(remap);
                }
            }

            HeaderKind::Choice {
                generic_parameters,
                variants,
            } => {
                generic_parameters.remap_string_ids(remap);
                for variant in variants {
                    variant.remap_string_ids(remap);
                }
            }

            HeaderKind::TypeAlias { target } => {
                target.remap_string_ids(remap);
            }

            HeaderKind::ConstTemplate {
                condition_references,
                ..
            } => {
                for reference in condition_references {
                    reference.remap_string_ids(remap);
                }
            }

            HeaderKind::StartFunction => {}

            HeaderKind::Trait { declaration } => {
                declaration.remap_string_ids(remap);
            }

            HeaderKind::TraitConformance { conformance } => {
                conformance.remap_string_ids(remap);
            }

            HeaderKind::TraitIncompatibility { incompatibility } => {
                incompatibility.remap_string_ids(remap);
            }
        }
    }
}

impl Header {
    /// Remap every interned string owned by this header into the merged global string table.
    ///
    /// WHAT: remaps the kind payload, dependency paths, source locations, token stream,
    ///       and source file.
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    ///      table requires shifting every `StringId`, `InternedPath`, and `SourceLocation` so later
    ///      stages resolve names through the global table.
    /// NOTE: file imports are no longer stored on `Header`; they are remapped through
    ///       `FileFrontendPrepareOutput::remap_string_ids` instead.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.kind.remap_string_ids(remap);

        // Rebuild the hint set after remapping because InternedPath hash values
        // depend on their component StringIds, which change during remapping.
        let mut remapped_hints = HashSet::with_capacity(self.local_ordering_hints.len());
        for mut hint in self.local_ordering_hints.drain() {
            hint.remap_string_ids(remap);
            remapped_hints.insert(hint);
        }
        self.local_ordering_hints = remapped_hints;

        self.name_location.remap_string_ids(remap);
        self.tokens.remap_string_ids(remap);
        self.source_file.remap_string_ids(remap);
        for reference in &mut self.capacity_references {
            reference.remap_string_ids(remap);
        }
    }

    /// Returns the canonical (real OS) filesystem path for the source file that owns this header.
    /// Falls back to the logical source-file path when no OS path is recorded.
    ///
    /// WHY: const-template scopes use synthetic paths; the canonical path is needed for
    /// project-path-resolver lookups and rendered-path-usage tracking.
    pub(crate) fn canonical_source_file(&self, string_table: &mut StringTable) -> InternedPath {
        // Canonical filesystem paths are project-derived inputs that must be interned before
        // downstream stages can use them as InternedPath values.
        //
        // Stage 0 validates filesystem names as UTF-8 before they become canonical OS paths, so
        // a non-UTF-8 component here is a proven compiler invariant violation, not user input.
        // The expect documents that invariant rather than silently dropping the component.
        match self.tokens.canonical_os_path.as_ref() {
            Some(canonical_path) => InternedPath::try_from_filesystem_path(
                canonical_path,
                string_table,
            )
            .expect("canonical_os_path must be UTF-8; Stage 0 validates filesystem names before canonicalization"),
            None => self.source_file.to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileImport {
    /// Structural provider reference: the normalized import path and its exact source location.
    ///
    /// WHAT: carries the provider path Stage 0 resolves today plus the `path_location` retained
    /// for the graph boundary, type-distinct from the alias/export metadata below.
    /// WHY: structural provider references and imported-symbol bindings are separate data
    /// classes; embedding the shared `StructuralProviderReference` keeps one authority for the
    /// provider path and its location across Stage 0 scanning and retained import shells.
    pub provider: StructuralProviderReference,
    /// The exact authored structural path before module-root normalization.
    ///
    /// Stage 0 resolves topology and provider classes from this spelling so obsolete relative
    /// source imports and provider prefixes retain their authored diagnostics. Semantic binding
    /// continues to consume `provider`, whose path is normalized for module-local lookup.
    pub authored_provider: StructuralProviderReference,
    pub alias: Option<StringId>,
    /// Location of the `import` clause that introduced this record.
    pub location: SourceLocation,
    pub alias_location: Option<SourceLocation>,
    pub from_grouped: bool,
    /// Whether this import is part of the module public surface.
    ///
    /// WHAT: `Public` for grouped imports inside an `export:` block;
    /// `Private` for ordinary imports.
    pub export_mode: HeaderExportMode,
}

/// Classification of a source file's role within the current module compilation.
///
/// WHAT: distinguishes the active module root, an imported module root used as an export surface,
///       and ordinary source.
/// WHY: root identity and compilation context are independent: a root can expose declarations
///       when imported without contributing its own top-level runtime body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRole {
    /// A normal root file for the module currently being compiled.
    ActiveModuleRoot,
    /// A support or project-facade root currently being compiled as an API-only module.
    ActiveApiOnlyModuleRoot,
    /// A foreign root file compiled only to validate its public declaration surface.
    ImportedModuleRoot,
    /// An ordinary source file.
    Normal,
}

impl FileRole {
    pub(crate) fn is_module_root(self) -> bool {
        matches!(
            self,
            Self::ActiveModuleRoot | Self::ActiveApiOnlyModuleRoot | Self::ImportedModuleRoot
        )
    }

    /// Whether this root belongs to the module currently being compiled.
    pub(crate) fn is_active_module_root(self) -> bool {
        matches!(self, Self::ActiveModuleRoot | Self::ActiveApiOnlyModuleRoot)
    }

    pub(crate) fn is_export_capable(self) -> bool {
        self.is_module_root()
    }
}

/// Per-file output produced by header parsing before module-wide aggregation.
///
/// WHAT: carries all data produced from a single source file during header parsing so that
/// `prepare_header_syntax` can aggregate per-file outputs deterministically instead of relying on
/// shared mutable buffers during the file loop.
/// WHY: explicit per-file boundaries are required before tokenization/header parsing can run
/// in parallel; each file must be self-contained so later phases can merge/remap outputs.
pub struct FileFrontendPrepareOutput {
    pub source_file: InternedPath,
    /// Preserved for later parallel phases that need stable file identity before aggregation.
    #[allow(dead_code)]
    pub file_id: Option<FileId>,
    /// Number of tokens produced for this file before header parsing consumes the stream.
    ///
    /// WHY: benchmark instrumentation needs module-level token volume without retokenizing or
    /// walking source text after the Stage 2 preparation boundary.
    pub token_count: usize,
    /// Cheap token classification for this file.
    ///
    /// WHAT: counts gathered during the existing tokenization pass.
    /// WHY: per-file stats merge into the module-wide aggregate without a second traversal.
    pub token_stats: TokenStats,
    /// The role of this source file within the module.
    ///
    /// WHAT: distinguishes active roots, imported roots and normal source files.
    /// WHY: module-wide symbol collection needs file roles for every prepared file,
    /// including import-only files that may produce no headers.
    pub file_role: FileRole,
    /// Parsed imports for this source file.
    ///
    /// WHAT: file-level import records are stored once per file instead of duplicated onto
    /// every header from that file.
    /// WHY: import-only root files may produce no declaration headers but still contribute
    /// imports to the module symbol package.
    pub file_imports: Vec<FileImport>,
    /// Canonical OS filesystem path for this source file, if available.
    ///
    /// WHAT: the real filesystem path used by Stage 0 path resolution.
    /// WHY: import-only files and files without declaration headers still need path metadata
    /// for module membership and public export data registration.
    pub canonical_os_path: Option<std::path::PathBuf>,
    pub headers: Vec<Header>,
    pub top_level_const_fragments: Vec<TopLevelConstFragment>,
    /// Number of const templates parsed in this file.
    ///
    /// WHY: const-template synthetic names must remain unique across the module while per-file
    /// parsing reports its contribution separately from module aggregation.
    // Phase 6 parallel preparation keeps this contribution explicit for validation and future
    // fragment instrumentation, even though Alpha currently permits const templates only in the
    // single active module root.
    pub const_template_count: usize,
    /// Number of runtime fragments contributed by this file.
    pub runtime_fragment_count: usize,
    /// Whether this file is the active root and contains non-trivial top-level root/start code.
    pub has_non_trivial_root_body: bool,
    /// Warnings emitted while parsing this file.
    ///
    /// WHY: per-file preparation must be self-contained; warnings are merged into the caller's
    /// warning vector in deterministic file iteration order before module-wide aggregation.
    pub warnings: Vec<CompilerDiagnostic>,
}

/// Failed per-file header preparation plus warnings emitted before the failure.
///
/// WHY: warnings are produced while parsing declarations before a later token in the same file can
/// fail. The module parser must keep those warnings even when the file contributes no headers.
#[derive(Debug)]
pub struct FileFrontendPrepareError {
    pub warnings: Vec<CompilerDiagnostic>,
    pub diagnostic: Box<CompilerDiagnostic>,
}

impl FileFrontendPrepareOutput {
    /// Remap every interned string owned by this per-file output into the merged global string table.
    ///
    /// WHAT: remaps source file, file imports, headers, const fragments, and warnings.
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    ///      table requires shifting every `StringId`, `InternedPath`, and `SourceLocation` so later
    ///      stages resolve names through the global table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        // Per-file merges frequently append strings without changing numeric IDs; avoid walking
        // the full token/header/template payload when those IDs already target the merged table.
        if remap.is_identity() {
            return;
        }

        self.source_file.remap_string_ids(remap);

        for import in &mut self.file_imports {
            import.remap_string_ids(remap);
        }

        for header in &mut self.headers {
            header.remap_string_ids(remap);
        }

        for fragment in &mut self.top_level_const_fragments {
            fragment.remap_string_ids(remap);
        }

        for warning in &mut self.warnings {
            warning.remap_string_ids(remap);
        }
    }
}

impl FileFrontendPrepareError {
    /// Remap every interned string owned by this failed per-file output into the merged global
    /// string table.
    ///
    /// WHAT: remaps warnings and the primary diagnostic.
    /// WHY: per-file frontend preparation uses local string tables; even failed files may have
    ///      emitted warnings before the error, and those strings must resolve through the global table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        // Keep failed-file diagnostics on the same identity fast path as successful outputs.
        if remap.is_identity() {
            return;
        }

        for warning in &mut self.warnings {
            warning.remap_string_ids(remap);
        }

        self.diagnostic.remap_string_ids(remap);
    }
}

// Shared file-level state that stays live while one source file is being split into headers.
pub(super) struct HeaderParseContext<'a> {
    pub file_role: FileRole,
    pub is_config_file: bool,
    pub string_table: &'a mut StringTable,
    /// Module-wide base offset for const-template synthetic names in this file.
    ///
    /// WHY: const-template names must be unique across the module; each file's parser
    /// starts numbering from this offset so later aggregation does not need to renumber.
    pub const_template_offset: usize,
    /// Entry-file base offset for runtime-fragment insertion indices in this file.
    ///
    /// WHY: only active module roots produce runtime fragments, but passing the offset keeps
    /// per-file preparation deterministic even if the caller changes ordering later.
    pub runtime_fragment_offset: usize,
}

// Shared per-header builder inputs that stay stable while one declaration is classified.
pub(super) struct HeaderBuildContext<'a> {
    pub warnings: &'a mut Vec<CompilerDiagnostic>,
    pub source_file: &'a InternedPath,
    pub file_imports: &'a HashSet<InternedPath>,
    pub file_import_entries: &'a [FileImport],
    pub string_table: &'a mut StringTable,
    pub file_role: FileRole,
}

#[cfg(test)]
#[path = "tests/header_remap_tests.rs"]
mod header_remap_tests;
