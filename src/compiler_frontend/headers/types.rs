//! Header-stage data contracts.
//!
//! WHAT: shared structs/enums produced by header parsing and consumed by dependency sorting,
//! AST construction, and module symbol collection.
//! WHY: keeping these types separate from parser control flow makes the header-stage API obvious
//! and avoids making `parse_file_headers.rs` the dumping ground for every header concern.

use crate::compiler_frontend::arena::{HeaderStats, TokenStats};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DependencyClauseKind, PremergeDiagnosticBatch,
};
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::declaration_syntax::build_config_contract::SourceBuildConfigContract;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantSyntax;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionSignatureSyntax, SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::dependency_clause_syntax::{
    DependencyAlias, RetainedDependencyPath,
};
use crate::compiler_frontend::headers::dependency_target::decode_dependency_target;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identity::DependencySelectionId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::lexer::TokenizeFailure;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token};
use crate::compiler_frontend::traits::syntax::{
    TraitConformanceSyntax, TraitDeclarationSyntax, TraitIncompatibilitySyntax,
};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use std::collections::HashSet;
use std::fmt::Display;
use std::sync::Arc;

/// Provider-independent retained header syntax produced before provider interfaces exist.
///
/// WHAT: aggregates per-file declaration shells, dependency shells, order-independent local/module
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
    /// Provider-independent source `#Config` declarations, normalized from top-level constant
    /// shells before any provider binding or AST expression resolution.
    ///
    /// These shells intentionally do not belong to `ModuleSymbols`, public exports, dependency
    /// clauses/edges, or local ordering hints. Their authored constant shells remain in `headers`
    /// so the later config barrier can consume one retained source identity.
    pub source_build_config_contracts: Vec<SourceBuildConfigContract>,
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
/// WHAT: owns the completed public-surface/header binding environment and dependency facts required by
/// dependency sorting and AST. Produced only by `bind_module_headers`, which resolves retained
/// dependency shells against immutable provider interfaces, canonicalizes dependency edges, and
/// completes constant initializer dependencies.
/// WHY: binding does not retokenize source or reparse declaration syntax — it consumes the
/// retained `PreparedHeaderSyntax` and adds the provider-dependent facts that cannot be known
/// before the provider graph has compiled.
pub struct BoundModuleHeaders {
    pub headers: Vec<Header>,
    /// Provider-independent source `#Config` declarations retained through binding for the
    /// module-local static-value projection in AST construction.
    pub source_build_config_contracts: Vec<SourceBuildConfigContract>,
    pub top_level_const_fragments: Vec<TopLevelConstFragment>,
    pub entry_runtime_fragment_count: usize,
    pub const_fragment_count: usize,
    pub has_non_trivial_root_body: bool,
    pub token_stats: TokenStats,
    pub header_stats: HeaderStats,
    pub module_symbols: ModuleSymbols,
    /// Header-built per-file binding visibility environment.
    ///
    /// WHY: dependency binding and visibility construction is owned by the header binding phase;
    /// AST consumes this directly without rebuilding bindings or rediscovering visibility.
    pub binding_environment: HeaderBindingEnvironment,
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
    pub header_path: PathId,
    /// Exact authored source span of this const fragment.
    pub span: SourceSpan,
}

/// Optional settings that affect module header parsing.
///
/// WHAT: bundles optional entry identity and a borrowed path resolver for one parse invocation.
/// WHY: the parser is called from both production and tests, and grouping these keeps the API concise.
///      The resolver is build-lifetime immutable data, so options borrow it instead of cloning it.
#[derive(Clone, Copy)]
pub struct HeaderParseOptions<'a> {
    pub entry_file_id: Option<SourceId>,
    pub project_path_resolver: Option<&'a ProjectPathResolver>,
    /// An explicit role for the active entry file, when the caller is compiling a transient
    /// selection rather than a graph-owned module root.
    ///
    /// WHY: check-only source selections retain their real source semantics and must not acquire
    ///      active-root privileges merely because their selected file is the entry path.
    pub entry_file_role: Option<FileRole>,
    /// The graph-owned semantic role of the root currently being compiled.
    ///
    /// WHY: entry-path equality identifies which file is active but cannot decide whether that
    ///      root owns dormant runtime work. Support and project-facade roots are API-only even
    ///      while they are the active compilation root.
    pub active_root_role: ModuleRootRole,
}

impl Default for HeaderParseOptions<'_> {
    fn default() -> Self {
        Self {
            entry_file_id: None,
            project_path_resolver: None,
            entry_file_role: None,
            active_root_role: ModuleRootRole::Normal,
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "keeping declaration shells inline avoids an allocation for every constant header"
)]
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
    /// Start functions are build-system-only; they are not dependency-bindable or callable from modules.
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

/// Explicit export mode for a parsed header or file dependency.
///
/// WHAT: distinguishes private source-file items from public module-root API surface.
/// WHY: module-root files use one explicit `export:` block to mark public declarations and
/// direct-selection re-exports. All other files keep every declaration as `Private`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderExportMode {
    /// Private to the source file or depending file.
    Private,
    /// Public module API entry exposed through a module-root file.
    Public,
}

impl HeaderExportMode {
    pub fn is_public(&self) -> bool {
        matches!(self, HeaderExportMode::Public)
    }
}

/// Provenance for one conservative declaration-ordering hint.
///
/// WHAT: distinguishes a same-file path from a provider dependency spelling, a qualified type
/// namespace spelling, and another file's generated content constant.
/// WHY: final source rebinding must reject missing prefixes for same-file paths while preserving
/// the other spellings exactly for their distinct binding-time canonicalization paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocalDeclarationOrderingHintOrigin {
    SourceOwned,
    ProviderSpelling,
    /// A namespace-qualified type spelling resolved directly through bound namespace visibility.
    QualifiedTypeSpelling,
    /// A content source's synthetic `content` constant.
    ///
    /// WHAT: the path targets `<content source>/content` of another prepared file, recorded from a
    ///       content-class file-value path in a declaration shell.
    /// WHY: the target is not prefixed by the referencing file, so source-identity rebinding must
    ///       leave it unchanged and canonicalization must not classify it through dependency
    ///       clauses, which never spell content paths.
    ContentSource,
}

/// A conservative declaration-shell ordering fact retained before provider binding.
///
/// WHAT: one referenced path captured from a declaration shell's type surface, constant
/// initializer, or shell value expression, recorded in the spelling seen during syntax
/// preparation. It is not an already-proven graph edge.
/// WHY: Stage 2 retains these hints without knowing which providers are source graph participants
/// versus virtual or provider bindings. Stage 3 alone resolves retained local hints into
/// sortable graph edges after binding has canonicalized or dropped provider-spelled hints.
/// MUST NOT: carry alias, export, or provider classification; that metadata stays on
/// `RetainedDependencyClause` and `RetainedDependencyPath`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalDeclarationOrderingHint {
    /// The conservative referenced path: a provider spelling, a same-file spelling, or a content
    /// source's `content` constant path.
    path: PathId,
    origin: LocalDeclarationOrderingHintOrigin,
    /// The authored occurrence's path-row handle for content-source hints.
    occurrence: Option<PathSyntaxId>,
    /// The exact authored span for a content-source occurrence.
    occurrence_span: Option<SourceSpan>,
}

impl LocalDeclarationOrderingHint {
    /// Record a same-file path that must move with final source identity.
    pub fn source_owned(path: PathId) -> Self {
        Self {
            path,
            origin: LocalDeclarationOrderingHintOrigin::SourceOwned,
            occurrence: None,
            occurrence_span: None,
        }
    }

    /// Record a provider spelling that deliberately has no dependency on this source prefix.
    pub fn provider_spelling(path: PathId) -> Self {
        Self {
            path,
            origin: LocalDeclarationOrderingHintOrigin::ProviderSpelling,
            occurrence: None,
            occurrence_span: None,
        }
    }

