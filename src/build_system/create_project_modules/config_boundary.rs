//! Build-owned configuration boundary assembly.
//!
//! WHAT: collects source contracts, projects effective project fields into fixed providers and
//! `@project`, and resolves one authoritative map for each compilation boundary.
//! WHY: configuration resolution is a Stage 0 boundary operation. Keeping it outside module
//! orchestration prevents canonical modules from rebuilding or rebasing an already resolved map.

use crate::compiler_frontend::build_config::{
    BuildConfigContractFact, BuildConfigFingerprint, BuildConfigInputEntry, BuildConfigInputSet,
    BuildConfigResolutionError, BuildConfigValueLocation, BuildConfigValueOrigin, BuildInputName,
    BuildInputType, BuilderConfigGlobalSet, PrimitiveBuildInputType, PrimitiveBuildValue,
    ResolvedBuildConfigMap, build_config_fingerprint, resolve_build_config_values,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, InvalidConfigReason,
};
use crate::compiler_frontend::declaration_syntax::build_config_contract::build_input_type_name;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::project_globals::{
    PROJECT_GLOBALS_DEPENDENCY_NAME, ProjectGlobalsFieldInput, ProjectGlobalsInterface,
};
use crate::compiler_frontend::public_interface::portable_source_location;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::projects::settings::{Config, ProjectMetadataField};

use rustc_hash::FxHashSet;
use std::hash::{Hash, Hasher};

use super::module_inventory;
use super::prepared_module::PreparedModule;

/// Convert one prepared module's retained source contract shells into global-table facts.
///
/// The module table is forked from the boundary table. Merging its delta here makes every
/// diagnostic location usable by the boundary string table before semantic compilation later
/// performs the same merge for the complete module result.
pub(super) fn source_contract_facts_from_prepared(
    prepared: &PreparedModule,
    string_table: &mut StringTable,
    string_table_base_len: usize,
) -> Vec<BuildConfigContractFact> {
    let contracts = prepared.semantic.source_build_config_contracts();
    if contracts.is_empty() {
        return Vec::new();
    }

    let remap =
        string_table.merge_delta_from(&prepared.semantic.string_table, string_table_base_len);
    contracts
        .iter()
        .map(|contract| {
            let mut location = contract.location.clone();
            if !remap.is_identity() {
                location.remap_string_ids(&remap);
            }
            BuildConfigContractFact::new(
                contract.name.clone(),
                contract.value_type,
                contract.required,
                contract.default.clone(),
                location,
            )
        })
        .collect()
}

/// Collect canonical source contracts in deterministic compile-wave and source order.
pub(super) fn source_contract_facts_from_module_waves(
    module_waves: &[Vec<module_inventory::ModuleCompilationJob>],
    string_table: &mut StringTable,
) -> Vec<BuildConfigContractFact> {
    module_waves
        .iter()
        .flat_map(|wave| wave.iter())
        .flat_map(|job| {
            source_contract_facts_from_prepared(
                &job.prepared,
                string_table,
                job.string_table_base_len,
            )
        })
        .collect()
}

/// Collect transient source contracts in owner/source order for check mode.
pub(super) fn source_contract_facts_from_check_only_jobs(
    check_only_jobs: &[module_inventory::CheckOnlyModuleCompilationJob],
    string_table: &mut StringTable,
) -> Vec<BuildConfigContractFact> {
    check_only_jobs
        .iter()
        .flat_map(|job| {
            source_contract_facts_from_prepared(
                &job.prepared,
                string_table,
                job.string_table_base_len,
            )
        })
        .collect()
}
/// Copy one prepared module's source contracts for an isolated check-only resolution.
///
/// Canonical modules consume the boundary map directly; only a transient check-only unit needs
/// its own source facts when building its private resolution view.
pub(super) fn source_contract_facts_for_current_module(
    prepared: &PreparedModule,
) -> Vec<BuildConfigContractFact> {
    prepared
        .semantic
        .source_build_config_contracts()
        .iter()
        .map(|contract| {
            BuildConfigContractFact::new(
                contract.name.clone(),
                contract.value_type,
                contract.required,
                contract.default.clone(),
                contract.location.clone(),
            )
        })
        .collect()
}

