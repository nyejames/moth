//! HTML resource output placement and conflict preflight.
//!
//! WHAT: turns live stable resource origins into builder-owned output records with explicit URL
//!       contexts, while validating their destinations against page and backend outputs.
//! WHY: resource identity, physical source, output placement and rendered URL are separate facts.
//!      Keeping this planner byte-free lets every conflict fail before a provider or resource
//!      reader is called.
//!
//! This module deliberately does not read or hash resource bytes, render structural strings or
//! create output files. The later emission phase consumes [`PlannedResourceOutput`] records.

use crate::build_system::build::ProjectEntry;
use crate::build_system::output::{OutputPathIdentity, output_path_identity};
use crate::build_system::resource_unions::ResourceOriginUnion;
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::paths::resource_identity::{
    StableResourceOriginId, StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::html_project::diagnostics::{
    resource_output_path_collision_messages, resource_output_path_reserved_messages,
};
use crate::projects::html_project::page_metadata::{HtmlPageMetadataPlan, MetadataResourceUse};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
/// The artefact whose URL rules observe one resource-bearing string.
///
/// The current HTML entry pipeline has one explicit page-document context. Stylesheet contexts are
/// represented now so a later standalone CSS lane can use the same record shape without treating a
/// stylesheet as a JavaScript or Wasm container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceUrlContext {
    PageDocument(PathBuf),
    Stylesheet(PathBuf),
}

impl ResourceUrlContext {
    fn page_document(path: &Path) -> Result<Self, CompilerError> {
        validate_output_path(path, "page document")?;
        Ok(Self::PageDocument(path.to_path_buf()))
    }

    #[allow(dead_code)]
    pub(crate) fn stylesheet(path: &Path) -> Result<Self, CompilerError> {
        validate_output_path(path, "stylesheet")?;
        Ok(Self::Stylesheet(path.to_path_buf()))
    }

    #[allow(dead_code)]
    pub(crate) fn artefact_path(&self) -> &Path {
        match self {
            Self::PageDocument(path) | Self::Stylesheet(path) => path,
        }
    }
}

/// One reachable use of a planned resource output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedResourceUse {
    pub(crate) context: ResourceUrlContext,
    pub(crate) authored_location: SourceLocation,
}

/// One deduplicated resource output record.
///
/// The record contains no bytes and no content hash. One origin may carry several uses when
/// entries observe the same emitted output from different page contexts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedResourceOutput {
    pub(crate) origin: StableResourceOriginId,
    pub(crate) output_path: PathBuf,
    pub(crate) first_authored_location: SourceLocation,
    pub(crate) uses: Vec<PlannedResourceUse>,
    has_live_use: bool,
}

/// Authored locations for one origin, with intern-table fallback last.
#[derive(Clone, Debug, Default)]
pub(crate) struct OriginAuthoredLocations {
    pub live: Vec<SourceLocation>,
    pub non_live: Vec<SourceLocation>,
    pub fallback: Option<SourceLocation>,
}

#[derive(Clone, Copy)]
enum PlannedResourceUseKind {
    Live,
    NonLive,
    Fallback,
}

/// Byte-free HTML resource plan accumulated across all selected entries.
#[derive(Debug, Default)]
pub(crate) struct HtmlResourceOutputPlan {
    project_name: String,
    records: Vec<PlannedResourceOutput>,
    record_by_output: HashMap<OutputPathIdentity, usize>,
    builder_output_paths: HashMap<OutputPathIdentity, StringId>,
}

impl HtmlResourceOutputPlan {
    /// Start a plan for one configured project package.
    pub(crate) fn new(project_name: &str) -> Self {
        Self {
            project_name: project_name.to_owned(),
            ..Self::default()
        }
    }

    /// Plan the live resource union for one HTML entry before lowering that entry.
    ///
    /// Every current HTML resource use observes the page document. A context is created and
    /// validated before any backend lowering, so an invalid or unsupported page artefact cannot
    /// reach a lowerer with an unresolved URL policy.
    pub(crate) fn plan_entry(
        &mut self,
        entry: &ProjectEntry<'_>,
        page_output_path: &Path,
        page_metadata_plan: &HtmlPageMetadataPlan,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let context = ResourceUrlContext::page_document(page_output_path)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        self.reserve_builder_output_path(page_output_path, "HTML page", string_table)?;

        let locations = first_authored_locations(entry)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        self.plan_union(
            entry.resource_union,
            &locations,
            context.clone(),
            string_table,
        )?;
        self.plan_page_metadata_uses(&page_metadata_plan.resource_uses, context, string_table)
    }