    /// Record a namespace-qualified type spelling for direct visibility resolution.
    pub fn qualified_type_spelling(path: PathId) -> Self {
        Self {
            path,
            origin: LocalDeclarationOrderingHintOrigin::QualifiedTypeSpelling,
            occurrence: None,
            occurrence_span: None,
        }
    }

    /// Record an ordering fact against a content source's synthetic `content` constant.
    pub fn content_source(path: PathId, occurrence: PathSyntaxId) -> Self {
        Self {
            path,
            origin: LocalDeclarationOrderingHintOrigin::ContentSource,
            occurrence: Some(occurrence),
            occurrence_span: None,
        }
    }

    /// Attach the exact authored span for a content-source occurrence.
    pub fn with_occurrence_span(mut self, span: SourceSpan) -> Self {
        self.occurrence_span = Some(span);
        self
    }

    /// The authored occurrence handle for content-source hints; absent for other origins.
    pub fn occurrence(&self) -> Option<PathSyntaxId> {
        self.occurrence
    }

    /// The exact authored span for a content-source occurrence.
    pub fn occurrence_span(&self) -> Option<SourceSpan> {
        self.occurrence_span
    }

    /// The conservative referenced path this hint records.
    pub fn path(&self) -> PathId {
        self.path
    }

    pub fn origin(&self) -> LocalDeclarationOrderingHintOrigin {
        self.origin
    }

    pub fn remap_string_ids(&mut self, _remap: &StringIdRemap) {}

    /// Remap the complete path identity after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.path = remap.get(self.path);
    }

    fn validate_required_source_prefix(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        if self.origin == LocalDeclarationOrderingHintOrigin::SourceOwned
            && !path_fork.starts_with(self.path, provisional_source_file)
        {
            return Err(CompilerError::compiler_error(
                "source-owned retained path is missing its provisional source prefix",
            ));
        }
        Ok(())
    }

    fn rebind_source_identity(
        self,
        final_file_id: SourceId,
        provisional_source_file: PathId,
        logical_path: PathId,
        path_fork: &mut PathInternerFork,
    ) -> Result<Self, CompilerError> {
        let path = match self.origin {
            LocalDeclarationOrderingHintOrigin::SourceOwned => rebind_required_path(
                self.path,
                provisional_source_file,
                logical_path,
                path_fork,
            )?,
            LocalDeclarationOrderingHintOrigin::ProviderSpelling
            | LocalDeclarationOrderingHintOrigin::QualifiedTypeSpelling
            | LocalDeclarationOrderingHintOrigin::ContentSource => self.path,
        };
        Ok(Self {
            path,
            origin: self.origin,
            occurrence: self.occurrence,
            occurrence_span: self
                .occurrence_span
                .map(|span| SourceSpan::new(final_file_id, span.local())),
        })
    }
}

fn rebind_required_path(
    path: PathId,
    old_prefix: PathId,
    new_prefix: PathId,
    path_fork: &mut PathInternerFork,
) -> Result<PathId, CompilerError> {
    if !path_fork.starts_with(path, old_prefix) {
        return Err(CompilerError::compiler_error(
            "source-owned retained path is missing its provisional source prefix",
        ));
    }
    let mut scratch = Vec::new();
    path_fork.resolve_components(path, &mut scratch);
    let old_depth = path_fork.depth(old_prefix) as usize;
    let mut rebound = new_prefix;
    for component in scratch.iter().skip(old_depth) {
        rebound = path_fork.try_intern_child(rebound, *component).ok_or_else(|| {
            CompilerError::compiler_error("path table exhausted while rebinding source-owned path")
        })?;
    }
    Ok(rebound)
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
    /// WHY: dependency preparation builds module APIs from explicit public-surface metadata, not from
    /// file role alone.
    pub export_mode: HeaderExportMode,
    /// Conservative local declaration-ordering hints retained before provider binding.
    ///
    /// WHAT: referenced paths from this declaration shell's type surface, constant initializer,
    /// and shell value expressions, recorded in the provider, same-file, or content-constant
    /// spelling seen during syntax preparation. These are ordering hints, not already-proven
    /// graph edges.
    /// WHY: binding canonicalizes or drops dependency-spelled hints using bound visibility, then
    /// Stage 3 resolves retained local hints into sortable graph edges.
    pub local_ordering_hints: HashSet<LocalDeclarationOrderingHint>,
    /// Exact authored declaration-name span; synthetic headers use `None`.
    pub name_span: Option<SourceSpan>,

    // Token Body (for functions / templates) and info about canonical_os_path
    pub tokens: FileTokens,

    pub source_file: PathId,
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
    /// Remap the complete path identity after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.header_path = remap.get(self.header_path);
    }

    pub fn remap_string_ids(&mut self, _remap: &StringIdRemap) {}

    fn validate_required_source_prefix(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        if !path_fork.starts_with(self.header_path, provisional_source_file) {
            return Err(CompilerError::compiler_error(
                "source-owned retained path is missing its provisional source prefix",
            ));
        }
        Ok(())
    }

    pub fn rebind_source_identity(
        &mut self,
        final_file_id: SourceId,
        provisional_source_file: PathId,
        logical_path: PathId,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), CompilerError> {
        self.header_path = rebind_required_path(
            self.header_path,
            provisional_source_file,
            logical_path,
            path_fork,
        )?;
        self.span = SourceSpan::new(final_file_id, self.span.local());
        Ok(())
    }
}

impl RetainedDependencyClause {
    /// Remap every interned string owned by this dependency clause into the merged global string table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.dependency.remap_string_ids(remap);
        self.binding.remap_string_ids(remap);
    }

    /// Remap the retained dependency complete-path identity after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.dependency.remap_path_ids(remap);
    }

    /// Commit the source identity after `FileFrontendPrepareOutput` has preflighted every
    /// required source-owned retained path.
    pub fn commit_source_rebinding(&mut self, file_id: SourceId, logical_path: PathId) {
        self.dependency
            .commit_source_rebinding(file_id, logical_path);
        self.binding.rebind_source_identity(file_id);
    }
}

impl HeaderKind {
    /// Whether this header kind is an authored declaration that may participate in a
    /// module-root public export surface.
    ///
    /// WHAT: returns `true` exactly for authored declaration kinds that are exportable after
    /// syntax-only filtering: functions, structs, choices, transparent type aliases, ordinary
    /// compile-time constants and trait declarations. Source `#Config` constants are contract
    /// shells rather than exported declarations and therefore return `false`.
    /// WHY: the header, AST public-surface and semantic-origin stages all need the same
    /// declaration-kind and contract-shell gate to decide which headers may become public export
    /// entries. Owning both gates here keeps the stage-local predicates from drifting.
    pub fn is_authored_public_export_declaration(&self) -> bool {
        match self {
            HeaderKind::Function { .. }
            | HeaderKind::Struct { .. }
            | HeaderKind::Choice { .. }
            | HeaderKind::TypeAlias { .. }
            | HeaderKind::Trait { .. } => true,
            HeaderKind::Constant { declaration } => declaration.config_qualifier.is_none(),
            HeaderKind::StartFunction
            | HeaderKind::ConstTemplate { .. }
            | HeaderKind::TraitConformance { .. }
            | HeaderKind::TraitIncompatibility { .. } => false,
        }
    }

    /// Remap every interned string owned by this header kind into the merged global string table.
    ///
    /// WHAT: dispatches to nested remap methods for function signatures, declaration shells,
    ///       struct fields, choice variants, and type-alias targets.
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    ///      table requires shifting every `StringId` and `InternedPath`.
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