/// Return the names that are known to one boundary's canonical or direct-project contracts.
///
/// Fixed project fields are deliberately absent: they provide values only for a matching source
/// contract and must not make an explicit input known on their own.
fn known_build_config_names(
    source_facts: &[BuildConfigContractFact],
    direct_project_facts: &[BuildConfigContractFact],
) -> FxHashSet<BuildInputName> {
    source_facts
        .iter()
        .chain(direct_project_facts)
        .map(|fact| fact.name().clone())
        .collect()
}

/// Keep explicit inputs that can participate in a selected canonical/check-only resolution.
///
/// Resolution uses the filtered set so an input belonging only to a sibling transient unit cannot
/// be classified as unknown while that unit is being resolved.
pub(super) fn filter_build_config_inputs_to_known_facts(
    explicit_inputs: &BuildConfigInputSet,
    source_facts: &[BuildConfigContractFact],
    direct_project_facts: &[BuildConfigContractFact],
) -> BuildConfigInputSet {
    let known_names = known_build_config_names(source_facts, direct_project_facts);
    let mut filtered = BuildConfigInputSet::new();
    for input in explicit_inputs.iter() {
        if known_names.contains(input.name()) {
            filtered
                .insert(input.clone())
                .expect("filtered build-config inputs preserve unique names");
        }
    }
    filtered
}

/// Return the first explicit input absent from the union of all facts actually analyzed.
pub(super) fn first_unknown_build_config_input(
    explicit_inputs: &BuildConfigInputSet,
    source_facts: &[BuildConfigContractFact],
    direct_project_facts: &[BuildConfigContractFact],
) -> Option<BuildConfigInputEntry> {
    let known_names = known_build_config_names(source_facts, direct_project_facts);
    explicit_inputs
        .iter()
        .find(|input| !known_names.contains(input.name()))
        .cloned()
}

/// One effective project field shared by fixed source providers and `@project`.
///
/// The snapshot keeps the semantic folded value and canonical type together with the field's
/// capability/provenance kind. Direct project `#Config` records retain their selected provider
/// metadata; ordinary fixed fields retain fixed provenance; arbitrary metadata remains visible
/// only through `@project`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EffectiveProjectField {
    pub(super) name: String,
    pub(super) type_identity: CanonicalTypeIdentity,
    pub(super) value: PublicFoldedValue,
    pub(super) location: SourceLocation,
    pub(super) fingerprint: BuildConfigFingerprint,
    pub(super) kind: EffectiveProjectFieldKind,
}

/// Capabilities and provenance retained for one effective project field.
///
/// Keeping these cases together makes fixed-provider and direct-config projections exhaustive:
/// metadata cannot accidentally acquire build-config contract state, while direct contracts always
/// carry the values needed to preserve their already-selected provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EffectiveProjectFieldKind {
    FixedPrimitive {
        value_type: BuildInputType,
        value: Option<PrimitiveBuildValue>,
        required: bool,
    },
    DirectConfig {
        contract: BuildInputType,
        value: Option<PrimitiveBuildValue>,
        required: bool,
        default: Option<PrimitiveBuildValue>,
        origin: BuildConfigValueOrigin,
        value_location: Option<BuildConfigValueLocation>,
    },
    Metadata,
}