    /// Reserve a known HTML, JavaScript, Wasm, CSS or manifest destination.
    ///
    /// Duplicate destinations owned by the same builder lane are left to the existing builder
    /// duplicate checks. A resource destination, however, is rejected immediately so this method
    /// can be called before a byte-reading emitter.
    pub(crate) fn reserve_builder_output_path(
        &mut self,
        output_path: &Path,
        artefact_kind: &str,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let identity = validate_output_path(output_path, artefact_kind)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        if let Some(&record_index) = self.record_by_output.get(&identity) {
            let record = &self.records[record_index];
            return Err(resource_output_path_reserved_messages(
                output_path,
                &display_origin(&record.origin),
                artefact_kind,
                &record.first_authored_location,
                string_table,
            ));
        }

        let artefact_kind_id = string_table.intern(artefact_kind);
        self.builder_output_paths
            .entry(identity)
            .or_insert(artefact_kind_id);
        Ok(())
    }

    /// Add one origin and one explicit context, primarily for focused planner tests and future
    /// builders whose link-plan owner already has the first authored location.
    pub(crate) fn plan_origin(
        &mut self,
        origin: StableResourceOriginId,
        first_authored_location: SourceLocation,
        context: ResourceUrlContext,
        string_table: &mut StringTable,
        is_live_use: bool,
    ) -> Result<(), CompilerMessages> {
        let output_path = self
            .output_path_for_origin(&origin)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        let use_kind = if is_live_use {
            PlannedResourceUseKind::Live
        } else {
            PlannedResourceUseKind::NonLive
        };
        self.plan_one_origin(
            origin,
            output_path,
            first_authored_location,
            context,
            string_table,
            use_kind,
        )
    }

    fn plan_page_metadata_uses(
        &mut self,
        resource_uses: &[MetadataResourceUse],
        context: ResourceUrlContext,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for resource_use in resource_uses {
            self.plan_origin(
                resource_use.origin.clone(),
                resource_use.authored_location.clone(),
                context.clone(),
                string_table,
                false,
            )?;
        }
        Ok(())
    }

    pub(crate) fn records(&self) -> &[PlannedResourceOutput] {
        &self.records
    }

    /// Consume the plan into its records for a writer-owned handoff.
    pub(crate) fn into_records(self) -> Vec<PlannedResourceOutput> {
        self.records
    }

    fn plan_union(
        &mut self,
        union: &ResourceOriginUnion,
        locations: &HashMap<StableResourceOriginId, OriginAuthoredLocations>,
        context: ResourceUrlContext,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for origin in union.iter() {
            let Some(authored_locations) = locations.get(origin) else {
                let error = CompilerError::compiler_error(format!(
                    "HTML resource origin {origin:?} has no first authored location in its linked module resource tables"
                ));
                return Err(CompilerMessages::from_error_ref(error, string_table));
            };

            let output_path = self
                .output_path_for_origin(origin)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

            for authored_location in &authored_locations.live {
                self.plan_one_origin(
                    origin.clone(),
                    output_path.clone(),
                    authored_location.clone(),
                    context.clone(),
                    string_table,
                    PlannedResourceUseKind::Live,
                )?;
            }

            for authored_location in &authored_locations.non_live {
                self.plan_one_origin(
                    origin.clone(),
                    output_path.clone(),
                    authored_location.clone(),
                    context.clone(),
                    string_table,
                    PlannedResourceUseKind::NonLive,
                )?;
            }

            if authored_locations.live.is_empty() && authored_locations.non_live.is_empty() {
                if let Some(fallback) = &authored_locations.fallback {
                    self.plan_one_origin(
                        origin.clone(),
                        output_path,
                        fallback.clone(),
                        context.clone(),
                        string_table,
                        PlannedResourceUseKind::Fallback,
                    )?;
                } else {
                    let error = CompilerError::compiler_error(format!(
                        "HTML resource origin {origin:?} has no first authored location in its linked module resource tables"
                    ));
                    return Err(CompilerMessages::from_error_ref(error, string_table));
                }
            }
        }

        Ok(())
    }

