//! Builder surface definition.
//!
//! WHAT: bundles the external packages and source-backed packages that a builder exposes,
//! along with config schemas, import providers and source file kinds.
//! WHY: builders provide both binding-backed package metadata and source-backed package roots;
//!      the compiler needs both during different stages.

use crate::builder_surface::SourcePackageRegistry;
use crate::builder_surface::config_schema::{
    ConfigFieldShape, ConfigSchema, ConfigSchemaField, ConfigSchemas, UnknownFieldPolicy,
};
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::source_file_kind_registry::SourceFileKindRegistry;
use crate::compiler_frontend::build_config::{
    BuildInputName, BuilderConfigGlobalError, BuilderConfigGlobalSet, PrimitiveBuildValue,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The complete builder surface a backend exposes to a project.
///
/// WHAT: collects every package kind the frontend and backends need.
/// WHY: one unified builder return type instead of separate APIs for binding-backed
#[derive(Clone, Debug)]
pub struct BuilderSurface {
    pub binding_packages: ExternalPackageRegistry,
    pub source_packages: SourcePackageRegistry,
    pub config_schemas: ConfigSchemas,
    /// Typed, platform-neutral primitive globals available during config folding.
    config_globals: BuilderConfigGlobalSet,
    pub external_import_providers: ExternalImportProviderRegistry,
    pub external_import_cache: ExternalImportProviderCache,
    pub external_dependency_resolution_table: ExternalImportResolutionTable,
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

impl BuilderSurface {
    /// Builds a builder surface with mandatory compiler core packages and no source-backed packages.
    ///
    /// WHAT: the minimal default every builder starts from: prelude, core IO namespace, compiler-owned
    /// collection helpers, and error helpers.
    /// WHY: user-facing optional core packages such as `@core/math` and `@core/text`
    /// must be explicit builder opt-ins.
    pub fn with_mandatory_core() -> Self {
        Self {
            binding_packages: ExternalPackageRegistry::new(),
            source_packages: SourcePackageRegistry::default(),
            config_schemas: ConfigSchemas::new(compiler_project_record_schema()),
            config_globals: BuilderConfigGlobalSet::new(),
            external_import_providers: ExternalImportProviderRegistry::empty(),
            external_import_cache: ExternalImportProviderCache::new(),
            external_dependency_resolution_table: ExternalImportResolutionTable::new(),
            builder_runtime_packages: Vec::new(),
            source_file_kinds: SourceFileKindRegistry::new(),
            implicit_template_scope_source_packages: BTreeSet::new(),
        }
    }

    /// Seals config schemas after the selected builder and tooling overlays finish registering.
    pub fn finish_config_registration(&mut self) {
        self.config_schemas
            .validate()
            .expect("builder config schemas are valid");
    }

    /// Declare a source-backed package whose exported constants enter `.mtf` implicit scope.
    pub fn register_implicit_template_scope_source_package(
        &mut self,
        package_prefix: impl Into<String>,
    ) {
        self.implicit_template_scope_source_packages
            .insert(package_prefix.into());
    }
    /// Typed primitive globals this surface contributes while compiler config constants fold.
    pub(crate) fn config_globals(&self) -> &BuilderConfigGlobalSet {
        &self.config_globals
    }

    /// Register one typed, platform-neutral builder global.
    ///
    /// The underlying carrier rejects backend and platform identity names before they can enter
    /// the builder surface.
    #[allow(dead_code)]
    pub(crate) fn register_config_global(
        &mut self,
        name: BuildInputName,
        value: PrimitiveBuildValue,
    ) -> Result<Option<PrimitiveBuildValue>, BuilderConfigGlobalError> {
        self.config_globals.insert(name, value)
    }

    pub fn builtin_source_package_root(prefix: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(BUILTIN_SOURCE_PACKAGES_DIR)
            .join(prefix)
    }
}

/// The grouped `project #= |...|` record schema with compiler-owned fields only.
///
/// WHAT: `name` is required; `entry_root` defaults to `src`; remaining compiler-owned
///       project fields stay optional. Additional authored metadata is preserved.
/// WHY: compiler-owned project fields must stay off builder section schemas, and open
///      metadata must survive for `@project`.
fn compiler_project_record_schema() -> ConfigSchema {
    let mut schema = ConfigSchema::new("project record", UnknownFieldPolicy::Preserve);
    let root = schema.root();

    schema
        .register_field(root, ConfigSchemaField::string("name").required())
        .expect("project schema is under construction");
    schema
        .register_field(
            root,
            ConfigSchemaField::string_with_default("entry_root", "src"),
        )
        .expect("project schema is under construction");
    schema
        .register_field(
            root,
            ConfigSchemaField::optional("version", ConfigFieldShape::String).configurable(),
        )
        .expect("project schema is under construction");
    schema
        .register_field(
            root,
            ConfigSchemaField::optional("author", ConfigFieldShape::String).configurable(),
        )
        .expect("project schema is under construction");
    schema
        .register_field(
            root,
            ConfigSchemaField::optional("license", ConfigFieldShape::String).configurable(),
        )
        .expect("project schema is under construction");
    // Keep the template-loop limit fixed-only: it controls compiler expansion work rather than
    // ordinary project metadata, so configuration must not alter compilation topology/resources.
    schema
        .register_field(
            root,
            ConfigSchemaField::int("template_const_loop_iteration_limit"),
        )
        .expect("project schema is under construction");
    schema
        .validate_and_freeze()
        .expect("project schema is valid");
    schema
}