/// Build one deterministic snapshot of the project fields visible at this build boundary.
///
/// Omitted optional metadata has no field unless a schema/default or authored/direct record made
/// it effective. `entry_root` is always present for a loaded config because its schema default is
/// applied by project-config validation. Direct project `#Config` records take precedence over
/// fixed fields with the same name.
pub(super) fn effective_project_fields(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<Vec<EffectiveProjectField>, CompilerError> {
    if !config.project_config_loaded {
        return Ok(Vec::new());
    }

    let mut fields = Vec::new();
    let mut seen_names = FxHashSet::default();
    for record in &config.config_resolution_records {
        let field = effective_project_field_from_resolution_record(record, string_table)?;
        if !seen_names.insert(field.name.clone()) {
            return Err(CompilerError::compiler_error(format!(
                "direct project config field '{}' was retained more than once",
                field.name
            )));
        }
        fields.push(field);
    }

    let add_fixed = |fields: &mut Vec<EffectiveProjectField>,
                     seen_names: &mut FxHashSet<String>,
                     field: EffectiveProjectField|
     -> Result<(), CompilerError> {
        if seen_names.insert(field.name.clone()) {
            fields.push(field);
            Ok(())
        } else {
            Ok(())
        }
    };

    let name_type = BuildInputType::Primitive(PrimitiveBuildInputType::String);
    add_fixed(
        &mut fields,
        &mut seen_names,
        effective_fixed_project_field(
            "name",
            name_type,
            true,
            Some(PrimitiveBuildValue::String(config.project_name.clone())),
            config.setting_location_or_config_file("name", string_table),
        )?,
    )?;

    let entry_root = config.entry_root.to_str().ok_or_else(|| {
        CompilerError::file_error(
            &config.entry_root,
            "configured entry root is not valid UTF-8",
            string_table,
        )
    })?;
    add_fixed(
        &mut fields,
        &mut seen_names,
        effective_fixed_project_field(
            "entry_root",
            name_type,
            true,
            Some(PrimitiveBuildValue::String(entry_root.to_owned())),
            config.setting_location_or_config_file("entry_root", string_table),
        )?,
    )?;

    let optional_string_type = BuildInputType::Optional(PrimitiveBuildInputType::String);
    if config.version.is_some() || config.setting_locations.contains_key("version") {
        add_fixed(
            &mut fields,
            &mut seen_names,
            effective_fixed_project_field(
                "version",
                optional_string_type,
                false,
                config.version.clone().map(PrimitiveBuildValue::String),
                config.setting_location_or_config_file("version", string_table),
            )?,
        )?;
    }
    if config.author.is_some() || config.setting_locations.contains_key("author") {
        add_fixed(
            &mut fields,
            &mut seen_names,
            effective_fixed_project_field(
                "author",
                optional_string_type,
                false,
                config.author.clone().map(PrimitiveBuildValue::String),
                config.setting_location_or_config_file("author", string_table),
            )?,
        )?;
    }
    if config.license.is_some() || config.setting_locations.contains_key("license") {
        add_fixed(
            &mut fields,
            &mut seen_names,
            effective_fixed_project_field(
                "license",
                optional_string_type,
                false,
                config.license.clone().map(PrimitiveBuildValue::String),
                config.setting_location_or_config_file("license", string_table),
            )?,
        )?;
    }

    if config
        .setting_locations
        .contains_key("template_const_loop_iteration_limit")
    {
        let loop_limit =
            i32::try_from(config.template_const_loop_iteration_limit).map_err(|_| {
                CompilerError::compiler_error(
                    "configured template loop limit cannot be represented as a build Int",
                )
            })?;
        add_fixed(
            &mut fields,
            &mut seen_names,
            effective_fixed_project_field(
                "template_const_loop_iteration_limit",
                BuildInputType::Primitive(PrimitiveBuildInputType::Int),
                true,
                Some(PrimitiveBuildValue::Int(loop_limit)),
                config.setting_location_or_config_file(
                    "template_const_loop_iteration_limit",
                    string_table,
                ),
            )?,
        )?;
    }

    for metadata in &config.extra_project_fields {
        if !seen_names.insert(metadata.name.clone()) {
            continue;
        }
        fields.push(effective_project_field_from_metadata(metadata));
    }

    Ok(fields)
}

fn effective_project_field_from_resolution_record(
    record: &crate::compiler_frontend::build_config::ConfigResolutionRecord,
    string_table: &StringTable,
) -> Result<EffectiveProjectField, CompilerError> {
    let field_name = string_table.resolve(record.field_name);
    let name = BuildInputName::new(field_name).map_err(|_| {
        CompilerError::compiler_error(format!(
            "direct project config record has invalid build-input name '{field_name}'"
        ))
    })?;
    let value = public_folded_value_for_build_input(record.contract, record.value.as_ref())?;
    Ok(EffectiveProjectField {
        name: name.as_str().to_owned(),
        type_identity: canonical_type_identity_for_build_input(record.contract),
        value,
        location: record.qualifier_location.clone(),
        fingerprint: record.fingerprint,
        kind: EffectiveProjectFieldKind::DirectConfig {
            contract: record.contract,
            value: record.value.clone(),
            required: record.required,
            default: record.default.clone(),
            origin: record.origin,
            value_location: record.value_location.clone(),
        },
    })
}

fn effective_fixed_project_field(
    name: &str,
    build_input_type: BuildInputType,
    required: bool,
    primitive_value: Option<PrimitiveBuildValue>,
    location: SourceLocation,
) -> Result<EffectiveProjectField, CompilerError> {
    let build_name = BuildInputName::new(name).expect("compiler-owned project field name is valid");
    let value = public_folded_value_for_build_input(build_input_type, primitive_value.as_ref())?;
    let fingerprint = build_config_fingerprint(name, build_input_type, primitive_value.as_ref());
    Ok(EffectiveProjectField {
        name: build_name.as_str().to_owned(),
        type_identity: canonical_type_identity_for_build_input(build_input_type),
        value,
        location,
        fingerprint,
        kind: EffectiveProjectFieldKind::FixedPrimitive {
            value_type: build_input_type,
            value: primitive_value,
            required,
        },
    })
}

fn effective_project_field_from_metadata(field: &ProjectMetadataField) -> EffectiveProjectField {
    let kind = match fixed_build_value_from_metadata(field) {
        Some((value_type, value)) => EffectiveProjectFieldKind::FixedPrimitive {
            value_type,
            value,
            required: false,
        },
        None => EffectiveProjectFieldKind::Metadata,
    };
    let fingerprint = project_metadata_fingerprint(&field.name, &field.type_identity, &field.value);
    EffectiveProjectField {
        name: field.name.clone(),
        type_identity: field.type_identity.clone(),
        value: field.value.clone(),
        location: field.location.clone(),
        fingerprint,
        kind,
    }
}

/// Convert fixed primitive fields in one effective snapshot into fixed provider facts.
pub(super) fn fixed_project_contract_facts(
    fields: &[EffectiveProjectField],
) -> Vec<BuildConfigContractFact> {
    fields
        .iter()
        .filter_map(|field| {
            let EffectiveProjectFieldKind::FixedPrimitive {
                value_type,
                value,
                required,
            } = &field.kind
            else {
                return None;
            };
            let name = BuildInputName::new(&field.name).ok()?;
            Some(BuildConfigContractFact::new(
                name,
                *value_type,
                *required,
                value.clone(),
                field.location.clone(),
            ))
        })
        .collect()
}

/// Convert direct project `#Config` records in one effective snapshot into barrier facts.
pub(super) fn direct_project_contract_facts(
    fields: &[EffectiveProjectField],
) -> Vec<BuildConfigContractFact> {
    fields
        .iter()
        .filter_map(|field| {
            let EffectiveProjectFieldKind::DirectConfig {
                contract,
                value,
                required,
                default,
                origin,
                value_location,
            } = &field.kind
            else {
                return None;
            };
            Some(
                BuildConfigContractFact::new(
                    BuildInputName::new(&field.name)
                        .expect("effective direct project field has a valid name"),
                    *contract,
                    *required,
                    default.clone(),
                    field.location.clone(),
                )
                .with_resolved_provider(
                    value.clone(),
                    *origin,
                    field.fingerprint,
                    value_location.clone(),
                ),
            )
        })
        .collect()
}

/// Build the immutable synthetic `@project` provider from one effective project-field snapshot.
pub(super) fn build_project_globals_interface(
    config: &Config,
    fields: &[EffectiveProjectField],
    string_table: &StringTable,
) -> Result<Option<ProjectGlobalsInterface>, CompilerError> {
    if !config.project_config_loaded {
        return Ok(None);
    }

    let project_fields = fields
        .iter()
        .map(|field| {
            ProjectGlobalsFieldInput::new(
                field.name.clone(),
                field.type_identity.clone(),
                field.value.clone(),
                portable_source_location(&field.location, string_table),
                field.fingerprint,
                project_global_provenance(&field.name),
            )
        })
        .collect();
    ProjectGlobalsInterface::new(
        StablePackageIdentity::project_local(&config.project_name),
        project_fields,
    )
    .map(Some)
}

fn fixed_build_value_from_metadata(
    field: &ProjectMetadataField,
) -> Option<(BuildInputType, Option<PrimitiveBuildValue>)> {
    match &field.value {
        PublicFoldedValue::Int(value) => Some((
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            Some(PrimitiveBuildValue::Int(*value)),
        )),
        PublicFoldedValue::Float(value) => Some((
            BuildInputType::Primitive(PrimitiveBuildInputType::Float),
            Some(PrimitiveBuildValue::Float(value.clone())),
        )),
        PublicFoldedValue::Bool(value) => Some((
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            Some(PrimitiveBuildValue::Bool(*value)),
        )),
        PublicFoldedValue::Char(value) => Some((
            BuildInputType::Primitive(PrimitiveBuildInputType::Char),
            Some(PrimitiveBuildValue::Char(*value)),
        )),
        PublicFoldedValue::String(value) => Some((
            BuildInputType::Primitive(PrimitiveBuildInputType::String),
            Some(PrimitiveBuildValue::String(value.clone().into_text()?)),
        )),
        PublicFoldedValue::OptionSome(value) => {
            let value = primitive_build_value_from_metadata(value)?;
            Some((
                BuildInputType::Optional(value.primitive_type()),
                Some(value),
            ))
        }
        PublicFoldedValue::OptionNone => Some((
            BuildInputType::Optional(canonical_option_primitive(&field.type_identity)?),
            None,
        )),
        PublicFoldedValue::ConstTemplate(_)
        | PublicFoldedValue::Collection(_)
        | PublicFoldedValue::Record(_)
        | PublicFoldedValue::Choice { .. }
        | PublicFoldedValue::Range { .. } => None,
    }
}

fn primitive_build_value_from_metadata(value: &PublicFoldedValue) -> Option<PrimitiveBuildValue> {
    match value {
        PublicFoldedValue::Int(value) => Some(PrimitiveBuildValue::Int(*value)),
        PublicFoldedValue::Float(value) => Some(PrimitiveBuildValue::Float(value.clone())),
        PublicFoldedValue::Bool(value) => Some(PrimitiveBuildValue::Bool(*value)),
        PublicFoldedValue::Char(value) => Some(PrimitiveBuildValue::Char(*value)),
        PublicFoldedValue::String(value) => {
            Some(PrimitiveBuildValue::String(value.clone().into_text()?))
        }
        PublicFoldedValue::ConstTemplate(_)
        | PublicFoldedValue::Collection(_)
        | PublicFoldedValue::Record(_)
        | PublicFoldedValue::Choice { .. }
        | PublicFoldedValue::Range { .. }
        | PublicFoldedValue::OptionSome(_)
        | PublicFoldedValue::OptionNone => None,
    }
}

fn canonical_option_primitive(
    type_identity: &CanonicalTypeIdentity,
) -> Option<PrimitiveBuildInputType> {
    let CanonicalTypeIdentity::Option(inner) = type_identity else {
        return None;
    };
    match inner.as_ref() {
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String) => {
            Some(PrimitiveBuildInputType::String)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int) => {
            Some(PrimitiveBuildInputType::Int)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float) => {
            Some(PrimitiveBuildInputType::Float)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool) => {
            Some(PrimitiveBuildInputType::Bool)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Char) => {
            Some(PrimitiveBuildInputType::Char)
        }
        _ => None,
    }
}