    fn plan_one_origin(
        &mut self,
        origin: StableResourceOriginId,
        output_path: PathBuf,
        first_authored_location: SourceLocation,
        context: ResourceUrlContext,
        string_table: &mut StringTable,
        use_kind: PlannedResourceUseKind,
    ) -> Result<(), CompilerMessages> {
        let output_identity = validate_output_path(&output_path, "resource")
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        if let Some(&record_index) = self.record_by_output.get(&output_identity) {
            let record = &mut self.records[record_index];
            if record.origin != origin {
                return Err(resource_output_path_collision_messages(
                    &output_path,
                    &display_origin(&record.origin),
                    &record.first_authored_location,
                    &display_origin(&origin),
                    &first_authored_location,
                    string_table,
                ));
            }

            match use_kind {
                PlannedResourceUseKind::Live if !record.has_live_use => {
                    record.first_authored_location = first_authored_location.clone();
                    record.has_live_use = true;
                }
                PlannedResourceUseKind::NonLive
                    if !record.has_live_use && record.uses.is_empty() =>
                {
                    record.first_authored_location = first_authored_location.clone();
                }
                PlannedResourceUseKind::Live
                | PlannedResourceUseKind::NonLive
                | PlannedResourceUseKind::Fallback => {}
            }

            if !matches!(use_kind, PlannedResourceUseKind::Fallback) {
                let use_record = PlannedResourceUse {
                    context,
                    authored_location: first_authored_location,
                };
                if !record.uses.contains(&use_record) {
                    record.uses.push(use_record);
                }
            }
            return Ok(());
        }

        if let Some(&artefact_kind_id) = self.builder_output_paths.get(&output_identity) {
            let artefact_kind = string_table.resolve(artefact_kind_id).to_owned();
            return Err(resource_output_path_reserved_messages(
                &output_path,
                &display_origin(&origin),
                &artefact_kind,
                &first_authored_location,
                string_table,
            ));
        }

        let record_index = self.records.len();
        let uses = if matches!(use_kind, PlannedResourceUseKind::Fallback) {
            Vec::new()
        } else {
            vec![PlannedResourceUse {
                context,
                authored_location: first_authored_location.clone(),
            }]
        };
        self.records.push(PlannedResourceOutput {
            origin,
            output_path,
            first_authored_location,
            uses,
            has_live_use: matches!(use_kind, PlannedResourceUseKind::Live),
        });
        self.record_by_output.insert(output_identity, record_index);
        Ok(())
    }

    fn output_path_for_origin(
        &self,
        origin: &StableResourceOriginId,
    ) -> Result<PathBuf, CompilerError> {
        let output_path = match origin.owner() {
            StableResourceOwnerId::Module(module_origin) => {
                let package = module_origin.package();
                let package_relative_path = package_relative_path(
                    module_origin.logical_module_path(),
                    origin.logical_path().as_str(),
                );

                if package.origin() == PackageOrigin::ProjectLocal
                    && package.name() == self.project_name
                {
                    package_relative_path
                } else {
                    package_output_prefix(package).join(package_relative_path)
                }
            }
            StableResourceOwnerId::Provider(_) => {
                // A provider's logical path is its declared stable output path. Provider metadata
                // is deliberately not converted into a guessed package-relative filesystem path.
                PathBuf::from(origin.logical_path().as_str())
            }
        };

        validate_output_path(&output_path, "resource")?;
        Ok(output_path)
    }
}

fn first_authored_locations(
    entry: &ProjectEntry<'_>,
) -> Result<HashMap<StableResourceOriginId, OriginAuthoredLocations>, CompilerError> {
    let mut locations = HashMap::new();
    let mut module_views = Vec::with_capacity(1 + entry.linked_modules.len());
    module_views.push((entry.module, entry.reachability));
    module_views.extend(
        entry
            .linked_modules
            .iter()
            .map(|linked| (linked.module, linked.reachability)),
    );

    for (module, reachability) in &module_views {
        record_reachable_resource_locations(&mut locations, module, reachability)?;
    }

    // Only the selected entry root contributes compile-time fragments to its entry union.
    record_const_fragment_resource_locations(&mut locations, entry.module);

    for (module, _) in module_views {
        for resource in module.executable.resource_table.origins() {
            locations
                .entry(resource.origin.clone())
                .or_insert_with(|| OriginAuthoredLocations {
                    live: Vec::new(),
                    non_live: Vec::new(),
                    fallback: Some(resource.first_authored_location.clone()),
                });
        }
    }

    Ok(locations)
}