    /// Remap nested declaration member path identities after the path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        match self {
            HeaderKind::Function { signature, .. } => signature.remap_path_ids(remap),
            HeaderKind::Struct { fields, .. } => {
                for field in fields {
                    field.remap_path_ids(remap);
                }
            }
            HeaderKind::Choice { variants, .. } => {
                for variant in variants {
                    variant.remap_path_ids(remap);
                }
            }
            HeaderKind::Trait { declaration } => declaration.remap_path_ids(remap),
            HeaderKind::Constant { .. }
            | HeaderKind::TypeAlias { .. }
            | HeaderKind::ConstTemplate { .. }
            | HeaderKind::StartFunction
            | HeaderKind::TraitConformance { .. }
            | HeaderKind::TraitIncompatibility { .. } => {}
        }
    }

    fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        match self {
            HeaderKind::Function { signature, .. } => {
                signature.validate_required_source_prefixes(provisional_source_file, path_fork)
            }
            HeaderKind::Struct { fields, .. } => {
                for field in fields {
                    field.validate_required_source_prefix(provisional_source_file, path_fork)?;
                }
                Ok(())
            }
            HeaderKind::Choice { variants, .. } => {
                for variant in variants {
                    variant.validate_required_source_prefixes(provisional_source_file, path_fork)?;
                }
                Ok(())
            }
            HeaderKind::Trait { declaration } => {
                declaration.validate_required_source_prefixes(provisional_source_file, path_fork)
            }
            HeaderKind::Constant { .. }
            | HeaderKind::TypeAlias { .. }
            | HeaderKind::ConstTemplate { .. }
            | HeaderKind::StartFunction
            | HeaderKind::TraitConformance { .. }
            | HeaderKind::TraitIncompatibility { .. } => Ok(()),
        }
    }

    pub fn rebind_source_identity(
        &mut self,
        file_id: SourceId,
        logical_path: PathId,
        provisional_source_file: PathId,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), CompilerError> {
        match self {
            HeaderKind::Function {
                generic_parameters,
                signature,
            } => {
                generic_parameters.rebind_source_identity(file_id);
                rebind_function_signature_source_identity(
                    signature,
                    file_id,
                    provisional_source_file,
                    logical_path,
                    path_fork,
                )?;
            }

            HeaderKind::Constant { declaration, .. } => {
                rebind_declaration_source_identity(declaration, file_id)?;
            }

            HeaderKind::Struct {
                generic_parameters,
                fields,
            } => {
                generic_parameters.rebind_source_identity(file_id);
                for field in fields {
                    rebind_signature_member_source_identity(
                        field,
                        file_id,
                        provisional_source_file,
                        logical_path,
                        path_fork,
                    )?;
                }
            }

            HeaderKind::Choice {
                generic_parameters,
                variants,
            } => {
                generic_parameters.rebind_source_identity(file_id);
                for variant in variants {
                    rebind_choice_variant_source_identity(
                        variant,
                        file_id,
                        provisional_source_file,
                        logical_path,
                        path_fork,
                    )?;
                }
            }

            HeaderKind::TypeAlias { target } => {
                rebind_parsed_type_ref_source_identity(target, file_id);
            }

            // Initializer references retain the exact source-qualified span captured by their
            // owning token stream. They may point at a donor source, so finalising this header
            // must not reconstruct their span from the header's file identity.
            HeaderKind::ConstTemplate { .. } | HeaderKind::StartFunction => {}

            HeaderKind::Trait { declaration } => {
                rebind_trait_declaration_source_identity(
                    declaration,
                    file_id,
                    provisional_source_file,
                    logical_path,
                    path_fork,
                )?;
            }

            HeaderKind::TraitConformance { conformance } => {
                conformance.target.span = SourceSpan::new(file_id, conformance.target.span.local());
                for trait_ref in &mut conformance.traits {
                    trait_ref.span = SourceSpan::new(file_id, trait_ref.span.local());
                }
            }

            HeaderKind::TraitIncompatibility { incompatibility } => {
                incompatibility.subject.span =
                    SourceSpan::new(file_id, incompatibility.subject.span.local());
                for trait_ref in &mut incompatibility.incompatible_traits {
                    trait_ref.span = SourceSpan::new(file_id, trait_ref.span.local());
                }
            }
        }
        Ok(())
    }
}

fn rebind_source_span(span: &mut Option<SourceSpan>, file_id: SourceId) {
    if let Some(span) = span {
        *span = SourceSpan::new(file_id, span.local());
    }
}

fn rebind_parsed_type_ref_source_identity(type_ref: &mut ParsedTypeRef, file_id: SourceId) {
    match type_ref {
        ParsedTypeRef::Inferred => {}

        ParsedTypeRef::Named { span, .. }
        | ParsedTypeRef::Qualified { span, .. }
        | ParsedTypeRef::BuiltinBool { span }
        | ParsedTypeRef::BuiltinInt { span }
        | ParsedTypeRef::BuiltinFloat { span }
        | ParsedTypeRef::BuiltinString { span }
        | ParsedTypeRef::BuiltinChar { span }
        | ParsedTypeRef::This { span } => rebind_source_span(span, file_id),

        ParsedTypeRef::Applied {
            base,
            arguments,
            span,
        } => {
            rebind_source_span(span, file_id);
            rebind_parsed_type_ref_source_identity(base, file_id);
            for argument in arguments {
                rebind_parsed_type_ref_source_identity(argument, file_id);
            }
        }

        ParsedTypeRef::Collection {
            element,
            span,
            fixed_capacity,
        } => {
            rebind_source_span(span, file_id);
            rebind_parsed_type_ref_source_identity(element, file_id);
            if let Some(capacity) = fixed_capacity {
                match capacity {
                    ParsedCollectionCapacity::Literal { span, .. }
                    | ParsedCollectionCapacity::BareConstant { span, .. } => {
                        rebind_source_span(span, file_id);
                    }
                }
            }
        }

        ParsedTypeRef::Map {
            key, value, span, ..
        } => {
            rebind_source_span(span, file_id);
            rebind_parsed_type_ref_source_identity(key, file_id);
            rebind_parsed_type_ref_source_identity(value, file_id);
        }

        ParsedTypeRef::Optional { inner, span, .. } => {
            rebind_source_span(span, file_id);
            rebind_parsed_type_ref_source_identity(inner, file_id);
        }
    }
}

fn rebind_signature_member_source_identity(
    member: &mut SignatureMemberSyntax,
    file_id: SourceId,
    provisional_source_file: PathId,
    logical_path: PathId,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    member.id = rebind_required_path(
        member.id,
        provisional_source_file,
        logical_path,
        path_fork,
    )?;
    rebind_source_span(&mut member.span, file_id);
    rebind_parsed_type_ref_source_identity(&mut member.type_annotation, file_id);
    Ok(())
}

fn rebind_function_signature_source_identity(
    signature: &mut FunctionSignatureSyntax,
    file_id: SourceId,
    provisional_source_file: PathId,
    logical_path: PathId,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    for parameter in &mut signature.parameters {
        rebind_signature_member_source_identity(
            parameter,
            file_id,
            provisional_source_file,
            logical_path,
            path_fork,
        )?;
    }
    for return_slot in &mut signature.returns {
        rebind_source_span(&mut return_slot.value.span, file_id);
        rebind_parsed_type_ref_source_identity(&mut return_slot.value.type_annotation, file_id);
    }
    Ok(())
}
fn rebind_trait_declaration_source_identity(
    declaration: &mut TraitDeclarationSyntax,
    file_id: SourceId,
    provisional_source_file: PathId,
    logical_path: PathId,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    declaration.name_span = SourceSpan::new(file_id, declaration.name_span.local());
    declaration.span = SourceSpan::new(file_id, declaration.span.local());
    for requirement in &mut declaration.requirements {
        requirement.name_span = SourceSpan::new(file_id, requirement.name_span.local());
        requirement.span = SourceSpan::new(file_id, requirement.span.local());
        rebind_function_signature_source_identity(
            &mut requirement.signature,
            file_id,
            provisional_source_file,
            logical_path,
            path_fork,
        )?;
    }
    Ok(())
}

