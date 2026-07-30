//! Compiler-owned stable semantic identity values that cross compiler and build stages.
//!
//! WHAT: owns the portable, hashable, cross-build identity values for source packages, canonical
//! module origins and exported declarations. These values are the stable semantic identity
//! contract used by later exported-declaration identity, public-interface binding and
//! cross-module call targets. The dense `ModuleId` and identity table remain Stage 0-owned and
//! live in `build_system::create_project_modules::module_identity`; this module owns only the
//! portable value types those dense handles refer to.
//! WHY: the compiler owns semantic identity. Stage 0 owns discovery, the dense assignment table
//! and structural ancestry, but the cross-build identity values must not be build-local. Moving
//! the value types here keeps one owner for the cross-stage identity vocabulary so later phases
//! embed stable origins without reaching into the build system and without leaking process-local
//! IDs, source files, declaration order or export aliases into identity.
//!
//! Boundary: reusable evidence identity is deliberately not owned here. Evidence identity needs
//! canonical target-type and trait/evidence semantics that a later phase owns, so this module
//! does not invent a string-based or placeholder evidence key.

use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

use std::path::{Component, Path};

/// Stable declaration target for one generated generic function.
///
/// Public generics retain their consumer-visible origin. Private generics use the same
/// artefact-scoped executable identity as other private call targets, so they remain resolvable
/// without acquiring a public interface identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum GeneratedDeclarationIdentity {
    Public(OriginFunctionId),
    ModulePrivate(ModulePrivateExecutableIdentity),
}

impl GeneratedDeclarationIdentity {
    pub(crate) fn defining_name(&self) -> &str {
        match self {
            Self::Public(origin) => origin.defining_name(),
            Self::ModulePrivate(identity) => identity.defining_name(),
        }
    }
}

/// Stable identity for one concrete generated generic function.
///
/// The build-owned worklist deduplicates this exact declaration/type/evidence tuple across
/// requesters. Call locations and requester modules remain diagnostic context and never affect
/// identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GeneratedFunctionIdentity {
    declaration: GeneratedDeclarationIdentity,
    type_arguments: Box<[CanonicalTypeIdentity]>,
    evidence: Box<[CanonicalEvidenceIdentity]>,
}

impl GeneratedFunctionIdentity {
    pub(crate) fn new(
        declaration: GeneratedDeclarationIdentity,
        type_arguments: Box<[CanonicalTypeIdentity]>,
        evidence: Box<[CanonicalEvidenceIdentity]>,
    ) -> Self {
        Self {
            declaration,
            type_arguments,
            evidence,
        }
    }

    pub(crate) fn declaration(&self) -> &GeneratedDeclarationIdentity {
        &self.declaration
    }

    pub(crate) fn type_arguments(&self) -> &[CanonicalTypeIdentity] {
        &self.type_arguments
    }

    pub(crate) fn evidence(&self) -> &[CanonicalEvidenceIdentity] {
        &self.evidence
    }
}

/// Artefact-scoped identity for an executable that is not part of a public interface.
///
/// Private executables remain resolvable by generated sidecars without acquiring a public
/// [`OriginFunctionId`]. The declaring source binds the identity to its defining artefact scope,
/// while the optional receiver path keeps same-named methods on distinct module-private receiver
/// surfaces disjoint. The receiver path is artefact-local by design: private source nominals do
/// not acquire a public [`OriginTypeId`] merely so their methods can be called from a generated
/// sidecar. Generated sidecars retain the declaring implementation compatibility fingerprint
/// separately because compatibility and executable identity have different owners.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModulePrivateExecutableIdentity {
    module_origin: StableModuleOriginIdentity,
    declaring_source: String,
    category: ModulePrivateExecutableCategory,
    defining_name: String,
    receiver_path: Option<String>,
}

impl ModulePrivateExecutableIdentity {
    pub(crate) fn new(
        module_origin: StableModuleOriginIdentity,
        declaring_source: String,
        category: ModulePrivateExecutableCategory,
        defining_name: String,
        receiver_path: Option<String>,
    ) -> Self {
        Self {
            module_origin,
            declaring_source,
            category,
            defining_name,
            receiver_path,
        }
    }

