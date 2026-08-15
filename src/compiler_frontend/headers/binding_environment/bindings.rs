//! Header-stage binding environment data shapes.
//!
//! WHAT: defines `FileVisibility` and `HeaderBindingEnvironment`, the per-file visibility maps
//! produced by header binding preparation and consumed by dependency sorting and AST.
//! WHY: after header parsing, every source file needs a stable, complete visibility snapshot
//! so later stages do not rebuild dependency bindings or rediscover top-level symbols.
//! MUST NOT: parse executable bodies, fold constants, or perform AST semantic validation.

use crate::compiler_frontend::canonical_type_identity::CanonicalEvidenceIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_interface::{PublicDeclarationRecord, PublicEvidenceRecord};
use crate::compiler_frontend::semantic_identity::{
    ModulePrivateExecutableIdentity, OriginDeclarationId, OriginFunctionId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::{FxHashMap, FxHashSet};

/// Per-file visible-name environment.
///
/// WHAT: maps the names visible in one source file to their resolved targets.
/// WHY: AST `ScopeContext` consumes this directly instead of rebuilding binding visibility.
///
/// Includes same-file declarations, source dependencies, external symbols, type aliases, and
/// builtin/prelude reservations. Name collision policy is enforced during construction.
/// A member of a namespace record that is valid in value/expression context.
#[derive(Clone, Debug)]
pub(crate) enum NamespaceValueMember {
    SourceDeclaration(SourceDeclarationTarget),
    ExternalSymbol(ExternalSymbolId),
}

/// A member of a namespace record that is valid in type context.
#[derive(Clone, Debug)]
pub(crate) enum NamespaceTypeMember {
    SourceDeclaration(SourceDeclarationTarget),
    ExternalSymbol(ExternalSymbolId),
}

/// Where a namespace record originated, for diagnostics and HIR boundary checks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NamespaceRecordSource {
    SourceFile(InternedPath),
    ExternalPackage(StringId),
}

/// A field-access-only namespace binding record built from one dependency surface.
///
/// WHAT: maps member names to their resolved targets so AST can resolve
/// `namespace.member` and `namespace.Type` without rebuilding binding visibility.
/// WHY: namespace bindings expose value and type members separately; mixing them
/// in the wrong context produces targeted diagnostics. External package surfaces
/// can additionally expose child namespace records for nested symbol paths.
/// BOUNDARY: source and public export namespace records remain shallow; child namespaces
/// are only built from external package symbol paths.
#[derive(Clone, Debug)]
pub(crate) struct NamespaceRecord {
    pub(crate) value_members: FxHashMap<StringId, NamespaceValueMember>,
    pub(crate) type_members: FxHashMap<StringId, NamespaceTypeMember>,
    /// Child namespace records for external package surfaces that expose nested
    /// namespaces such as `io.input`.
    ///
    /// WHAT: maps a namespace member name to its own namespace record, allowing
    /// nested dotted paths to be represented at header/binding visibility without
    /// creating runtime namespace values.
    /// WHY: external packages can register multi-component symbol paths; this tree
    /// mirrors those paths so later phases can walk them by dotted name.
    /// BOUNDARY: source and public export namespace records remain shallow; child namespaces
    /// are only populated from external package symbol paths.
    pub(crate) child_namespaces: FxHashMap<StringId, NamespaceRecord>,
    /// Where this namespace record originated.
    ///
    /// WHAT: records whether the record was built from a source file, a public export, or an
    /// external package surface. AST uses this to produce the correct diagnostic for
    /// nested traversal attempts: source and public export records are shallow, so any
    /// second dot is rejected with the existing `nested_traversal` diagnostic, while
    /// external records may expose child namespaces.
    /// WHY: the record structure itself does not encode origin; keeping origin here lets
    /// value-position traversal respect the source/public export shallow boundary without
    /// rebuilding binding visibility in the expression parser.
    pub(crate) record_source: NamespaceRecordSource,
}

impl NamespaceRecord {
    /// Create an empty namespace record with the given origin.
    ///
    /// WHAT: helper used by record builders and tests to start a fresh record.
    /// WHY: every record must carry its origin so AST traversal can distinguish
    /// shallow source/public export records from recursive external package records.
    pub(crate) fn empty(record_source: NamespaceRecordSource) -> Self {
        Self {
            value_members: FxHashMap::default(),
            type_members: FxHashMap::default(),
            child_namespaces: FxHashMap::default(),
            record_source,
        }
    }
}