fn canonical_type_identity_for_build_input(value_type: BuildInputType) -> CanonicalTypeIdentity {
    let primitive = match value_type.primitive() {
        PrimitiveBuildInputType::String => CanonicalBuiltinType::String,
        PrimitiveBuildInputType::Int => CanonicalBuiltinType::Int,
        PrimitiveBuildInputType::Float => CanonicalBuiltinType::Float,
        PrimitiveBuildInputType::Bool => CanonicalBuiltinType::Bool,
        PrimitiveBuildInputType::Char => CanonicalBuiltinType::Char,
    };
    let primitive = CanonicalTypeIdentity::Builtin(primitive);
    if value_type.is_optional() {
        CanonicalTypeIdentity::Option(Box::new(primitive))
    } else {
        primitive
    }
}

fn public_folded_value_for_build_input(
    value_type: BuildInputType,
    value: Option<&PrimitiveBuildValue>,
) -> Result<PublicFoldedValue, CompilerError> {
    let Some(value) = value else {
        if value_type.is_optional() {
            return Ok(PublicFoldedValue::OptionNone);
        }
        return Err(CompilerError::compiler_error(format!(
            "required project-global field has no folded value for {}",
            build_input_type_name(value_type)
        )));
    };
    if !value_type.accepts_primitive(value.primitive_type()) {
        return Err(CompilerError::compiler_error(format!(
            "project-global field value type {} does not satisfy {}",
            value.primitive_type().name(),
            build_input_type_name(value_type)
        )));
    }
    let folded = match value {
        PrimitiveBuildValue::String(value) => {
            PublicFoldedValue::String(OwnedFoldedString::Text(value.clone()))
        }
        PrimitiveBuildValue::Int(value) => PublicFoldedValue::Int(*value),
        PrimitiveBuildValue::Float(value) => PublicFoldedValue::Float(value.clone()),
        PrimitiveBuildValue::Bool(value) => PublicFoldedValue::Bool(*value),
        PrimitiveBuildValue::Char(value) => PublicFoldedValue::Char(*value),
    };
    if value_type.is_optional() {
        Ok(PublicFoldedValue::OptionSome(Box::new(folded)))
    } else {
        Ok(folded)
    }
}