    pub(crate) fn defining_name(&self) -> &str {
        &self.defining_name
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(dead_code)]
pub(crate) enum ModulePrivateExecutableCategory {
    FreeFunction,
    ReceiverMethod,
    GenericFunction,
}

/// Owned, hashable, cross-build identity for one source package within one build boundary.
///
/// WHAT: carries the package origin and the canonical package/project name. For the project
/// graph it is constructed from [`PackageOrigin::ProjectLocal`] and the exact configured
/// `Config.project_name`. It stores neither absolute filesystem paths nor process-local
/// string-table IDs, so the same logical package resolves to the same identity across checkout
/// roots, processes and cosmetic root-filename suffixes.
/// WHY: later exported declaration identities embed the package identity so origin identities
/// remain stable when source moves across machines or checkouts. Identity is never inferred from
/// checkout-directory names or absolute paths.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StablePackageIdentity {
    origin: PackageOrigin,
    name: String,
}

impl StablePackageIdentity {
    /// Project-local package identity for the project graph, from the configured project name.
    ///
    /// The configured name is preserved exactly as supplied. Validation of empty or malformed
    /// project names belongs to config/bootstrap owners and is intentionally not added here.
    pub(crate) fn project_local(project_name: &str) -> Self {
        Self {
            origin: PackageOrigin::ProjectLocal,
            name: project_name.to_owned(),
        }
    }

    /// Stable identity for one source-package compilation boundary.
    ///
    /// WHAT: builds the package identity from the registry's `PackageOrigin` and the package's
    /// import prefix. The import prefix is the portable `@`-stripped spelling (for example
    /// `"html"` or a project-local `"helper"`), so two packages with the same prefix but
    /// different origins remain distinct, and the same logical package resolves to the same
    /// identity across checkout roots.
    /// WHY: each source-package boundary owns its own `SourceTreeIndex` with boundary-local
    /// `SourceId`/`ModuleId` values. The stable package identity is the only cross-boundary
    /// value those dense handles refer to, so it must be derivable from registry facts rather
    /// than the configured project name or an absolute path.
    pub(crate) fn source_package(origin: PackageOrigin, import_prefix: &str) -> Self {
        Self {
            origin,
            name: import_prefix.to_owned(),
        }
    }

    /// Stable identity for one binding-backed package.
    ///
    /// Binding packages retain their exact canonical import path, including the leading `@`,
    /// because that path and the package origin together identify the builder-supplied semantic
    /// interface across independent registry constructions.
    pub(crate) fn binding(origin: PackageOrigin, canonical_package_path: &str) -> Self {
        Self {
            origin,
            name: canonical_package_path.to_owned(),
        }
    }

    /// The package origin classification.
    #[allow(dead_code)]
    pub(crate) fn origin(&self) -> PackageOrigin {
        self.origin
    }

    /// The canonical package/project name spelling.
    #[allow(dead_code)]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

/// The structural role of one canonical module root.
///
/// `Normal` roots (`#*.moth`) are entry candidates. `Support` roots (`+*.moth`) are scoped package
/// roots that are never entry candidates. `ProjectPackageFacade` is the optional project-root
/// `+*.moth` beside `config.moth`; Stage 0 assigns it from location rather than filename alone and
/// it never participates in entry-root containment or import-resolution lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ModuleRootRole {
    Normal,
    Support,
    ProjectPackageFacade,
}

impl ModuleRootRole {
    pub(crate) fn has_implicit_start(self) -> bool {
        matches!(self, Self::Normal)
    }
}

/// Owned, hashable, cross-build origin identity for one canonical module.
///
/// WHAT: derives a stable module origin from the owning [`StablePackageIdentity`], the canonical
/// portable logical module path (forward-slash logical spelling, including the empty entry-root
/// path) and the [`ModuleRootRole`]. It stores no `PathBuf`, `StringId`, `InternedPath`, dense
/// `ModuleId` or absolute filesystem path, so identity is stable across checkout roots,
/// traversal order, cosmetic root-filename suffixes and the ordinary source file that contains
/// a declaration.
/// WHY: later exported declaration identities embed this module origin identity. Keeping the
/// dense `ModuleId` as the build-local handle prevents process-local indexes from leaking across
/// module boundaries or into persistent artefacts.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StableModuleOriginIdentity {
    package: StablePackageIdentity,
    logical_module_path: String,
    role: ModuleRootRole,
}

