//! Builder surface definition.
//!
//! WHAT: bundles the external packages and source-backed packages that a builder exposes,
//! along with config keys, import providers and source file kinds.
//! WHY: builders provide both binding-backed package metadata and source-backed package roots;
//!      the compiler needs both during different stages.

use crate::builder_surface::config_key_registry::ProjectConfigKeyRegistry;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::source_file_kind_registry::SourceFileKindRegistry;
use crate::builder_surface::source_package_registry::SourcePackageRegistry;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The complete builder surface a backend exposes to a project.
///
/// WHAT: collects every package kind the frontend and backends need.
/// WHY: one unified builder return type instead of separate APIs for binding-backed
///      packages and source-backed packages.
#[derive(Clone, Debug)]
pub struct BuilderSurface {
    pub binding_packages: ExternalPackageRegistry,
    pub source_packages: SourcePackageRegistry,
    pub config_keys: ProjectConfigKeyRegistry,
    pub external_import_providers: ExternalImportProviderRegistry,
    pub external_import_cache: ExternalImportProviderCache,
    pub external_import_resolution_table: ExternalImportResolutionTable,
    pub builder_runtime_packages: Vec<BuilderRuntimePackageMetadata>,
    pub source_file_kinds: SourceFileKindRegistry,
    /// Source-backed package prefixes whose constants are implicitly visible in `.mtf` files.
    ///
    /// WHAT: records a builder capability rather than making generic build orchestration infer
    ///       implicit template providers from package names.
    /// WHY: only the active builder owns the contract that connects a source package to the
    ///      synthetic `.mtf` constant scope.
    pub implicit_template_scope_source_packages: BTreeSet<String>,
}

const BUILTIN_SOURCE_PACKAGES_DIR: &str = "packages";
const SUPPORTED_PROJECT_CONFIG_VALUES: &[&str] = &["html"];

impl BuilderSurface {
    /// Builds a builder surface with mandatory compiler core packages and no source-backed packages.
    ///
    /// WHAT: the minimal default every builder starts from: prelude, core IO namespace, compiler-owned
    /// collection helpers, and error helpers.
    /// WHY: user-facing optional core packages such as `@core/math` and `@core/text`
    /// must be explicit builder opt-ins.
    pub fn with_mandatory_core() -> Self {
        let mut config_keys = ProjectConfigKeyRegistry::new();

        // Core config keys are always accepted regardless of backend.
        config_keys.register_core_closed_string_set("project", SUPPORTED_PROJECT_CONFIG_VALUES);
        config_keys.register_core_string("entry_root");
        config_keys.register_core_string("dev_folder");
        config_keys.register_core_string("output_folder");
        config_keys.register_core_int("template_const_loop_iteration_limit");
        config_keys.register_core_string_collection("package_folders");
        config_keys.register_core_string("name");
        config_keys.register_core_string("project_name");
        config_keys.register_core_string("version");
        config_keys.register_core_string("author");
        config_keys.register_core_string("license");

        Self {
            binding_packages: ExternalPackageRegistry::new(),
            source_packages: SourcePackageRegistry::default(),
            config_keys,
            external_import_providers: ExternalImportProviderRegistry::empty(),
            external_import_cache: ExternalImportProviderCache::new(),
            external_import_resolution_table: ExternalImportResolutionTable::new(),
            builder_runtime_packages: Vec::new(),
            source_file_kinds: SourceFileKindRegistry::new(),
            implicit_template_scope_source_packages: BTreeSet::new(),
        }
    }

    /// Declare a source-backed package whose exported constants enter `.mtf` implicit scope.
    pub fn register_implicit_template_scope_source_package(
        &mut self,
        import_prefix: impl Into<String>,
    ) {
        self.implicit_template_scope_source_packages
            .insert(import_prefix.into());
    }

    /// Exposes the currently supported optional core packages for the HTML builder.
    ///
    /// WHAT: registers binding-backed core packages selected by the HTML builder.
    /// WHY: optional core packages are builder surface; they should not be assumed by
    /// the compiler's mandatory registry.
    pub fn expose_html_core_packages(&mut self) {
        crate::builder_surface::core_packages::register_core_math_package(
            &mut self.binding_packages,
        );
        crate::builder_surface::core_packages::register_core_text_package(
            &mut self.binding_packages,
        );
        crate::builder_surface::core_packages::register_core_random_package(
            &mut self.binding_packages,
        );
        crate::builder_surface::core_packages::register_core_time_package(
            &mut self.binding_packages,
        );
    }

    pub fn builtin_source_package_root(prefix: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(BUILTIN_SOURCE_PACKAGES_DIR)
            .join(prefix)
    }
}
