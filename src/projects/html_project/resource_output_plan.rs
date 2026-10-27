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
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::paths::resource_identity::{
    StableResourceOriginId, StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StablePackageIdentity};
use crate::compiler_frontend::source::SourceSpan;
use crate::projects::html_project::diagnostics::{
    artefact_output_path_invalid_diagnostic, capture_html_diagnostic,
    resource_output_path_collision_diagnostic, resource_output_path_reserved_diagnostic,
};
use crate::projects::html_project::page_metadata::{HtmlPageMetadataPlan, MetadataResourceUse};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Cold source provenance retained by the HTML resource planner.
///
/// A `SourceSpan` carries only a numeric source ID. This builder-local wrapper keeps the stable
/// package domain beside that ID until diagnostics cross the frozen rendering boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceDiagnosticSite {
    pub(crate) span: SourceSpan,
    pub(crate) source_domain: Option<StablePackageIdentity>,
}

impl ResourceDiagnosticSite {
    pub(crate) fn project(span: SourceSpan) -> Self {
        Self {
            span,
            source_domain: None,
        }
    }

    fn from_span(
        span: Option<SourceSpan>,
        source_domain: Option<&StablePackageIdentity>,
    ) -> Option<Self> {
        span.map(|span| Self {
            span,
            source_domain: source_domain.cloned(),
        })
    }
}

/// The artefact whose URL rules observe one resource-bearing string.
///
/// The current HTML entry pipeline has one explicit page-document context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceUrlContext {
    PageDocument(PathBuf),
}

impl ResourceUrlContext {
    fn page_document(path: &Path) -> Result<Self, InvalidOutputFolderReason> {
        output_path_identity(path)?;
        Ok(Self::PageDocument(path.to_path_buf()))
    }

    pub(crate) fn artefact_path(&self) -> &Path {
        match self {
            Self::PageDocument(path) => path,
        }
    }
}

/// One reachable use of a planned resource output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedResourceUse {
    pub(crate) context: ResourceUrlContext,
    pub(crate) authored_span: Option<ResourceDiagnosticSite>,
}

/// The lane through which one output record observes a resource URL use.
///
/// Executable uses are HIR-reachable and claim the record's first authored location over
/// metadata uses. Metadata uses are compile-time fragment and page-metadata observations, which
/// keep a resource output-planned without an executable reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceUseKind {
    Executable,
    Metadata,
}

/// One deduplicated resource output record.
///
/// The record contains no bytes and no content hash. One origin may carry several uses when
/// entries observe the same emitted output from different page contexts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedResourceOutput {
    pub(crate) origin: StableResourceOriginId,
    pub(crate) output_path: PathBuf,
    /// Exact span of the first authored owner when the record carries source-owned provenance.
    pub(crate) first_authored_span: Option<ResourceDiagnosticSite>,
    pub(crate) uses: Vec<PlannedResourceUse>,
    has_executable_use: bool,
}

/// Authored spans for one origin, with an intern-table fallback last.
///
/// `executable` holds HIR-reachable uses. `metadata` holds compile-time fragment and
/// page-metadata uses, which are output-live but not HIR-reachable.
#[derive(Clone, Debug, Default)]
pub(crate) struct OriginAuthoredSpans {
    pub executable: Vec<Option<ResourceDiagnosticSite>>,
    pub metadata: Vec<Option<ResourceDiagnosticSite>>,
    pub fallback: Option<ResourceDiagnosticSite>,
}

/// One page-observed URL use claimed while planning an origin.
///
/// Provider-declared outputs carry no observed use: their placement comes from the
/// provider's declared stable output path rather than a rendered reference.
#[derive(Clone)]
struct ObservedResourceUse {
    context: ResourceUrlContext,
    kind: ResourceUseKind,
}

/// Byte-free HTML resource plan accumulated across all selected entries.
#[derive(Debug, Default)]
pub(crate) struct HtmlResourceOutputPlan {
    project_name: String,
    records: Vec<PlannedResourceOutput>,
    record_by_output: HashMap<OutputPathIdentity, usize>,
    /// Direct origin index for renderer lookups; `record_by_output` owns collision detection.
    record_by_origin: HashMap<StableResourceOriginId, usize>,
    /// Backend-owned artifact labels; they are interned only if a diagnostic is emitted.
    builder_output_paths: HashMap<OutputPathIdentity, &'static str>,
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
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        let context = ResourceUrlContext::page_document(page_output_path).map_err(|reason| {
            invalid_artefact_output_path(page_output_path, "HTML page", reason, None, string_table)
        })?;
        self.reserve_builder_output_path(page_output_path, "HTML page", string_table)?;