impl StableModuleOriginIdentity {
    /// Build the cross-build identity from a base-relative logical module path.
    ///
    /// `relative_logical_path` is the `PathBuf` produced by stripping the canonical root
    /// directory against its base (the entry root, or the project root for the facade). It is
    /// converted to a portable forward-slash spelling so the identity is self-contained and
    /// platform-independent.
    ///
    /// Only normal relative components are accepted. `CurDir`, `ParentDir`, `RootDir`, `Prefix`
    /// and non-UTF-8 components are rejected through an internal `CompilerError` so two invalid
    /// inputs can never collapse to the same stable identity. Stage 0's earlier UTF-8 and
    /// base-relative validation makes these invariant failures, but the constructor remains
    /// total rather than panicking.
    pub(crate) fn from_relative_logical_path(
        package: StablePackageIdentity,
        relative_logical_path: &Path,
        role: ModuleRootRole,
    ) -> Result<Self, CompilerError> {
        Ok(Self {
            package,
            logical_module_path: portable_relative_logical_path_from(relative_logical_path)?,
            role,
        })
    }

    /// Construct a stable module origin from already-portable spelling inputs.
    ///
    /// Compiler-internal helper for tests and later phases that already hold the portable
    /// spelling and the owned package identity. Production Stage 0 construction goes through
    /// [`StableModuleOriginIdentity::from_relative_logical_path`].
    #[cfg(test)]
    pub(crate) fn from_portable_path(
        package: StablePackageIdentity,
        logical_module_path: String,
        role: ModuleRootRole,
    ) -> Self {
        Self {
            package,
            logical_module_path,
            role,
        }
    }

    /// The owning stable package identity.
    #[allow(dead_code)]
    pub(crate) fn package(&self) -> &StablePackageIdentity {
        &self.package
    }

    /// The canonical portable logical module path spelling (forward slashes, empty for the
    /// entry root).
    #[allow(dead_code)]
    pub(crate) fn logical_module_path(&self) -> &str {
        &self.logical_module_path
    }

    /// The structural root role.
    #[allow(dead_code)]
    pub(crate) fn role(&self) -> ModuleRootRole {
        self.role
    }
}

/// Convert a base-relative logical path into a portable forward-slash spelling.
///
/// The empty path yields the empty string (valid for the entry-root module origin); deeper
/// normal components are joined with `/`. Only normal relative components are accepted.
/// `CurDir`, `ParentDir`, `RootDir` and `Prefix` components are rejected through an internal
/// `CompilerError` so two invalid inputs cannot collapse to the same stable identity. Stage 0
/// traversal already rejects non-UTF-8 path components through structured diagnostics, so a
/// non-UTF-8 normal component here is a proven internal invariant; it is still surfaced as an
/// explicit `CompilerError` rather than a panic.
pub(crate) fn portable_relative_logical_path_from(
    relative: &Path,
) -> Result<String, CompilerError> {
    let mut spelling = String::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                let name = name.to_str().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "stable relative logical path component {name:?} in {relative:?} is not \
                        UTF-8; Stage 0 rejects non-UTF-8 path components before identity \
                        construction, so this is an internal invariant violation"
                    ))
                })?;
                if !spelling.is_empty() {
                    spelling.push('/');
                }
                spelling.push_str(name);
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(CompilerError::compiler_error(format!(
                    "stable logical path {relative:?} contains an invalid component \
                     {component:?}; only normal relative components are permitted, so two invalid \
                     inputs cannot collapse to the same stable identity"
                )));
            }
        }
    }
    Ok(spelling)
}

/// Owned, hashable, cross-build identity for one owned supported source file.
///
/// WHAT: derives a stable owned-source identity from the owning module's
/// [`StableModuleOriginIdentity`] and the module-relative logical source file path (portable
/// forward-slash spelling, including the root filename). It stores no `PathBuf`, `StringId`,
/// `InternedPath`, dense `SourceId`, traversal index, `SourceDatabase` or absolute filesystem path,
/// so identity is stable across checkout roots and traversal order. The module-relative source
/// path intentionally includes the actual root filename, so renaming the root file (for example
/// `#page.moth` to `#pages.moth`) changes the owned-source identity even though the module origin
/// remains stable.
/// WHY: later Phase 3 slices consume this identity as the canonical logical source identity for
/// semantic source sets, check-only orphan units and source attribution. Keeping the dense
/// build-local handles and absolute paths out prevents process-local indexes from leaking across
/// module boundaries or into persistent artefacts. Identity changes when the module origin or the
/// module-relative source file path changes, and is otherwise independent of checkout root.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StableOwnedSourceIdentity {
    module_origin: StableModuleOriginIdentity,
    relative_source_path: String,
}