fn rebind_choice_variant_source_identity(
    variant: &mut ChoiceVariantSyntax,
    file_id: SourceId,
    provisional_source_file: PathId,
    logical_path: PathId,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    rebind_source_span(&mut variant.span, file_id);
    if let crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Record {
        fields,
    } = &mut variant.payload
    {
        for field in fields {
            rebind_signature_member_source_identity(
                field,
                file_id,
                provisional_source_file,
                logical_path,
                path_fork,
            )?;
        }
    }
    Ok(())
}

fn rebind_declaration_source_identity(
    declaration: &mut DeclarationSyntax,
    file_id: SourceId,
) -> Result<(), CompilerError> {
    rebind_source_span(&mut declaration.span, file_id);
    rebind_parsed_type_ref_source_identity(&mut declaration.type_annotation, file_id);
    // Initializer references retain their captured source-qualified spans. In particular, a
    // generated or materialised initializer may own a different source than its containing header.
    Ok(())
}

impl Header {
    /// Remap every interned string owned by this header into the merged global string table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.kind.remap_string_ids(remap);
        let hints = std::mem::take(&mut self.local_ordering_hints);
        self.local_ordering_hints = hints
            .into_iter()
            .map(|mut hint| {
                hint.remap_string_ids(remap);
                hint
            })
            .collect();
        self.tokens.remap_string_ids(remap);
        for reference in &mut self.capacity_references {
            reference.remap_string_ids(remap);
        }
    }

    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.kind.remap_path_ids(remap);
        let hints = std::mem::take(&mut self.local_ordering_hints);
        self.local_ordering_hints = hints
            .into_iter()
            .map(|mut hint| {
                hint.remap_path_ids(remap);
                hint
            })
            .collect();
        self.tokens.remap_path_ids(remap);
        self.source_file = remap.get(self.source_file);
    }

    fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        if !path_fork.starts_with(self.tokens.src_path, provisional_source_file) {
            return Err(CompilerError::compiler_error(
                "source-owned retained path is missing its provisional source prefix",
            ));
        }
        for hint in &self.local_ordering_hints {
            hint.validate_required_source_prefix(provisional_source_file, path_fork)?;
        }
        self.kind
            .validate_required_source_prefixes(provisional_source_file, path_fork)
    }

    pub fn rebind_source_identity(
        &mut self,
        file_id: SourceId,
        logical_path: PathId,
        canonical_os_path: std::path::PathBuf,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), CompilerError> {
        let provisional_source_file = self.source_file;
        self.validate_required_source_prefixes(provisional_source_file, path_fork)?;
        self.kind.rebind_source_identity(
            file_id,
            logical_path,
            provisional_source_file,
            path_fork,
        )?;

        let mut rebound_hints = HashSet::with_capacity(self.local_ordering_hints.len());
        for hint in self.local_ordering_hints.drain() {
            rebound_hints.insert(hint.rebind_source_identity(
                file_id,
                provisional_source_file,
                logical_path,
                path_fork,
            )?);
        }
        self.local_ordering_hints = rebound_hints;

        if let Some(name_span) = self.name_span {
            self.name_span = Some(SourceSpan::new(file_id, name_span.local()));
        }
        let rebound_header_path = rebind_required_path(
            self.tokens.src_path,
            provisional_source_file,
            logical_path,
            path_fork,
        )?;
        self.tokens
            .rebind_file_identity(logical_path, file_id, Some(canonical_os_path));
        self.tokens.src_path = rebound_header_path;
        self.source_file = logical_path;
        Ok(())
    }
}

/// One contiguous range in a file's dependency-selection store.
///
/// WHAT: keeps each authored clause's selected names as a view into one file-owned flat table.
/// WHY: a selected name is a clause fact, not a provider row. Contiguous ranges preserve authored
/// order without allocating a separate selection vector for every clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DependencySelectionRange {
    pub start: u32,
    pub end: u32,
}

impl DependencySelectionRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn get(
        self,
        selections: &[DependencySelection],
    ) -> Result<&[DependencySelection], CompilerError> {
        let start = usize::try_from(self.start).map_err(|_| {
            CompilerError::compiler_error("dependency selection range start does not fit usize")
        })?;
        let end = usize::try_from(self.end).map_err(|_| {
            CompilerError::compiler_error("dependency selection range end does not fit usize")
        })?;
        if start > end || end > selections.len() {
            return Err(CompilerError::compiler_error(format!(
                "dependency selection range {start}..{end} is outside a table of length {}",
                selections.len()
            )));
        }
        Ok(&selections[start..end])
    }
}

/// Mutually exclusive binding modes retained for one dependency clause.
///
/// WHAT: represents either one namespace alias or one direct-selection range.
/// WHY: the parser and header consumers must not be able to represent a clause that combines a
///      namespace alias with direct selections or carries an alias without its source span.
#[derive(Clone, Debug)]
pub enum DependencyBindingSyntax {
    Namespace { alias: Option<DependencyAlias> },
    DirectSelections { range: DependencySelectionRange },
}

impl DependencyBindingSyntax {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if let Self::Namespace { alias: Some(alias) } = self {
            alias.remap_string_ids(remap);
        }
    }

    pub fn rebind_source_identity(&mut self, file_id: SourceId) {
        if let Self::Namespace { alias: Some(alias) } = self {
            alias.span = SourceSpan::new(file_id, alias.span.local());
        }
    }

    pub fn selection_range(&self) -> Option<DependencySelectionRange> {
        match self {
            Self::Namespace { .. } => None,
            Self::DirectSelections { range } => Some(*range),
        }
    }

    /// Return the typed clause kind represented by this binding syntax.
    pub fn clause_kind(&self) -> DependencyClauseKind {
        match self {
            Self::Namespace { alias: None } => DependencyClauseKind::Namespace,
            Self::Namespace { alias: Some(_) } => DependencyClauseKind::NamespaceAlias,
            Self::DirectSelections { .. } => DependencyClauseKind::DirectSelection,
        }
    }
}

/// One direct name selected from a dependency clause's provider surface.
///
/// WHAT: retains the source name, optional local alias and both source locations without copying
///       the provider root or creating a provider identity per selected name.
/// WHY: one authored clause owns one dependency shell. Selection identity is only needed when a
///      later public-interface projection refers back to one selected binding.
#[derive(Clone, Debug)]
pub struct DependencySelection {
    pub source_name: StringId,
    pub source_span: SourceSpan,
    pub local_alias: Option<DependencyAlias>,
}

impl DependencySelection {
    pub fn local_name(&self) -> StringId {
        self.local_alias
            .as_ref()
            .map_or(self.source_name, |alias| alias.name)
    }

    pub fn local_alias(&self) -> Option<&DependencyAlias> {
        self.local_alias.as_ref()
    }

    pub fn rebind_source_identity(&mut self, file_id: SourceId) {
        self.source_span = SourceSpan::new(file_id, self.source_span.local());
        if let Some(alias) = &mut self.local_alias {
            alias.span = SourceSpan::new(file_id, alias.span.local());
        }
    }
}

#[derive(Clone, Debug)]
pub struct RetainedDependencyClause {
    /// The consolidated path authority: one shell, one structural path and one path location.
    ///
    /// WHAT: Stage 0 and later binding both consume this retained path. Diagnostics continue
    ///       to use the path location and original source text for authored spelling.
    /// WHY: authored and normalized spellings were identical, so a second path field would
    ///      only recreate a redundant invariant.
    pub dependency: RetainedDependencyPath,
    /// The mutually exclusive namespace/direct-selection binding owned by this clause.
    pub binding: DependencyBindingSyntax,
    /// Whether this dependency clause is part of the module public surface.
    ///
    /// WHAT: `Public` for direct selections inside an `export:` block;
    /// `Private` for ordinary dependency clauses.
    pub export_mode: HeaderExportMode,
}