fn project_global_provenance(name: &str) -> SyntheticInterfaceProvenance {
    SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        PROJECT_GLOBALS_DEPENDENCY_NAME,
        name,
    ))
}

/// Hash one project-global field's semantic identity without rendering compiler-owned values.
pub(super) fn project_metadata_fingerprint(
    name: &str,
    type_identity: &CanonicalTypeIdentity,
    value: &PublicFoldedValue,
) -> BuildConfigFingerprint {
    let mut hasher = StableFingerprintHasher::default();
    hasher.write_u64(name.len() as u64);
    hasher.write(name.as_bytes());
    type_identity.hash(&mut hasher);
    value.hash(&mut hasher);
    BuildConfigFingerprint(hasher.finish())
}

struct StableFingerprintHasher {
    hash: u64,
}

impl Default for StableFingerprintHasher {
    fn default() -> Self {
        Self {
            hash: 0xcbf29ce484222325,
        }
    }
}

impl Hasher for StableFingerprintHasher {
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, bytes: &[u8]) {
        const FNV_PRIME: u64 = 0x100000001b3;
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(FNV_PRIME);
        }
    }
}

/// Render one contract fact for a conflict diagnostic without exposing compiler internals in the
/// diagnostic payload.
fn describe_build_config_contract(fact: &BuildConfigContractFact) -> String {
    let required = if fact.required() {
        "required"
    } else {
        "optional"
    };
    let default = fact.default_value().map_or_else(
        || "no default".to_owned(),
        |value| format!("default={value:?}"),
    );
    format!(
        "{}; {required}; {default}",
        build_input_type_name(fact.value_type())
    )
}