/// Result of looking up one dotted name inside a namespace record.
///
/// WHAT: tells the caller whether the name leads to another namespace, a value leaf,
/// a type leaf, or is absent. The caller decides how to report each case based on
/// position in the dotted path.
/// WHY: traversal and diagnostics are separate concerns; this enum lets the parser
/// loop stay readable while the lookup details live in one place.
/// BOUNDARY: this is shared between value-position namespace access and type-position
/// namespace resolution; keep it near `NamespaceRecord` so both stages use the same
/// member-classification semantics.
pub(crate) enum NamespaceMemberLookup<'a> {
    ChildNamespace(&'a NamespaceRecord),
    Value(&'a NamespaceValueMember),
    Type,
    Missing,
}

/// Look up one member name in a namespace record.
///
/// WHAT: searches child namespaces first, then value members, then type members.
/// WHY: namespace slots are exclusive in the record builder, so at most one branch
/// can match; the order only affects which diagnostic the caller produces when a
/// name is both a namespace and a leaf (which cannot happen for valid records).
pub(crate) fn lookup_namespace_member<'a>(
    record: &'a NamespaceRecord,
    name: StringId,
) -> NamespaceMemberLookup<'a> {
    if let Some(child) = record.child_namespaces.get(&name) {
        return NamespaceMemberLookup::ChildNamespace(child);
    }

    if let Some(value_member) = record.value_members.get(&name) {
        return NamespaceMemberLookup::Value(value_member);
    }

    if record.type_members.contains_key(&name) {
        return NamespaceMemberLookup::Type;
    }

    NamespaceMemberLookup::Missing
}

/// One receiver method made visible to a source file.
///
/// WHAT: stores the canonical function path plus the dependency/declaration location that made the
/// method visible.
/// WHY: receiver methods live in the receiver-call namespace rather than the ordinary value
/// namespace, so they need their own visibility entries and diagnostics.
#[derive(Clone, Debug)]
pub(crate) struct ReceiverMethodVisibility {
    pub(crate) target: SourceFunctionTarget,
    pub(crate) location: SourceLocation,
}

/// One source declaration visible in a consumer file.
///
/// Local declarations retain their module-local path handle. Imported declarations retain the
/// stable origin selected from a completed provider interface plus a consumer-local path handle
/// used only to index projected AST facts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SourceDeclarationTarget {
    Local(InternedPath),
    Imported {
        origin: OriginDeclarationId,
        local_path: InternedPath,
    },
}

impl SourceDeclarationTarget {
    pub(crate) fn local_path(&self) -> &InternedPath {
        match self {
            Self::Local(path)
            | Self::Imported {
                local_path: path, ..
            } => path,
        }
    }
}

impl std::ops::Deref for SourceDeclarationTarget {
    type Target = InternedPath;