impl RetainedDependencyClause {
    /// Derive the effective local name for a bare namespace clause.
    ///
    /// WHAT: preserves an explicit namespace alias, otherwise strips the registered JavaScript
    /// provider suffix from the provider basename before interning the binding name.
    /// WHY: parser collision checks, ordering-hint lookup and namespace registration must all
    /// compare the same file-local name. Direct-selection clauses do not define a namespace name.
    pub(crate) fn effective_namespace_local_name(
        &self,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> Option<StringId> {
        let DependencyBindingSyntax::Namespace { alias } = &self.binding else {
            return None;
        };

        if let Some(alias) = alias {
            return Some(alias.name);
        }

        let provider_name = path_fork.component(self.dependency.path)?;
        let provider_name = string_table.resolve(provider_name).to_owned();
        let stem = provider_name.strip_suffix(".js").unwrap_or(&provider_name);
        (!stem.is_empty()).then(|| string_table.intern(stem))
    }

    /// Resolve this clause's flat selection range against its file-owned selection table.
    pub fn selections<'a>(
        &self,
        selection_table: &'a [DependencySelection],
    ) -> Result<&'a [DependencySelection], CompilerError> {
        let Some(range) = self.binding.selection_range() else {
            return Ok(&[]);
        };
        if range.is_empty() {
            return Err(CompilerError::compiler_error(
                "direct-selection binding has an empty selection range",
            ));
        }
        range.get(selection_table)
    }

    /// Stable identity of one selected name within this authored clause.
    pub fn selection_id(
        &self,
        selection_table: &[DependencySelection],
        selected_index: usize,
    ) -> Result<DependencySelectionId, CompilerError> {
        let selection_count = self
            .binding
            .selection_range()
            .map_or(0, |range| range.len());
        let selections = self.selections(selection_table)?;
        if selected_index >= selection_count || selected_index >= selections.len() {
            return Err(CompilerError::compiler_error(format!(
                "dependency selection index {selected_index} is outside clause range of length {}",
                selection_count
            )));
        }
        Ok(DependencySelectionId::new(
            self.dependency.dependency_shell_id,
            selected_index as u32,
        ))
    }

    pub(crate) fn namespace_binding_span(&self) -> Option<&SourceSpan> {
        match &self.binding {
            DependencyBindingSyntax::Namespace { alias: Some(alias) } => Some(&alias.span),
            DependencyBindingSyntax::Namespace { alias: None } => Some(&self.dependency.span),
            DependencyBindingSyntax::DirectSelections { .. } => None,
        }
    }
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
    /// Complete logical identity of the prepared source file in the active path table.
    ///
    /// WHAT: the `PathId` interned for this file through the owning `PathInternerFork`,
    ///      shared with its `FileTokens.src_path` and authored path rows.
    /// WHY: tokenizer and dependency owners share one compact path domain; the
    ///      filesystem `canonical_os_path` remains the only `PathBuf` identity for IO.
    pub source_file: PathId,
    /// Stable source identity used by the prepared-file invariant gate to validate every header
    /// stream and retained dependency shell before module aggregation.
    pub file_id: SourceId,
    /// The sole file-owned table while source preparation remains mutable, then the immutable
    /// table shared by every retained header stream from this file.
    pub(crate) path_syntax: PreparedFilePathSyntax,
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
    /// including dependency-only files that may produce no headers.
    pub file_role: FileRole,
    /// Prepared dependency clauses for this source file.
    ///
    /// WHAT: file-level dependency clauses are stored once per file instead of duplicated onto
    /// every header from that file.
    /// WHY: dependency-only root files may produce no declaration headers but still contribute
    /// clauses to the module symbol package.
    pub file_dependency_clauses: Vec<RetainedDependencyClause>,
    /// Graph-active file-value paths authored by this file, excluding dependency-clause rows.
    pub structural_file_references: PreparedFileReferenceTable,
    /// One flat selection store for all dependency clauses authored by this file.
    pub dependency_selections: Vec<DependencySelection>,
    /// Canonical OS filesystem path for this source file, if available.
    ///
    /// WHAT: the real filesystem path used by Stage 0 path resolution.
    /// WHY: dependency-only files and files without declaration headers still need path metadata
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

/// Lifecycle owner for one prepared source file's path syntax.
///
/// WHAT: keeps exactly one mutable owner through string remapping and final identity rebinding,
///       then records the immutable table shared by all retained header streams.
/// WHY: ordinary header/default/start substreams must never copy rows or trigger `Arc`
///      copy-on-write while the source table is still being finalised.
#[derive(Debug)]
pub(crate) enum PreparedFilePathSyntax {
    Preparing(Arc<PathSyntaxTable>),
    Frozen(Arc<PathSyntaxTable>),
}

impl PreparedFilePathSyntax {
    pub(crate) fn from_file_tokens(tokens: &mut FileTokens) -> Result<Self, CompilerError> {
        Ok(Self::Preparing(tokens.take_preparing_path_syntax()?))
    }

    pub(crate) fn empty() -> Self {
        Self::Preparing(Arc::new(PathSyntaxTable::new()))
    }

    pub(crate) fn table(&self) -> &PathSyntaxTable {
        match self {
            Self::Preparing(table) | Self::Frozen(table) => table,
        }
    }

    fn table_mut(&mut self) -> Result<&mut PathSyntaxTable, CompilerError> {
        let Self::Preparing(table) = self else {
            return Err(CompilerError::compiler_error(
                "prepared-file path table was mutated after its immutable freeze boundary",
            ));
        };

        Arc::get_mut(table).ok_or_else(|| {
            CompilerError::compiler_error(
                "prepared-file path table was mutated while retained header streams already shared it",
            )
        })
    }

    fn validate_header_stream(&self, header_tokens: &FileTokens) -> Result<(), CompilerError> {
        match self {
            Self::Preparing(_) => header_tokens.require_deferred_path_syntax(),
            Self::Frozen(table) => header_tokens.require_shared_path_syntax(table),
        }
    }

    /// Change the lifecycle owner after all retained headers have been preflighted as deferred.
    ///
    /// The caller performs no fallible operations after this transition. Header streams receive
    /// cloned immutable handles immediately afterward, so an attachment failure cannot leave a
    /// partially frozen output or make a mutable table observable through copy-on-write.
    fn freeze_preflighted(&mut self) -> Arc<PathSyntaxTable> {
        let Self::Preparing(table) = self else {
            unreachable!("prepared-file path table was preflighted as preparing before freeze")
        };

        let table = Arc::clone(table);
        *self = Self::Frozen(Arc::clone(&table));
        table
    }
}

/// A source preparation owner retained independently of its result.
///
/// The same builder survives successful syntax, source diagnosis and infrastructure failure.
/// File/chunk consumers return it to the source owner before propagating any result.
pub(crate) struct SourcePreparationDelta {
    pub(crate) file_id: SourceId,
    pub(crate) span_builder: ExtendedSpanBuilder,
    pub(crate) result: Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure>,
}

/// A diagnosed file preparation result and warnings emitted before rejection.
///
/// Its source's live span builder belongs to the enclosing preparation owner.
#[derive(Debug)]
pub struct FileFrontendPrepareError {
    pub warnings: Vec<CompilerDiagnostic>,
    pub diagnostic: CompilerDiagnostic,
}
impl FileFrontendPrepareError {
    /// Remap every complete-path identity owned by this failed per-file output.
    ///
    /// Failed preparation keeps the same local path domain as successful output: warnings and the
    /// primary diagnostic may carry token, dependency, or place paths. The path fork must therefore
    /// be merged after strings and before the failure leaves the module-local preparation boundary.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }

        for warning in &mut self.warnings {
            warning.remap_path_ids(remap);
        }
        self.diagnostic.remap_path_ids(remap);
    }
}


/// Per-file preparation outcome that preserves the diagnostic and infrastructure lanes.
///
/// WHAT: carries ordinary authored-source rejection separately from an internal failure while
///       the caller still owns the file-local string-table delta.
/// WHY: remapping, source rebinding and path-table lifecycle failures mean retained compiler
///      state is malformed. Rendering them as source diagnostics would let Stage 0 continue with
///      an untrustworthy prepared-file boundary.
#[derive(Debug)]
pub enum FileFrontendPrepareFailure {
    Diagnosed(FileFrontendPrepareError),
    Infrastructure(CompilerError),
}

impl FileFrontendPrepareFailure {
    pub(crate) fn from_tokenization(failure: TokenizeFailure) -> Self {
        match failure {
            TokenizeFailure::Diagnosed(diagnostic) => Self::Diagnosed(FileFrontendPrepareError {
                warnings: Vec::new(),
                diagnostic,
            }),
            TokenizeFailure::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}

/// Header-parser local error lane before file-level warnings are attached.
///
/// WHAT: allows item parsers to return source diagnostics or compiler-state failures without
///       prematurely converting either one into the other.
/// WHY: the file parser is the only owner that can attach accumulated warnings to a diagnosed
///      source failure. It must forward malformed retained state unchanged to the frontend
///      boundary.
#[derive(Debug)]
pub(crate) enum HeaderParseFailure {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(CompilerError),
}

impl From<CompilerDiagnostic> for HeaderParseFailure {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for HeaderParseFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }
}

impl From<FileFrontendPrepareError> for FileFrontendPrepareFailure {
    fn from(error: FileFrontendPrepareError) -> Self {
        Self::Diagnosed(error)
    }
}

impl From<CompilerError> for FileFrontendPrepareFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }
}

impl FileFrontendPrepareOutput {
    /// Remap every interned string owned by this per-file output into the merged global string table.
    ///
    /// WHAT: remaps dependency target extensions, headers, const fragments, and warnings.
    /// WHY: per-file frontend preparation uses local string tables; merging them into the module
    ///      table requires shifting every `StringId` in retained syntax. Complete-path
    ///      identities (`source_file`, dependency paths, token file identities, path rows)
    ///      remap separately through `remap_path_ids` after the string delta merges.
    ///      stages resolve names through the global table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) -> Result<(), CompilerError> {
        // Per-file merges frequently append strings without changing numeric IDs; avoid walking
        // the full token/header/template payload when those IDs already target the merged table.
        if remap.is_identity() {
            return Ok(());
        }

        for clause in &mut self.file_dependency_clauses {
            clause.remap_string_ids(remap);
        }

        self.structural_file_references.remap_string_ids(remap);

        for selection in &mut self.dependency_selections {
            selection.source_name = remap.get(selection.source_name);
            if let Some(alias) = &mut selection.local_alias {
                alias.remap_string_ids(remap);
            }
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

        Ok(())
    }

    /// Remap every complete-path identity owned by this per-file output.
    ///
    /// WHAT: rewrites the file identity, retained dependency paths, header token file
    ///       identities and the file-owned path table through a worker-local merge remap.
    /// WHY: strings merge before paths; worker `StringId` components rewrite first, then
    ///      path nodes re-intern into the module fork. Callers must invoke `remap_string_ids`
    ///      first (when non-identity) and then this method with the corresponding path remap,
    ///      mirroring the chunk merge order.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) -> Result<(), CompilerError> {
        if remap.is_identity() {
            return Ok(());
        }
        self.source_file = remap.get(self.source_file);
        for clause in &mut self.file_dependency_clauses {
            clause.remap_path_ids(remap);
        }
        for header in &mut self.headers {
            header.remap_path_ids(remap);
        }
        for fragment in &mut self.top_level_const_fragments {
            fragment.remap_path_ids(remap);
        }
        self.path_syntax.table_mut()?.remap_path_ids(remap);
        Ok(())
    }

    /// Rebind the complete retained file output to its deterministic module source identity.
    ///
    /// WHAT: updates the output identity, the retained dependency path location in every clause, the
    ///       file-owned selection locations, header-owned token streams, detached declaration
    ///       syntax, fragment metadata and warning locations in one consuming-file operation.
    /// WHY: synthetic Stage 0 discovery prepares files before the complete closure is known, so
    ///      traversal-local `SourceId`s and absolute source scopes must be reconciled before the
    ///      output can enter module-wide aggregation. Retaining only dependency clauses would
    ///      lose the selection table and the syntax needed by later stages.
    pub fn rebind_source_identity(
        &mut self,
        final_file_id: SourceId,
        final_logical_path: PathId,
        canonical_os_path: std::path::PathBuf,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), CompilerError> {
        let provisional_source_file = self.source_file;
        // Validate all required source-owned paths and the mutable lifecycle before any retained
        // output is changed. A failed rebinding must never leave a partially finalised file.
        self.path_syntax.table_mut()?;
        self.validate_source_rebinding(provisional_source_file, &*path_fork)?;
        self.source_file = final_logical_path;
        self.file_id = final_file_id;
        self.canonical_os_path = Some(canonical_os_path.clone());

        for clause in &mut self.file_dependency_clauses {
            clause.commit_source_rebinding(final_file_id, final_logical_path);
        }
        self.structural_file_references
            .rebind_source_identity(final_file_id, final_logical_path);
        for selection in &mut self.dependency_selections {
            selection.rebind_source_identity(final_file_id);
        }
        for header in &mut self.headers {
            header.rebind_source_identity(
                final_file_id,
                final_logical_path.clone(),
                canonical_os_path.clone(),
                path_fork,
            )?;
        }
        for fragment in &mut self.top_level_const_fragments {
            fragment.rebind_source_identity(
                final_file_id,
                provisional_source_file,
                final_logical_path,
                path_fork,
            )?;
        }
        for warning in &mut self.warnings {
            rebind_source_span(&mut warning.primary_span, final_file_id);
            for label in &mut warning.labels {
                rebind_source_span(&mut label.span, final_file_id);
            }
        }

        // The table remains private to this output until every retained header stream has had
        // its top-level source identity rebound. Update its path-location scopes exactly once.
        self.path_syntax
            .table_mut()?
            .rebind_source_identity(final_file_id);
        Ok(())
    }

    /// Freeze the one file-owned table and attach it to every retained header token stream.
    ///
    /// The caller must complete all remapping and source-identity rebinding first. Once this
    /// succeeds, downstream parsing can interpret every `PathSyntaxId` against the same immutable
    /// table without copied rows or mutable shared state.
    pub(crate) fn freeze_path_syntax(
        &mut self,
        string_table: &StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), CompilerError> {
        self.validate_file_invariants(string_table, &*path_fork)?;
        if matches!(self.path_syntax, PreparedFilePathSyntax::Frozen(_)) {
            return Ok(());
        }

        // The invariant pass above proves every stream remains deferred. From this point onward
        // the transition has no fallible steps: freeze the sole owner, then attach that immutable
        // allocation to every already-validated retained header.
        let path_syntax = self.path_syntax.freeze_preflighted();
        for header in &mut self.headers {
            header
                .tokens
                .attach_preflighted_shared_path_syntax(Arc::clone(&path_syntax));
        }
        Ok(())
    }

    /// Confirm an output crossed its whole-file validation and freeze boundary earlier.
    ///
    /// Already-global synthetic outputs are validated when their final source identity is
    /// rebound. Aggregation uses this constant-time state check so it cannot traverse retained
    /// tokens, headers, clauses or path rows a second time.
    pub(crate) fn require_frozen_path_syntax(&self) -> Result<(), CompilerError> {
        if matches!(self.path_syntax, PreparedFilePathSyntax::Frozen(_)) {
            return Ok(());
        }

        Err(CompilerError::compiler_error(
            "already-global prepared-file output reached aggregation before its path table froze",
        ))
    }

    fn validate_source_rebinding(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        for header in &self.headers {
            header.validate_required_source_prefixes(provisional_source_file, path_fork)?;
        }
        for fragment in &self.top_level_const_fragments {
            fragment.validate_required_source_prefix(provisional_source_file, path_fork)?;
        }
        Ok(())
    }

    /// Validate the complete retained file before its table becomes immutable and shared.
    ///
    /// WHAT: checks one file identity, one final source scope, valid table handles and every
    ///       retained token-bearing shell that can outlive header preparation.
    /// WHY: this is the last boundary with the whole prepared-file output in one owner. Later
    ///      stages intentionally receive independent header substreams and must not reconstruct
    ///      source ownership, path-row validity or final source identity from those pieces.
    fn validate_file_invariants(
        &self,
        string_table: &StringTable,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        add_frontend_counter(FrontendCounter::PreparedFileInvariantValidationCount, 1);
        let path_syntax = self.path_syntax.table();
        path_syntax.validate_file_owned_locations(self.file_id)?;

        validate_dependency_clauses(
            &self.file_dependency_clauses,
            &self.dependency_selections,
            self.file_id,
            string_table,
            path_fork,
        )?;

        for header in &self.headers {
            self.path_syntax.validate_header_stream(&header.tokens)?;
            validate_header(
                header,
                self.file_id,
                self.canonical_os_path.as_deref(),
                self.source_file,
                path_syntax,
                path_fork,
            )?;
        }

        for fragment in &self.top_level_const_fragments {
            if !path_fork.starts_with(fragment.header_path, self.source_file) {
                return Err(CompilerError::compiler_error(
                    "top-level const fragment retained a path outside its prepared source file",
                ));
            }
            validate_source_span(
                Some(fragment.span),
                self.file_id,
                "top-level const fragment",
            )?;
        }
        Ok(())
    }
}
fn validate_dependency_clauses(
    clauses: &[RetainedDependencyClause],
    selections: &[DependencySelection],
    file_id: SourceId,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    let mut next_selection_start = 0usize;

    for (clause_index, clause) in clauses.iter().enumerate() {
        let expected_ordinal = u32::try_from(clause_index).map_err(|_| {
            CompilerError::compiler_error(
                "prepared file contains more dependency clauses than its dense shell identity can represent",
            )
        })?;
        validate_dependency_path(&clause.dependency, file_id, string_table, path_fork)?;
        if clause.dependency.dependency_shell_id.ordinal != expected_ordinal {
            return Err(CompilerError::compiler_error(
                "retained dependency clause shell ordinal does not match its dense file-local clause position",
            ));
        }

        match &clause.binding {
            DependencyBindingSyntax::Namespace { alias } => {
                if clause.export_mode.is_public() {
                    return Err(CompilerError::compiler_error(
                        "public dependency clause retained a namespace binding instead of direct selections",
                    ));
                }
                if let Some(alias) = alias {
                    validate_source_span(Some(alias.span), file_id, "dependency namespace alias")?;
                }
            }
            DependencyBindingSyntax::DirectSelections { range } => {
                let start = usize::try_from(range.start).map_err(|_| {
                    CompilerError::compiler_error(
                        "dependency selection range start does not fit usize",
                    )
                })?;
                if start != next_selection_start {
                    return Err(CompilerError::compiler_error(
                        "dependency selection ranges do not partition the file-owned table in clause order",
                    ));
                }
                let selected = range.get(selections)?;
                if selected.is_empty() {
                    return Err(CompilerError::compiler_error(
                        "direct dependency selection clause retained an empty selection range",
                    ));
                }
                for selection in selected {
                    validate_dependency_selection(selection, file_id)?;
                }
                next_selection_start = usize::try_from(range.end).map_err(|_| {
                    CompilerError::compiler_error(
                        "dependency selection range end does not fit usize",
                    )
                })?;
            }
        }
    }

    if next_selection_start != selections.len() {
        return Err(CompilerError::compiler_error(
            "file-owned dependency selection table contains unclaimed rows",
        ));
    }
    Ok(())
}