impl StableOwnedSourceIdentity {
    /// Build the cross-build owned-source identity from a module-relative logical source path.
    ///
    /// `relative_source_path` is the path of the source file relative to its owning module root
    /// directory, including the root filename. It is validated by the same portable-component
    /// rules as the module origin path: only normal relative UTF-8 components are accepted, so
    /// `CurDir`, `ParentDir`, `RootDir`, `Prefix` and non-UTF-8 components surface as an internal
    /// `CompilerError` rather than panicking or collapsing two invalid inputs to one identity.
    /// Stage 0 traversal already rejects non-UTF-8 source names through structured diagnostics,
    /// so a non-UTF-8 component here is an internal invariant violation surfaced explicitly.
    /// An owned source identity always includes a filename, so an empty relative source path is
    /// rejected through an internal `CompilerError` rather than collapsing to the entry-root
    /// module's empty path spelling.
    pub(crate) fn from_relative_source_path(
        module_origin: StableModuleOriginIdentity,
        relative_source_path: &Path,
    ) -> Result<Self, CompilerError> {
        if relative_source_path.as_os_str().is_empty() {
            return Err(CompilerError::compiler_error(
                "stable owned-source identity cannot be constructed from an empty relative source \
                 path; an owned source identity always includes a filename",
            ));
        }
        Ok(Self {
            module_origin,
            relative_source_path: portable_relative_logical_path_from(relative_source_path)?,
        })
    }

    /// The owning module origin identity.
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    /// The canonical portable module-relative source file path spelling (forward slashes,
    /// including the root filename).
    #[allow(dead_code)]
    pub(crate) fn relative_source_path(&self) -> &str {
        &self.relative_source_path
    }
}

/// The semantic category of one exported source type that receives a stable origin identity.
///
/// Struct, choice and transparent alias are distinguished so changing a declaration's category
/// changes its origin identity: a struct and a transparent alias with the same defining name in
/// the same module are distinct exported declarations and must not share an [`OriginTypeId`].
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum OriginTypeCategory {
    Struct,
    Choice,
    TransparentAlias,
}

/// Distinguishes a free function from a receiver method for stable function origin identity.
///
/// Receiver methods are not independent namespace exports: they live on their receiver type's
/// surface. A method exported on a receiver surface still needs a stable function origin, so the
/// `Receiver` variant carries the receiver's [`OriginTypeId`] inline. Two methods named `run` on
/// distinct receiver types cannot collapse because the receiver type identity is embedded in the
/// variant. This is the sole authoritative receiver state: a free function with a receiver or a
/// receiver method without a receiver type is unrepresentable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum FunctionOriginKind {
    Free,
    Receiver(OriginTypeId),
}

/// Owned, hashable, cross-build origin identity for one exported source type.
///
/// WHAT: derives a stable type origin from the owning [`StableModuleOriginIdentity`], the exact
/// defining declaration name and the [`OriginTypeCategory`]. It stores no `StringId`,
/// `InternedPath`, `FileId`, source location, ordinary source-file path, declaration order,
/// export alias or dense build-local ID, so identity is stable when source moves between files,
/// reordered or aliased at export time.
/// WHY: cross-module type references and generated instances key off this origin so changing a
/// declaration's name, category or module alters identity while cosmetic moves do not.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OriginTypeId {
    module_origin: StableModuleOriginIdentity,
    defining_name: String,
    category: OriginTypeCategory,
}

impl OriginTypeId {
    /// Construct the stable origin identity for one exported source type.
    ///
    /// Compiler-internal: only declarations admitted to a consumer-visible or exported surface
    /// receive an origin ID. Private source types remain without origin IDs by policy; this
    /// slice does not add identity fields to private headers.
    pub(crate) fn new(
        module_origin: StableModuleOriginIdentity,
        defining_name: String,
        category: OriginTypeCategory,
    ) -> Self {
        Self {
            module_origin,
            defining_name,
            category,
        }
    }

    /// The owning module origin identity.
    #[allow(dead_code)]
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    /// The exact defining declaration name.
    #[allow(dead_code)]
    pub(crate) fn defining_name(&self) -> &str {
        &self.defining_name
    }

    /// The semantic source type category.
    #[allow(dead_code)]
    pub(crate) fn category(&self) -> OriginTypeCategory {
        self.category
    }
}