        let spans = first_authored_spans(entry)?;
        self.plan_union(entry.resource_union, &spans, context.clone(), string_table)?;
        self.plan_page_metadata_uses(
            &page_metadata_plan.resource_uses,
            context,
            entry.source_domain,
            string_table,
        )
    }

    /// Reserve a known HTML, JavaScript, Wasm, CSS or manifest destination.
    ///
    /// Duplicate destinations owned by the same builder lane are left to the existing builder
    /// duplicate checks. A resource destination, however, is rejected immediately so this method
    /// can be called before a byte-reading emitter.
    pub(crate) fn reserve_builder_output_path(
        &mut self,
        output_path: &Path,
        artefact_kind: &'static str,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        let identity = validated_output_identity(output_path, artefact_kind, None, string_table)?;
        self.reserve_identity(output_path, artefact_kind, identity, string_table)
    }

    /// Reserve a compiler-owned runtime module destination.
    ///
    /// Runtime module specifiers come from registry constants, so an invalid destination is a
    /// compiler contract breach rather than an authored path diagnostic.
    pub(crate) fn reserve_compiler_output_path(
        &mut self,
        output_path: &Path,
        artefact_kind: &'static str,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        let identity = output_path_identity(output_path)
            .map_err(|reason| invalid_output_path_internal(output_path, artefact_kind, reason))?;
        self.reserve_identity(output_path, artefact_kind, identity, string_table)
    }

    fn reserve_identity(
        &mut self,
        output_path: &Path,
        artefact_kind: &'static str,
        identity: OutputPathIdentity,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        if let Some(&record_index) = self.record_by_output.get(&identity) {
            let record = &self.records[record_index];
            return Err(capture_html_diagnostic(
                resource_output_path_reserved_diagnostic(
                    output_path,
                    &display_origin(&record.origin),
                    artefact_kind,
                    record.first_authored_span.clone(),
                    string_table,
                ),
                string_table.strings(),
                None,
                || None,
            ));
        }

        self.builder_output_paths
            .entry(identity)
            .or_insert(artefact_kind);
        Ok(())
    }

    /// Add one origin and one explicit context, primarily for focused planner tests and future
    /// builders whose link-plan owner already has the first authored span.
    pub(crate) fn plan_origin(
        &mut self,
        origin: StableResourceOriginId,
        first_authored_span: Option<ResourceDiagnosticSite>,
        context: ResourceUrlContext,
        string_table: &mut DiagnosticStringBuilder,
        use_kind: ResourceUseKind,
    ) -> Result<(), PreparedFailure> {
        let output_path = self.output_path_for_origin(&origin);
        let observed_use = ObservedResourceUse {
            context,
            kind: use_kind,
        };
        self.plan_one_origin(
            origin,
            output_path,
            first_authored_span,
            Some(observed_use),
            string_table,
        )
    }

    fn plan_page_metadata_uses(
        &mut self,
        resource_uses: &[MetadataResourceUse],
        context: ResourceUrlContext,
        source_domain: Option<&StablePackageIdentity>,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        for resource_use in resource_uses {
            self.plan_origin(
                resource_use.origin.clone(),
                ResourceDiagnosticSite::from_span(resource_use.authored_span, source_domain),
                context.clone(),
                string_table,
                ResourceUseKind::Metadata,
            )?;
        }
        Ok(())
    }

    /// Plan one provider-owned runtime asset origin into the shared byte-free plan.
    ///
    /// Provider assets have no page-observed URL use: the provider's declared output path is
    /// the stable placement contract. Planning the origin still rejects collisions with
    /// every other planned origin and reserved builder path before any byte read.
    pub(crate) fn plan_provider_runtime_asset(
        &mut self,
        origin: StableResourceOriginId,
        source_span: Option<ResourceDiagnosticSite>,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        let output_path = self.output_path_for_origin(&origin);

        self.plan_one_origin(origin, output_path, source_span, None, string_table)
    }

    /// Look up one planned record directly by its stable resource origin.
    pub(crate) fn record_for_origin(
        &self,
        origin: &StableResourceOriginId,
    ) -> Option<&PlannedResourceOutput> {
        self.record_by_origin
            .get(origin)
            .map(|&record_index| &self.records[record_index])
    }

    /// Consume the plan into its records for a writer-owned handoff.
    pub(crate) fn into_records(self) -> Vec<PlannedResourceOutput> {
        self.records
    }

    fn plan_union(
        &mut self,
        union: &ResourceOriginUnion,
        spans: &HashMap<StableResourceOriginId, OriginAuthoredSpans>,
        context: ResourceUrlContext,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        for origin in union.iter() {
            let Some(authored_spans) = spans.get(origin) else {
                let error = InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "HTML resource origin {origin:?} has no authored span in its linked module resource tables"
                    ),
                );
                return Err(error.into());
            };

            let output_path = self.output_path_for_origin(origin);

            for authored_span in &authored_spans.executable {
                self.plan_one_origin(
                    origin.clone(),
                    output_path.clone(),
                    authored_span.clone(),
                    Some(ObservedResourceUse {
                        context: context.clone(),
                        kind: ResourceUseKind::Executable,
                    }),
                    string_table,
                )?;
            }

            for authored_span in &authored_spans.metadata {
                self.plan_one_origin(
                    origin.clone(),
                    output_path.clone(),
                    authored_span.clone(),
                    Some(ObservedResourceUse {
                        context: context.clone(),
                        kind: ResourceUseKind::Metadata,
                    }),
                    string_table,
                )?;
            }

            if authored_spans.executable.is_empty() && authored_spans.metadata.is_empty() {
                if let Some(fallback_span) = authored_spans.fallback.clone() {
                    self.plan_one_origin(
                        origin.clone(),
                        output_path,
                        Some(fallback_span),
                        None,
                        string_table,
                    )?;
                } else {
                    let error = InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "HTML resource origin {origin:?} has no authored span in its linked module resource tables"
                        ),
                    );
                    return Err(error.into());
                }
            }
        }

        Ok(())
    }

    fn plan_one_origin(
        &mut self,
        origin: StableResourceOriginId,
        output_path: PathBuf,
        first_authored_span: Option<ResourceDiagnosticSite>,
        observed_use: Option<ObservedResourceUse>,
        string_table: &mut DiagnosticStringBuilder,
    ) -> Result<(), PreparedFailure> {
        let output_identity = match output_path_identity(&output_path) {
            Ok(identity) => identity,
            Err(reason) if output_path_is_authored(&origin) => {
                return Err(invalid_artefact_output_path(
                    &output_path,
                    "HTML resource",
                    reason,
                    first_authored_span.clone(),
                    string_table,
                ));
            }
            Err(reason) => {
                return Err(invalid_output_path_internal(
                    &output_path,
                    "resource",
                    reason,
                ));
            }
        };
        if let Some(&record_index) = self.record_by_output.get(&output_identity) {
            let record = &mut self.records[record_index];
            if record.origin != origin {
                return Err(capture_html_diagnostic(
                    resource_output_path_collision_diagnostic(
                        &output_path,
                        &display_origin(&record.origin),
                        record.first_authored_span.clone(),
                        &display_origin(&origin),
                        first_authored_span.clone(),
                        string_table,
                    ),
                    string_table.strings(),
                    None,
                    || None,
                ));
            }

            if let Some(observed_use) = observed_use {
                match observed_use.kind {
                    ResourceUseKind::Executable => {
                        if !record.has_executable_use {
                            record.first_authored_span = first_authored_span.clone();
                            record.has_executable_use = true;
                        }
                    }
                    ResourceUseKind::Metadata => {
                        if !record.has_executable_use && record.uses.is_empty() {
                            record.first_authored_span = first_authored_span.clone();
                        }
                    }
                }

                let use_record = PlannedResourceUse {
                    context: observed_use.context.clone(),
                    authored_span: first_authored_span.clone(),
                };
                if !record.uses.contains(&use_record) {
                    record.uses.push(use_record);
                }
            }
            return Ok(());
        }

        if let Some(&artefact_kind) = self.builder_output_paths.get(&output_identity) {
            return Err(capture_html_diagnostic(
                resource_output_path_reserved_diagnostic(
                    &output_path,
                    &display_origin(&origin),
                    artefact_kind,
                    first_authored_span.clone(),
                    string_table,
                ),
                string_table.strings(),
                None,
                || None,
            ));
        }

        let record_index = self.records.len();
        let has_executable_use = observed_use
            .as_ref()
            .is_some_and(|observed_use| observed_use.kind == ResourceUseKind::Executable);
        let uses = observed_use
            .map(|observed_use| {
                vec![PlannedResourceUse {
                    context: observed_use.context,
                    authored_span: first_authored_span.clone(),
                }]
            })
            .unwrap_or_default();
        self.record_by_origin.insert(origin.clone(), record_index);
        self.records.push(PlannedResourceOutput {
            origin,
            output_path,
            first_authored_span,
            uses,
            has_executable_use,
        });
        self.record_by_output.insert(output_identity, record_index);
        Ok(())
    }

    fn output_path_for_origin(&self, origin: &StableResourceOriginId) -> PathBuf {
        match origin.owner() {
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
        }
    }
}