fn validate_dependency_path(
    dependency: &RetainedDependencyPath,
    file_id: SourceId,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    if dependency.path == PathId::ROOT {
        return Err(CompilerError::compiler_error(
            "retained dependency clause has an empty path",
        ));
    }
    decode_dependency_target(dependency.path, &dependency.target, path_fork, string_table)?;
    validate_source_span(Some(dependency.span), file_id, "dependency path")?;
    if dependency.dependency_shell_id.source == file_id {
        Ok(())
    } else {
        Err(CompilerError::compiler_error(
            "dependency shell identity does not match the prepared file identity",
        ))
    }
}

fn validate_dependency_selection(
    selection: &DependencySelection,
    file_id: SourceId,
) -> Result<(), CompilerError> {
    validate_source_span(Some(selection.source_span), file_id, "dependency selection")?;
    if let Some(alias) = &selection.local_alias {
        validate_source_span(Some(alias.span), file_id, "dependency selection alias")?;
    }
    Ok(())
}
fn validate_header(
    header: &Header,
    file_id: SourceId,
    canonical_os_path: Option<&std::path::Path>,
    source_file: PathId,
    path_syntax: &PathSyntaxTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    if header.source_file != source_file {
        return Err(CompilerError::compiler_error(
            "retained header source file does not match its prepared-file identity",
        ));
    }
    if !path_fork.starts_with(header.tokens.src_path, source_file) {
        return Err(CompilerError::compiler_error(
            "retained header path does not use the prepared file's final source prefix",
        ));
    }
    if header.tokens.file_id != file_id {
        return Err(CompilerError::compiler_error(
            "retained header token stream does not match the prepared file identity",
        ));
    }
    if header.tokens.canonical_os_path.as_deref() != canonical_os_path {
        return Err(CompilerError::compiler_error(
            "retained header token stream does not match the prepared file's canonical path",
        ));
    }
    validate_source_span(header.name_span, file_id, "header name")?;
    validate_tokens(&header.tokens.tokens, file_id, path_syntax, "header body")?;
    for hint in &header.local_ordering_hints {
        if hint.origin() == LocalDeclarationOrderingHintOrigin::SourceOwned
            && !path_fork.starts_with(hint.path(), source_file)
        {
            return Err(CompilerError::compiler_error(
                "source-owned declaration ordering hint retained a provisional source prefix",
            ));
        }
    }
    for reference in &header.capacity_references {
        validate_source_span(reference.span, file_id, "header capacity reference")?;
    }
    validate_header_kind(&header.kind, file_id, source_file, path_syntax, path_fork)
}

