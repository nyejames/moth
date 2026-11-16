//! Shared package metadata: origin and backing axes.
//!
//! WHAT: defines `PackageOrigin` and `PackageBacking` as orthogonal metadata for
//! any package the compiler encounters, plus `PackageMetadata` which bundles them.
//! WHY: origin (who owns/distributes the package) and backing (how the compiler
//! obtains its implementation) are independent concerns. Keeping them separate
//! avoids conflating builder-owned source packages with binding-backed core packages.

/// Who owns or distributes a package.
///
/// WHAT: classifies packages by their provider relationship.
/// WHY: diagnostics and availability checks need to distinguish core, builder,
/// project-local and dependency packages without inspecting dependency paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PackageOrigin {
    /// Compiler-owned core packages (e.g. `@core/io`, `@core/html`, `@core/css`).
    Core,
    /// Builder-owned packages (e.g. `@builder/web/site`, `@builder/web/canvas`).
    Builder,
    /// Packages local to the current project.
    ProjectLocal,
    /// Reserved for future external dependency packages.
    Dependency,
}

/// How the compiler obtains a package's implementation.
///
/// WHAT: distinguishes Moth-source packages from binding-backed packages, and separates a
/// package implemented by native Rust host functions from one a backend implements itself.
/// WHY: source-backed packages contain Moth modules resolved through
/// `SourcePackageRegistry`; binding-backed packages expose typed external
/// symbols resolved through `ExternalPackageRegistry`. Host requirement production names only
/// `HostBinding` packages as native host imports, because a backend lowers or provides every
/// other binding itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageBacking {
    /// Package backed by Moth source files.
    MothSource,
    /// Package backed by backend-provided external bindings (JS, Wasm or a provider).
    ExternalBinding,
    /// Package backed by native Rust host functions registered through the builder SDK.
    HostBinding,
}

/// Origin and backing metadata for one package.
///
/// WHAT: bundles the two orthogonal package axes so every registration site
/// carries both pieces of information.
/// WHY: callers should not be able to construct invalid combinations such as
/// a `MothSource` backing inside `ExternalPackageRegistry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackageMetadata {
    pub origin: PackageOrigin,
    pub backing: PackageBacking,
}

impl PackageMetadata {
    /// Metadata for a source-backed package.
    pub const fn source(origin: PackageOrigin) -> Self {
        Self {
            origin,
            backing: PackageBacking::MothSource,
        }
    }

    /// Metadata for a binding-backed package.
    pub const fn binding(origin: PackageOrigin) -> Self {
        Self {
            origin,
            backing: PackageBacking::ExternalBinding,
        }
    }

    /// Metadata for a binding-backed package implemented by native Rust host functions.
    pub const fn host_binding(origin: PackageOrigin) -> Self {
        Self {
            origin,
            backing: PackageBacking::HostBinding,
        }
    }
}

/// Namespace roots owned by the Core and Builder package origins.
///
/// WHAT: the first path components that only the matching origin may register below.
/// WHY: `@core` and `@builder` group compiler-shipped and builder-owned packages. Neither root is
/// an importable package, and acknowledging a `core` or `builder` first component for any other
/// origin would let a builder or a local package impersonate either namespace.
const RESERVED_PACKAGE_NAMESPACE_ROOTS: [&str; 2] = ["core", "builder"];

/// Whether one `@`-stripped first path component names a reserved package namespace root.
pub(crate) fn is_reserved_package_namespace_root(component: &str) -> bool {
    declared_reserved_root(component).is_some()
}

/// The declared spelling of the reserved root named by one authored component.
fn declared_reserved_root(component: &str) -> Option<&'static str> {
    RESERVED_PACKAGE_NAMESPACE_ROOTS
        .iter()
        .copied()
        .find(|root| component.eq_ignore_ascii_case(root))
}

/// The reserved root claimed by one importable namespace key, if any.
///
/// WHAT: checks the first `/`-separated component of an extensionless namespace key, ignoring
/// ASCII case, and returns the authored component.
/// WHY: every owner that decides whether a local name is importable shares this one root
/// predicate. Directory compilation and selected module-root entries decide importability from
/// Stage 0 module namespaces, so only their namespace entries reach this check. An ordinary
/// selected file has no module topology, and its resolver joins every entry-root-relative path,
/// so each recognized source below `core/` or `builder/` is an importable claim there. Callers
/// pass only names that can actually be resolved, never provider-owned asset paths.
pub(crate) fn reserved_package_namespace_claim_root(namespace_key: &str) -> Option<&str> {
    namespace_key
        .split('/')
        .next()
        .filter(|component| is_reserved_package_namespace_root(component))
}

/// Validates a registered package path, given without its leading `@` introducer.
///
/// WHAT: one rule for source-backed and binding-backed registration. Components are nonempty and
/// contain no `#`. `project` is reserved for project globals. Core packages register below
/// `core/`, Builder packages below `builder/`, and the other origins may use any other root. Both
/// reserved roots must keep at least one name component below them.
/// WHY: native host import names join the canonical package path and symbol path with `#`, and
/// official Core certification must never follow from a builder choosing a `@core` path. Keeping
/// the origin/prefix rule here stops either registry from growing a second, weaker copy.
pub(crate) fn validate_package_path(
    path_without_introducer: &str,
    origin: PackageOrigin,
) -> Result<(), &'static str> {
    let mut components = path_without_introducer.split('/');
    if components
        .clone()
        .any(|component| component.is_empty() || component.contains('#'))
    {
        return Err("must have nonempty '/'-separated components without '#'");
    }
    let root = components.next().unwrap_or_default();
    if crate::compiler_frontend::project_globals::claims_project_globals_name(root) {
        return Err(
            "is reserved for the @project project-globals interface; choose a package path outside '@project'",
        );
    }

    let reserved_root = declared_reserved_root(root);
    if reserved_root.is_some() && components.next().is_none() {
        return Err(
            "is a reserved namespace root, not an importable package; register a package below it",
        );
    }

    match origin {
        // Core and Builder spellings stay canonical: only the exact lowercase root introduces
        // their packages, so a case variant cannot create a second registry spelling.
        PackageOrigin::Core if root != "core" => {
            Err("must register below the 'core' namespace root")
        }
        PackageOrigin::Builder if root != "builder" => match reserved_root {
            Some("core") => {
                Err("is reserved for official Core packages; choose a package path outside '@core'")
            }
            _ => Err("must register below the 'builder' namespace root"),
        },
        PackageOrigin::ProjectLocal | PackageOrigin::Dependency => match reserved_root {
            Some("core") => {
                Err("is reserved for official Core packages; choose a package path outside '@core'")
            }
            Some(_) => Err(
                "is reserved for builder-owned packages; choose a package path outside '@builder'",
            ),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Whether one `@`-stripped package path equals the other or nests inside it at a `/` boundary,
/// ignoring ASCII case.
///
/// WHY: an import path below an overlapping registration or local namespace would have two
/// owners, so registrations and namespace import collisions share this one overlap rule.
pub(crate) fn package_paths_overlap(left: &str, right: &str) -> bool {
    let (shorter, longer) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    longer
        .get(..shorter.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(shorter))
        && longer[shorter.len()..]
            .chars()
            .next()
            .is_none_or(|next| next == '/')
}