fn first_authored_spans(
    entry: &ProjectEntry<'_>,
) -> Result<HashMap<StableResourceOriginId, OriginAuthoredSpans>, InfrastructureFailure> {
    let mut spans = HashMap::new();
    let mut module_views = Vec::with_capacity(1 + entry.linked_modules.len());
    module_views.push((entry.module, entry.source_domain, entry.reachability));
    module_views.extend(
        entry
            .linked_modules
            .iter()
            .map(|linked| (linked.module, linked.source_domain, linked.reachability)),
    );

    for (module, source_domain, reachability) in &module_views {
        record_reachable_resource_spans(&mut spans, module, *source_domain, reachability)?;
    }

    // Only the selected entry root contributes compile-time fragments to its entry union.
    record_const_fragment_resource_spans(&mut spans, entry.module, entry.source_domain);

    for (module, source_domain, _) in module_views {
        for resource in module.executable.resource_table.origins() {
            spans
                .entry(resource.origin.clone())
                .or_insert_with(|| OriginAuthoredSpans {
                    executable: Vec::new(),
                    metadata: Vec::new(),
                    fallback: ResourceDiagnosticSite::from_span(
                        resource.first_authored_span,
                        source_domain,
                    ),
                });
        }
    }

    Ok(spans)
}

fn record_reachable_resource_spans(
    spans: &mut HashMap<StableResourceOriginId, OriginAuthoredSpans>,
    module: &Module,
    source_domain: Option<&StablePackageIdentity>,
    reachability: &HirReachability,
) -> Result<(), InfrastructureFailure> {
    for resource_use in &reachability.reachable_resource_uses {
        let resource = module
            .executable
            .resource_table
            .try_origin(resource_use.resource_id)?;
        spans
            .entry(resource.origin.clone())
            .or_default()
            .executable
            .push(ResourceDiagnosticSite::from_span(
                resource_use.span,
                source_domain,
            ));
    }

    Ok(())
}