/// Owned, hashable, cross-build origin identity for one exported function.
///
/// WHAT: derives a stable function origin from the owning [`StableModuleOriginIdentity`], the
/// exact defining declaration name and the sole [`FunctionOriginKind`], which embeds the receiver
/// type identity for a method. It stores no `StringId`, `InternedPath`, `FileId`, source
/// location, ordinary source-file path, declaration order, export alias or dense build-local ID.
/// WHY: identifies stable cross-module source call targets so a free function and a method of the
/// same name are distinct, and renaming a function or moving it between modules alters identity
/// while reordering and aliasing do not. Binding-backed external call identities are a separate
/// target class owned by a later phase, not this origin.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OriginFunctionId {
    module_origin: StableModuleOriginIdentity,
    defining_name: String,
    kind: FunctionOriginKind,
}

impl OriginFunctionId {
    /// Construct the stable origin identity for one exported free function.
    ///
    /// Compiler-internal: only declarations admitted to a consumer-visible or exported surface
    /// receive an origin ID. Private functions remain without origin IDs by policy.
    pub(crate) fn new_free(
        module_origin: StableModuleOriginIdentity,
        defining_name: String,
    ) -> Self {
        Self {
            module_origin,
            defining_name,
            kind: FunctionOriginKind::Free,
        }
    }

    /// Construct the stable origin identity for one exported receiver method.
    ///
    /// `receiver` is the stable [`OriginTypeId`] of the receiver type. Compiler-internal: only
    /// methods admitted to a receiver surface that is itself consumer-visible or exported receive
    /// an origin ID. A method is not an independent namespace export, but its exported receiver
    /// surface still needs a stable function origin.
    pub(crate) fn new_receiver(
        module_origin: StableModuleOriginIdentity,
        defining_name: String,
        receiver: OriginTypeId,
    ) -> Self {
        Self {
            module_origin,
            defining_name,
            kind: FunctionOriginKind::Receiver(receiver),
        }
    }

    /// The owning module origin identity.
    #[allow(dead_code)]
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    /// The exact defining declaration name.
    pub(crate) fn defining_name(&self) -> &str {
        &self.defining_name
    }

    /// Whether this origin is a free function or a receiver method.
    ///
    /// Returns the sole authoritative state: `Free` or `Receiver(OriginTypeId)`. The receiver
    /// type identity for a method is embedded in the variant, not stored in a parallel field.
    pub(crate) fn kind(&self) -> &FunctionOriginKind {
        &self.kind
    }

    /// The stable type origin of the receiver for a method, or `None` for a free function.
    pub(crate) fn receiver(&self) -> Option<&OriginTypeId> {
        match &self.kind {
            FunctionOriginKind::Free => None,
            FunctionOriginKind::Receiver(receiver) => Some(receiver),
        }
    }
}

/// Owned, hashable, cross-build origin identity for one exported constant.
///
/// WHAT: derives a stable constant origin from the owning [`StableModuleOriginIdentity`] and the
/// exact defining declaration name. It stores no `StringId`, `InternedPath`, `FileId`, source
/// location, ordinary source-file path, declaration order, export alias or dense build-local ID.
/// WHY: cross-module constant references key off this origin so renaming a constant or moving it
/// between modules alters identity while reordering and aliasing do not.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OriginConstantId {
    module_origin: StableModuleOriginIdentity,
    defining_name: String,
}

impl OriginConstantId {
    /// Construct the stable origin identity for one exported constant.
    ///
    /// Compiler-internal: only declarations admitted to a consumer-visible or exported surface
    /// receive an origin ID. Private constants remain without origin IDs by policy.
    pub(crate) fn new(module_origin: StableModuleOriginIdentity, defining_name: String) -> Self {
        Self {
            module_origin,
            defining_name,
        }
    }

    /// The owning module origin identity.
    #[allow(dead_code)]
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    /// The exact defining declaration name.
    #[allow(dead_code)]
    pub(crate) fn defining_name(&self) -> &str {
        &self.defining_name
    }
}

/// Owned, hashable, cross-build origin identity for one exported trait.
///
/// WHAT: derives a stable trait origin from the owning [`StableModuleOriginIdentity`] and the
/// exact defining declaration name. It stores no `StringId`, `InternedPath`, `FileId`, source
/// location, ordinary source-file path, declaration order, export alias or dense build-local ID.
/// WHY: cross-module conformance evidence and trait references key off this origin so renaming a
/// trait or moving it between modules alters identity while reordering and aliasing do not.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OriginTraitId {
    module_origin: StableModuleOriginIdentity,
    defining_name: String,
}