fn record_reachable_resource_locations(
    locations: &mut HashMap<StableResourceOriginId, OriginAuthoredLocations>,
    module: &Module,
    reachability: &HirReachability,
) -> Result<(), CompilerError> {
    for resource_use in &reachability.reachable_resource_uses {
        let resource = module
            .executable
            .resource_table
            .try_origin(resource_use.resource_id)?;
        locations
            .entry(resource.origin.clone())
            .or_default()
            .live
            .push(resource_use.location.clone());
    }

    Ok(())
}

fn record_const_fragment_resource_locations(
    locations: &mut HashMap<StableResourceOriginId, OriginAuthoredLocations>,
    module: &Module,
) {
    for fragment in &module.metadata.const_top_level_fragments {
        let OwnedFoldedString::Pieces(pieces) = &fragment.value else {
            continue;
        };

        for piece in pieces {
            if let OwnedFoldedStringPiece::Resource(origin) = piece {
                locations
                    .entry(origin.clone())
                    .or_default()
                    .non_live
                    .push(fragment.location.clone());
            }
        }
    }
}

fn package_relative_path(module_path: &str, resource_path: &str) -> PathBuf {
    let mut relative = PathBuf::new();
    if !module_path.is_empty() {
        relative.push(module_path);
    }
    relative.push(resource_path);
    relative
}

/// Encode a package identity into one stable output prefix.
///
/// Hex encoding keeps the prefix portable while making both the package origin and canonical name
/// injective. Consumer aliases never enter this identity.
pub(crate) fn package_output_prefix(
    package: &crate::compiler_frontend::semantic_identity::StablePackageIdentity,
) -> PathBuf {
    let mut encoded_name = String::from("p");
    for byte in package.name().as_bytes() {
        let _ = write!(encoded_name, "{byte:02x}");
    }

    PathBuf::from("_moth/packages").join(format!(
        "{}-{encoded_name}",
        package_origin_tag(package.origin())
    ))
}

fn package_origin_tag(origin: PackageOrigin) -> &'static str {
    match origin {
        PackageOrigin::Core => "core",
        PackageOrigin::Standard => "standard",
        PackageOrigin::Builder => "builder",
        PackageOrigin::ProjectLocal => "project-local",
        PackageOrigin::Dependency => "dependency",
    }
}
fn validate_output_path(
    output_path: &Path,
    artefact_kind: &str,
) -> Result<OutputPathIdentity, CompilerError> {
    output_path_identity(output_path).map_err(|reason| {
        CompilerError::compiler_error(format!(
            "HTML {artefact_kind} output path '{}' is invalid: {reason:?}",
            output_path.display()
        ))
    })
}

pub(crate) fn display_origin(origin: &StableResourceOriginId) -> String {
    let owner = match origin.owner() {
        StableResourceOwnerId::Module(module) => format!(
            "module package origin '{}' name '{}' role '{}' at '{}'",
            package_origin_tag(module.package().origin()),
            module.package().name(),
            module_root_role_tag(module.role()),
            module.logical_module_path()
        ),
        StableResourceOwnerId::Provider(provider) => format!(
            "provider '{}' package origin '{}' name '{}'",
            provider.provider_kind(),
            package_origin_tag(provider.package().origin()),
            provider.package().name()
        ),
    };
    format!("{owner}/{}", origin.logical_path().as_str())
}

fn module_root_role_tag(role: ModuleRootRole) -> &'static str {
    match role {
        ModuleRootRole::Normal => "normal",
        ModuleRootRole::Support => "support",
        ModuleRootRole::ProjectPackageFacade => "facade",
    }
}

#[cfg(test)]
#[path = "tests/resource_output_plan_tests.rs"]
mod tests;