fn validate_header_kind(
    kind: &HeaderKind,
    file_id: SourceId,
    source_file: PathId,
    path_syntax: &PathSyntaxTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    match kind {
        HeaderKind::Function {
            generic_parameters,
            signature,
        } => {
            validate_generic_parameters(generic_parameters, file_id)?;
            validate_function_signature(signature, file_id, source_file, path_syntax, path_fork)?;
        }
        HeaderKind::Constant { declaration } => {
            validate_declaration_syntax(declaration, file_id, source_file, path_syntax)?;
        }
        HeaderKind::Struct {
            generic_parameters,
            fields,
        } => {
            validate_generic_parameters(generic_parameters, file_id)?;
            for field in fields {
                validate_signature_member(field, file_id, source_file, path_syntax, path_fork)?;
            }
        }
        HeaderKind::Choice {
            generic_parameters,
            variants,
        } => {
            validate_generic_parameters(generic_parameters, file_id)?;
            for variant in variants {
                validate_choice_variant(variant, file_id, source_file, path_syntax, path_fork)?;
            }
        }
        HeaderKind::TypeAlias { target } => {
            validate_parsed_type_ref(target, file_id)?;
        }
        HeaderKind::ConstTemplate {
            condition_references,
        } => {
            for reference in condition_references {
                validate_source_span(
                    reference.span,
                    file_id,
                    "const-template condition reference",
                )?;
            }
        }
        HeaderKind::StartFunction => {}
        HeaderKind::Trait { declaration } => {
            // Trait name/reference fields currently retain source-local spans without an owning
            // SourceId. Their nested signatures carry the source-qualified spans that can be
            // checked here.
            for requirement in &declaration.requirements {
                validate_function_signature(
                    &requirement.signature,
                    file_id,
                    source_file,
                    path_syntax,
                    path_fork,
                )?;
            }
        }
        // Trait metadata currently stores LocalSpan values for these names, so there is no global
        // source identity to validate at this boundary.
        HeaderKind::TraitConformance { .. } | HeaderKind::TraitIncompatibility { .. } => {}
    }
    Ok(())
}

fn validate_generic_parameters(
    parameters: &GenericParameterList,
    file_id: SourceId,
) -> Result<(), CompilerError> {
    for parameter in &parameters.parameters {
        validate_source_span(parameter.span, file_id, "generic parameter")?;
        for bound in &parameter.trait_bounds {
            validate_source_span(bound.span, file_id, "generic parameter bound")?;
        }
    }
    Ok(())
}

fn validate_function_signature(
    signature: &FunctionSignatureSyntax,
    file_id: SourceId,
    source_file: PathId,
    path_syntax: &PathSyntaxTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    for parameter in &signature.parameters {
        validate_signature_member(parameter, file_id, source_file, path_syntax, path_fork)?;
    }
    for return_slot in &signature.returns {
        validate_source_span(return_slot.value.span, file_id, "function return")?;
        validate_parsed_type_ref(&return_slot.value.type_annotation, file_id)?;
    }
    Ok(())
}
fn validate_signature_member(
    member: &SignatureMemberSyntax,
    file_id: SourceId,
    source_file: PathId,
    path_syntax: &PathSyntaxTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    if !path_fork.starts_with(member.id, source_file) {
        return Err(CompilerError::compiler_error(
            "retained declaration member path does not use the prepared file's final source prefix",
        ));
    }
    validate_source_span(member.span, file_id, "declaration member")?;
    validate_parsed_type_ref(&member.type_annotation, file_id)?;
    validate_tokens(
        &member.default_tokens,
        file_id,
        path_syntax,
        "member default",
    )
}

fn validate_choice_variant(
    variant: &ChoiceVariantSyntax,
    file_id: SourceId,
    source_file: PathId,
    path_syntax: &PathSyntaxTable,
    path_fork: &PathInternerFork,
) -> Result<(), CompilerError> {
    validate_source_span(variant.span, file_id, "choice variant")?;
    match &variant.payload {
        crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Unit => {}
        crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Record {
            fields,
        } => {
            for field in fields.iter() {
                validate_signature_member(field, file_id, source_file, path_syntax, path_fork)?;
            }
        }
    }
    Ok(())
}

fn validate_declaration_syntax(
    declaration: &DeclarationSyntax,
    file_id: SourceId,
    _source_file: PathId,
    path_syntax: &PathSyntaxTable,
) -> Result<(), CompilerError> {
    validate_source_span(declaration.span, file_id, "declaration shell")?;
    validate_parsed_type_ref(&declaration.type_annotation, file_id)?;
    validate_tokens(
        &declaration.initializer_tokens,
        file_id,
        path_syntax,
        "declaration initializer",
    )?;
    for reference in &declaration.initializer_references {
        validate_source_span(reference.span, file_id, "declaration initializer reference")?;
    }
    Ok(())
}

fn validate_parsed_type_ref(
    type_ref: &ParsedTypeRef,
    file_id: SourceId,
) -> Result<(), CompilerError> {
    match type_ref {
        ParsedTypeRef::Inferred => {}
        ParsedTypeRef::Named { span, .. }
        | ParsedTypeRef::Qualified { span, .. }
        | ParsedTypeRef::BuiltinBool { span }
        | ParsedTypeRef::BuiltinInt { span }
        | ParsedTypeRef::BuiltinFloat { span }
        | ParsedTypeRef::BuiltinString { span }
        | ParsedTypeRef::BuiltinChar { span }
        | ParsedTypeRef::This { span } => {
            validate_source_span(*span, file_id, "parsed type")?;
        }
        ParsedTypeRef::Applied {
            base,
            arguments,
            span,
        } => {
            validate_parsed_type_ref(base, file_id)?;
            for argument in arguments {
                validate_parsed_type_ref(argument, file_id)?;
            }
            validate_source_span(*span, file_id, "applied type")?;
        }
        ParsedTypeRef::Collection {
            element,
            span,
            fixed_capacity,
        } => {
            validate_parsed_type_ref(element, file_id)?;
            validate_source_span(*span, file_id, "collection type")?;
            if let Some(capacity) = fixed_capacity {
                match capacity {
                    ParsedCollectionCapacity::Literal { span, .. }
                    | ParsedCollectionCapacity::BareConstant { span, .. } => {
                        validate_source_span(*span, file_id, "collection capacity")?;
                    }
                }
            }
        }
        ParsedTypeRef::Map {
            key, value, span, ..
        } => {
            validate_parsed_type_ref(key, file_id)?;
            validate_parsed_type_ref(value, file_id)?;
            validate_source_span(*span, file_id, "map type")?;
        }
        ParsedTypeRef::Optional { inner, span, .. } => {
            validate_parsed_type_ref(inner, file_id)?;
            validate_source_span(*span, file_id, "optional type")?;
        }
    }
    Ok(())
}

fn validate_tokens(
    tokens: &[Token],
    file_id: SourceId,
    path_syntax: &PathSyntaxTable,
    role: &str,
) -> Result<(), CompilerError> {
    path_syntax.validate_file_tokens(tokens, file_id, role)
}

fn validate_source_span(
    span: Option<SourceSpan>,
    expected_source: SourceId,
    role: &str,
) -> Result<(), CompilerError> {
    if let Some(span) = span
        && span.source() != expected_source
    {
        return Err(CompilerError::compiler_error(format!(
            "{role} span does not use the prepared file's source identity"
        )));
    }
    Ok(())
}

impl FileFrontendPrepareError {
    /// Remap every interned string owned by this failed per-file output into the merged global
    /// string table.
    ///
    /// WHAT: remaps warnings and the primary diagnostic.
    /// WHY: per-file frontend preparation uses local string tables. Even failed files may have
    ///      emitted warnings before the error, and those strings must resolve through the global
    ///      table. The source identity and span-builder ownership are source-local and do not
    ///      participate in string-ID remapping.
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

    pub(crate) fn into_premerge_batch(self, string_table: StringTable) -> PremergeDiagnosticBatch {
        let Self {
            mut warnings,
            diagnostic,
        } = self;
        warnings.push(diagnostic);
        PremergeDiagnosticBatch::from_diagnostics(warnings, string_table)
    }
}

// Shared file-level state that stays live while one source file is being split into headers.
pub(super) struct HeaderParseContext<'a> {
    pub file_role: FileRole,
    pub is_config_file: bool,
    pub string_table: &'a mut StringTable,
    pub path_fork: &'a mut PathInternerFork,
    pub span_builder: &'a mut ExtendedSpanBuilder,
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
    pub source_file: PathId,
    pub file_dependency_clauses: &'a [RetainedDependencyClause],
    pub dependency_selections: &'a [DependencySelection],
    pub string_table: &'a mut StringTable,
    pub path_fork: &'a mut PathInternerFork,
    pub file_role: FileRole,
}