impl OriginTraitId {
    /// Construct the stable origin identity for one exported trait.
    ///
    /// Compiler-internal: only declarations admitted to a consumer-visible or exported surface
    /// receive an origin ID. Private traits remain without origin IDs by policy.
    pub(crate) fn new(module_origin: StableModuleOriginIdentity, defining_name: String) -> Self {
        Self {
            module_origin,
            defining_name,
        }
    }

    /// The owning module origin identity.
    #[allow(dead_code)]
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    /// The exact defining declaration name.
    #[allow(dead_code)]
    pub(crate) fn defining_name(&self) -> &str {
        &self.defining_name
    }
}

/// Unified stable origin identity for one exported declaration, regardless of category.
///
/// WHAT: a typed sum over the exported declaration origin IDs. It preserves the typed category
/// so a type, function, constant and trait with the same defining name in the same module remain
/// distinct, and so later phases can dispatch over exported declarations without re-parsing.
/// WHY: public-interface binding and exported-symbol tables key over one stable identity value
/// while still distinguishing declaration category. Identity still derives only from module
/// origin, defining name, category and receiver type identity where applicable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OriginDeclarationId {
    Function(OriginFunctionId),
    Type(OriginTypeId),
    Constant(OriginConstantId),
    Trait(OriginTraitId),
}

impl OriginDeclarationId {
    /// The owning module origin identity for this declaration.
    #[allow(dead_code)]
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        match self {
            OriginDeclarationId::Function(function) => function.module_origin(),
            OriginDeclarationId::Type(type_id) => type_id.module_origin(),
            OriginDeclarationId::Constant(constant) => constant.module_origin(),
            OriginDeclarationId::Trait(trait_id) => trait_id.module_origin(),
        }
    }

    /// The exact defining declaration name.
    ///
    /// WHAT: the authored name of the declaration, independent of any export alias. For a
    /// directly-defined export this equals the public name; for a re-export with an alias it
    /// differs from the public name.
    pub(crate) fn defining_name(&self) -> &str {
        match self {
            OriginDeclarationId::Function(function) => function.defining_name(),
            OriginDeclarationId::Type(type_id) => type_id.defining_name(),
            OriginDeclarationId::Constant(constant) => constant.defining_name(),
            OriginDeclarationId::Trait(trait_id) => trait_id.defining_name(),
        }
    }
}

/// One stable export binding for a public declaration defined directly in the active module root.
///
/// WHAT: maps the exporting module's public API name to the stable origin declaration identity
///       for a declaration authored directly in the active module root's public surface. The
///       exporting module is the stable [`StableModuleOriginIdentity`], never a build-local dense
///       `ModuleId`. The public name and the origin's defining name are self-contained owned
///       strings, never `StringId` or `InternedPath` values that would not survive the graph
///       table. For a directly defined export the public name equals the defining name; re-export
///       aliasing is a separate fact owned by the future completed provider interface.
/// WHY: the compiler design overview's `ExportBinding` contract requires public-interface binding
///      to key over stable module origins and declaration identities. Recording these bindings at
///      the semantic compilation boundary lets later phases consume them without reparsing source
///      or re-deriving identity from donor-local paths.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ExportBinding {
    exporting_module: StableModuleOriginIdentity,
    public_name: String,
    origin: OriginDeclarationId,
}

impl ExportBinding {
    /// Construct one stable export binding for a directly defined public declaration.
    ///
    /// Compiler-internal: the construction owner in `public_interface::export_projection`
    /// builds these only for declarations admitted to the active module root's public surface,
    /// so the exporting module, public name and origin identity are already known to be consistent.
    pub(crate) fn new(
        exporting_module: StableModuleOriginIdentity,
        public_name: String,
        origin: OriginDeclarationId,
    ) -> Self {
        Self {
            exporting_module,
            public_name,
            origin,
        }
    }

    /// The stable identity of the module exporting the declaration.
    #[allow(dead_code)]
    pub(crate) fn exporting_module(&self) -> &StableModuleOriginIdentity {
        &self.exporting_module
    }

    /// The public API name the exporting module exposes the declaration under.
    pub(crate) fn public_name(&self) -> &str {
        &self.public_name
    }

    /// The stable origin identity of the exported declaration.
    pub(crate) fn origin(&self) -> &OriginDeclarationId {
        &self.origin
    }
}