    fn deref(&self) -> &Self::Target {
        self.local_path()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SourceFunctionTarget {
    Local(InternedPath),
    Imported {
        origin: OriginFunctionId,
        local_path: InternedPath,
    },
    Generated {
        identity: crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
        local_path: InternedPath,
    },
    ModulePrivate {
        identity: ModulePrivateExecutableIdentity,
        local_path: InternedPath,
    },
}

impl SourceFunctionTarget {
    pub(crate) fn local_path(&self) -> &InternedPath {
        match self {
            Self::Local(path)
            | Self::Imported {
                local_path: path, ..
            }
            | Self::Generated {
                local_path: path, ..
            }
            | Self::ModulePrivate {
                local_path: path, ..
            } => path,
        }
    }
}

impl std::ops::Deref for SourceFunctionTarget {
    type Target = InternedPath;

    fn deref(&self) -> &Self::Target {
        self.local_path()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FileVisibility {
    /// Declaration paths that are visible in this file (including builtins).
    /// Used as an access gate for permission checks.
    pub(crate) visible_declaration_paths: FxHashSet<InternedPath>,

    /// Source-visible names → canonical declaration path.
    /// Includes same-file declarations and imported source symbols (aliased or not).
    pub(crate) visible_source_names: FxHashMap<StringId, SourceDeclarationTarget>,

    /// Type aliases: local visible name → canonical type alias path.
    pub(crate) visible_type_alias_names: FxHashMap<StringId, SourceDeclarationTarget>,

    /// Trait declarations: local visible name → canonical trait path.
    ///
    /// WHY: traits are compile-time contracts, not value declarations or datatypes. Keeping them
    /// in their own map lets conformances and generic bounds resolve imported trait names without
    /// making `TRAIT` usable as a normal type annotation.
    pub(crate) visible_trait_names: FxHashMap<StringId, SourceDeclarationTarget>,

    /// External package functions/types/constants visible from this file.
    /// Populated by explicit virtual-package dependencies and prelude symbols.
    pub(crate) visible_external_symbols: FxHashMap<StringId, ExternalSymbolId>,

    /// Authored source locations that made explicit external symbols visible in this file.
    ///
    /// WHY: AST needs the authored local-binding location for duplicate-declaration
    /// diagnostics so the secondary label can point to the selected name or alias. Prelude
    /// symbols have no authored location and therefore have no entry in this map.
    pub(crate) visible_external_symbol_locations: FxHashMap<StringId, SourceLocation>,

    /// Namespace namespace binding records visible in this file.
    /// Populated by bare `@path` and `@path as alias` syntax.
    pub(crate) visible_namespace_records: FxHashMap<StringId, NamespaceRecord>,

    /// Receiver methods visible in this file.
    /// Key is the source method name.
    /// Value is the list of canonical function paths with that local name.
    /// WHY: receiver methods are callable only through receiver syntax, not as free
    ///      functions or dependency-namespace value members. Binding preparation derives
    ///      this map from visible receiver types so AST lookup can filter the
    ///      module-wide catalog by file.
    ///      Multiple paths per name are needed because different receiver types can
    ///      share a method name (e.g. Person.name and Company.name).
    ///      Only same-file nominal receiver types contribute source-authored methods;
    ///      builtins, external types, and types declared in other files do not.
    pub(crate) visible_receiver_methods: FxHashMap<StringId, Vec<ReceiverMethodVisibility>>,
}

/// Header-built binding environment for the entire module.
///
/// WHAT: collects one `FileVisibility` per parsed source file.
/// WHY: dependency sorting and AST need stable per-file visibility without rebuilding binding
/// semantics in later stages.
#[derive(Clone, Debug, Default)]
pub(crate) struct HeaderBindingEnvironment {
    pub(crate) file_visibility_by_source: FxHashMap<InternedPath, FileVisibility>,
    /// Consumer-local declaration paths mapped to their stable provider origins.
    ///
    /// WHAT: aliases and namespace members reference the one record stored under
    ///       [`HeaderBindingEnvironment::imported_declarations_by_origin`] instead of cloning the
    ///       complete declaration payload per local path.
    pub(crate) imported_declarations_by_local_path: FxHashMap<InternedPath, OriginDeclarationId>,
    /// Stable declaration closure supplied by completed provider interfaces.
    ///
    /// AST projects these records into consumer-local semantic handles. Keeping the stable
    /// records here lets nested canonical types resolve without retaining or reopening donor
    /// syntax, AST, HIR or donor-local `TypeId` values.
    pub(crate) imported_declarations_by_origin: FxHashMap<
        crate::compiler_frontend::semantic_identity::OriginDeclarationId,
        PublicDeclarationRecord,
    >,
    /// Stable reusable evidence supplied by imported provider interfaces, keyed by canonical
    /// evidence identity.
    ///
    /// Equal records deduplicate; differing records claiming one identity fail before AST
    /// projection.
    pub(crate) imported_evidence_by_identity:
        FxHashMap<CanonicalEvidenceIdentity, PublicEvidenceRecord>,
    /// Stable concrete call summaries supplied by imported provider interfaces, keyed by
    /// function origin.
    ///
    /// Local function contracts reference these summaries by origin instead of cloning the
    /// summary payload per alias or receiver method.
    pub(crate) imported_call_summaries_by_origin: FxHashMap<OriginFunctionId, PublicCallSummary>,
    pub(crate) imported_functions_by_local_path: FxHashMap<InternedPath, ImportedFunctionContract>,
    pub(crate) warnings: Vec<CompilerDiagnostic>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedFunctionContract {
    pub(crate) target: SourceFunctionTarget,
}

impl HeaderBindingEnvironment {
    /// Return the visibility map for a parsed source file.
    ///
    /// WHY: missing visibility means header preparation failed to populate its stage contract.
    /// This should only happen if a file was added to `module_file_paths` without running
    /// binding environment construction.
    pub(crate) fn visibility_for(
        &self,
        source_file: &InternedPath,
    ) -> Result<&FileVisibility, CompilerError> {
        self.file_visibility_by_source.get(source_file).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Missing visibility entry for source file. This is a compiler bug: header parsing did not produce a visibility map for '{source_file:?}'."
            ))
        })
    }
}