fn record_const_fragment_resource_spans(
    spans: &mut HashMap<StableResourceOriginId, OriginAuthoredSpans>,
    module: &Module,
    source_domain: Option<&StablePackageIdentity>,
) {
    for fragment in &module.metadata.const_top_level_fragments {
        let OwnedFoldedString::Pieces(pieces) = &fragment.value else {
            continue;
        };

        for piece in pieces {
            if let OwnedFoldedStringPiece::Resource(origin) = piece {
                spans.entry(origin.clone()).or_default().metadata.push(
                    ResourceDiagnosticSite::from_span(fragment.span, source_domain),
                );
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
pub(crate) fn package_output_prefix(package: &StablePackageIdentity) -> PathBuf {
    // Keep this total package-name hex encoding distinct from `percent_encode_url_segment`; output
    // paths depend on its opaque spelling.
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
        PackageOrigin::Builder => "builder",
        PackageOrigin::ProjectLocal => "project-local",
        PackageOrigin::Dependency => "dependency",
    }
}

fn validated_output_identity(
    output_path: &Path,
    artefact_kind: &str,
    span: Option<ResourceDiagnosticSite>,
    string_table: &mut DiagnosticStringBuilder,
) -> Result<OutputPathIdentity, PreparedFailure> {
    output_path_identity(output_path).map_err(|reason| {
        invalid_artefact_output_path(output_path, artefact_kind, reason, span, string_table)
    })
}

/// Only authored module resources carry user-chosen path components. Provider-declared paths and
/// builder or core package paths are contract breaches, so they stay infrastructure failures.
fn output_path_is_authored(origin: &StableResourceOriginId) -> bool {
    match origin.owner() {
        StableResourceOwnerId::Module(module_origin) => matches!(
            module_origin.package().origin(),
            PackageOrigin::ProjectLocal | PackageOrigin::Dependency
        ),
        StableResourceOwnerId::Provider(_) => false,
    }
}

fn invalid_artefact_output_path(
    output_path: &Path,
    artefact_kind: &str,
    reason: InvalidOutputFolderReason,
    span: Option<ResourceDiagnosticSite>,
    string_table: &mut DiagnosticStringBuilder,
) -> PreparedFailure {
    capture_html_diagnostic(
        artefact_output_path_invalid_diagnostic(
            output_path,
            artefact_kind,
            reason,
            span,
            string_table,
        ),
        string_table.strings(),
        None,
        || None,
    )
}

/// Compiler-owned and provider-declared destinations never carry authored path components, so an
/// invalid one is a contract breach rather than a user diagnostic.
fn invalid_output_path_internal(
    output_path: &Path,
    artefact_kind: &str,
    reason: InvalidOutputFolderReason,
) -> PreparedFailure {
    InfrastructureFailure::internal(
        CompilerStage::ProjectBuildAssembly,
        format!(
            "HTML {artefact_kind} output path '{}' is invalid: {reason:?}",
            output_path.display()
        ),
    )
    .into()
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