/// Map one compiler-owned barrier failure onto the existing structured config diagnostic lane.
pub(super) fn build_config_resolution_messages(
    error: BuildConfigResolutionError,
    fallback_location: SourceLocation,
    string_table: &mut StringTable,
) -> CompilerMessages {
    if matches!(
        &error,
        BuildConfigResolutionError::DirectProjectProviderMissing { .. }
    ) {
        return CompilerMessages::from_error_ref(
            CompilerError::compiler_error(format!(
                "direct project build-config value for '{}' was not retained by config folding",
                error.name().as_str()
            )),
            string_table,
        );
    }
    let key = string_table.intern(error.name().as_str());
    let provided_argument_index = match error.value_location() {
        Some(BuildConfigValueLocation::Command(location)) => Some(location.argument_index()),
        _ => None,
    };
    let diagnostic = if let Some((first, conflicting)) = error.conflict_facts() {
        let first_description = string_table.get_or_intern(describe_build_config_contract(first));
        let conflicting_description =
            string_table.get_or_intern(describe_build_config_contract(conflicting));
        CompilerDiagnostic::invalid_config_reason(
            Some(key),
            InvalidConfigReason::ConfigContractConflict {
                first: first_description,
                conflicting: conflicting_description,
            },
            conflicting.location().clone(),
        )
        .with_labels(vec![
            DiagnosticLabel::primary(conflicting.location().clone()),
            DiagnosticLabel::secondary(
                first.location().clone(),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ),
        ])
    } else if let Some(contract) = error.contract_fact() {
        if let Some(provided) = error.provided_type() {
            let provided_name = string_table.intern(provided.name());
            let expected_name =
                string_table.get_or_intern(build_input_type_name(contract.value_type()));
            let mut labels = vec![DiagnosticLabel::primary(contract.location().clone())];
            if let Some(BuildConfigValueLocation::Source(location)) = error.value_location() {
                labels.push(DiagnosticLabel::secondary(location.clone(), None));
            }
            CompilerDiagnostic::invalid_config_reason(
                Some(key),
                InvalidConfigReason::ConfigInputTypeMismatch {
                    provided: provided_name,
                    expected: expected_name,
                    provided_argument_index,
                },
                contract.location().clone(),
            )
            .with_labels(labels)
        } else {
            CompilerDiagnostic::invalid_config_reason(
                Some(key),
                InvalidConfigReason::MissingConfigInput,
                contract.location().clone(),
            )
        }
    } else {
        CompilerDiagnostic::invalid_config_reason(
            Some(key),
            InvalidConfigReason::UnknownBuildConfigInput {
                key,
                provided_argument_index,
            },
            fallback_location,
        )
    };

    CompilerMessages::from_diagnostic(diagnostic, string_table.clone())
}
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_boundary_build_config(
    source_facts: &[BuildConfigContractFact],
    fixed_project_facts: &[BuildConfigContractFact],
    direct_project_facts: &[BuildConfigContractFact],
    explicit_inputs: &BuildConfigInputSet,
    builder_globals: &BuilderConfigGlobalSet,
    fallback_location: SourceLocation,
    string_table: &mut StringTable,
) -> Result<ResolvedBuildConfigMap, CompilerMessages> {
    resolve_build_config_values(
        source_facts,
        fixed_project_facts,
        direct_project_facts,
        explicit_inputs,
        builder_globals,
    )
    .map_err(|error| build_config_resolution_messages(error, fallback_location, string_table))
}
