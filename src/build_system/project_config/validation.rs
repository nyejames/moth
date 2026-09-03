//! Config value validation and application helpers for Stage 0 project config loading.
//!
//! WHAT: validates the compiler config service's folded declarations against the recursive
//! project schema and named builder/tooling section schemas, then applies accepted results
//! to [`Config`]. Private helper constants of any shape stay folded without becoming settings.
//! WHY: the compiler service owns tokenization through folding; this module walks those owned
//! values against the builder surface's schema nodes without inspecting compiler AST internals.

use crate::build_system::output::{
    ValidatedDirectoryOutputSettings, ValidatedOutputFolder, canonical_output_root_for_identity,
    classify_output_folder, output_path_identity, validate_output_folder_containment,
};
use crate::compiler_frontend::single_source_compilation::{
    CompiledConfigSource, FoldedConfigDeclaration,
};

use crate::builder_surface::config_schema::{
    ConfigFieldShape, ConfigSchema, ConfigSchemaField, ConfigSchemaFieldId, ConfigSchemaNodeId,
    ConfigSchemas, NamedConfigSectionSchema, UnknownFieldPolicy,
};
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidConfigReason, InvalidOutputFolderReason,
};
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicFoldedField, PublicFoldedValue,
};
use crate::compiler_frontend::keywords::is_valid_identifier;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::settings::{
    Config, HtmlSectionConfig, MAX_TEMPLATE_CONST_LOOP_ITERATIONS, ProjectMetadataField,
    TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY,
};

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

// -------------------------
//  Validation Entry Point
// -------------------------

#[derive(Debug)]
pub(super) enum ConfigApplyError {
    Diagnostics(Vec<CompilerDiagnostic>),
    Compiler(CompilerError),
}

/// Validate folded config declarations and apply accepted values to the runtime config.
///
/// WHY: this keeps required-structure checks and value semantics in one place over the compiler
/// service's folded declaration boundary. Duplicate authored names are a compiler-service
/// invariant, not a second user-facing validation pass.
pub(super) fn validate_and_apply_config_declarations(
    config: &mut Config,
    compiled_config: &CompiledConfigSource,
    config_schemas: &ConfigSchemas,
    string_table: &mut StringTable,
) -> Result<(), ConfigApplyError> {
    debug_assert!(
        {
            let mut names = HashSet::new();
            compiled_config
                .declarations
                .iter()
                .all(|declaration| names.insert(declaration.name))
        },
        "compiled config declarations must have unique names"
    );

    let mut errors = Vec::new();
    let mut saw_project_record = false;
    let mut seen_sections = HashSet::new();

    let project_field_indexes = SchemaFieldIndexes::build(config_schemas.project());
    let mut section_indexes = Vec::with_capacity(config_schemas.project_sections().len());
    for section in config_schemas.project_sections() {
        section_indexes.push(SchemaFieldIndexes::build(&section.schema));
    }

    for declaration in &compiled_config.declarations {
        let key = string_table.resolve(declaration.name).to_string();

        if key == "project" {
            saw_project_record = true;
            if matches!(declaration.value, PublicFoldedValue::Record(_)) {
                validate_and_apply_project_record(
                    config,
                    declaration,
                    config_schemas.project(),
                    &project_field_indexes,
                    string_table,
                    &mut errors,
                );
            } else {
                errors.push(config_diagnostic(
                    Some(declaration.name),
                    InvalidConfigReason::InvalidConfigValueShape {
                        expected: string_table.intern("a record value"),
                    },
                    declaration.location.clone(),
                ));
            }
            continue;
        }

        if let Some((section_index, section)) = config_schemas
            .project_sections()
            .iter()
            .enumerate()
            .find(|(_, section)| section.name == key)
        {
            seen_sections.insert(section.name);
            if matches!(declaration.value, PublicFoldedValue::Record(_)) {
                if let Err(error) = validate_and_apply_named_section(
                    config,
                    declaration,
                    section,
                    &section_indexes[section_index],
                    string_table,
                    &mut errors,
                ) {
                    return Err(ConfigApplyError::Compiler(error));
                }
            } else {
                errors.push(config_diagnostic(
                    Some(declaration.name),
                    InvalidConfigReason::InvalidConfigValueShape {
                        expected: string_table.intern("a record value"),
                    },
                    declaration.location.clone(),
                ));
            }
            continue;
        }

        if is_retired_flat_config_key(&key) {
            errors.push(config_diagnostic(
                Some(declaration.name),
                InvalidConfigReason::UnknownKey {
                    key: declaration.name,
                },
                declaration.name_location.clone(),
            ));
            continue;
        }

        // Unregistered top-level constants are private folding helpers of any value shape.
    }

    if !saw_project_record {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::MissingProjectRecord,
            compiled_config
                .declarations
                .first()
                .map(|declaration| declaration.location.clone())
                .unwrap_or_else(|| config.setting_location_or_config_file("project", string_table)),
        ));
    }

    for section in config_schemas.project_sections() {
        if section.required && !seen_sections.contains(section.name) {
            errors.push(config_diagnostic(
                None,
                InvalidConfigReason::MissingActiveBuilderSection {
                    section: string_table.intern(section.name),
                },
                config.setting_location_or_config_file(section.name, string_table),
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ConfigApplyError::Diagnostics(errors))
    }
}

// -------------------------
//  Schema Field Index
// -------------------------

/// Transient field-name index over every node of one schema.
///
/// WHAT: maps each node's declared field names to their dense schema field IDs.
/// WHY: the recursive walk resolves field names by dense ID lookup without scanning the schema
///      vectors per field; the index is rebuilt once per validation operation.
struct SchemaFieldIndexes {
    by_node: Vec<HashMap<&'static str, ConfigSchemaFieldId>>,
}

impl SchemaFieldIndexes {
    fn build(schema: &ConfigSchema) -> Self {
        let mut by_node = Vec::with_capacity(schema.nodes().len());

        for node in schema.nodes() {
            let mut index = HashMap::with_capacity(node.field_ids().len());
            for field_id in node.field_ids() {
                index
                    .entry(schema.field(*field_id).name)
                    .or_insert(*field_id);
            }
            by_node.push(index);
        }

        Self { by_node }
    }

    fn lookup(&self, node: ConfigSchemaNodeId, name: &str) -> Option<ConfigSchemaFieldId> {
        self.by_node[node.0].get(name).copied()
    }
}

// -------------------------
//  Grouped Project Record Validation
// -------------------------

/// Validate a grouped `project #= |...|` record against the project schema root and apply it.
fn validate_and_apply_project_record(
    config: &mut Config,
    declaration: &FoldedConfigDeclaration,
    project_schema: &ConfigSchema,
    field_indexes: &SchemaFieldIndexes,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) {
    let location = declaration.location.clone();
    let view = ConfigSchemaView {
        schema: project_schema,
        field_indexes,
    };
    let mut diagnostics = ValueDiagnosticContext {
        location: &location,
        string_table,
        errors,
    };

    if let ValueCheck::Valid(ValidatedConfigValue::Record(fields)) = validate_record_fields(
        &view,
        &mut diagnostics,
        project_schema.root(),
        &declaration.value,
        Some(&declaration.direct_field_locations),
    ) && let Err(apply_errors) = apply_project_record_fields(config, fields, string_table)
    {
        errors.extend(apply_errors);
    }
}

fn apply_project_record_fields(
    config: &mut Config,
    fields: Vec<ValidatedRecordField>,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    let mut errors = Vec::new();
    config.extra_project_fields.clear();

    for field in fields {
        match field.name.as_str() {
            "name"
            | "entry_root"
            | "version"
            | "author"
            | "license"
            | "template_const_loop_iteration_limit" => {
                config
                    .setting_locations
                    .insert(field.name.clone(), field.location.clone());
            }
            _ => {}
        }

        match (field.name.as_str(), field.value) {
            ("name", ValidatedConfigValue::String(value)) => {
                if let Err(diagnostic) =
                    assign_project_name(config, value, &field.location, string_table)
                {
                    errors.push(diagnostic);
                }
            }

            ("entry_root", ValidatedConfigValue::String(value)) => {
                if let Err(mut diagnostics) =
                    assign_entry_root(config, value, &field.location, string_table)
                {
                    errors.append(&mut diagnostics);
                }
            }

            ("version", ValidatedConfigValue::String(value)) => config.version = Some(value),

            ("version", ValidatedConfigValue::OptionNone) => config.version = None,

            ("author", ValidatedConfigValue::String(value)) => config.author = Some(value),

            ("author", ValidatedConfigValue::OptionNone) => config.author = None,

            ("license", ValidatedConfigValue::String(value)) => config.license = Some(value),

            ("license", ValidatedConfigValue::OptionNone) => config.license = None,

            ("template_const_loop_iteration_limit", ValidatedConfigValue::Int(value)) => {
                match validate_template_const_loop_iteration_limit(
                    value,
                    &field.location,
                    string_table,
                ) {
                    Ok(limit) => config.template_const_loop_iteration_limit = limit,

                    Err(mut limit_errors) => errors.append(&mut limit_errors),
                }
            }

            (_, ValidatedConfigValue::Preserved(preserved)) => {
                let preserved = *preserved;
                config.extra_project_fields.push(ProjectMetadataField {
                    name: field.name,
                    type_identity: preserved.type_identity,
                    value: preserved.value,
                    location: preserved.location,
                });
            }

            _ => {}
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn assign_project_name(
    config: &mut Config,
    value: String,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), CompilerDiagnostic> {
    if value.is_empty() || !is_valid_identifier(&value) {
        return Err(config_diagnostic(
            Some(string_table.intern("name")),
            InvalidConfigReason::InvalidProjectSettingValue {
                value: string_table.intern(&value),
                expected: string_table.intern("a valid Moth project identifier"),
            },
            location.clone(),
        ));
    }

    config.project_name = value;
    Ok(())
}

// -------------------------
//  Named Builder Section Validation
// -------------------------

fn validate_and_apply_named_section(
    config: &mut Config,
    declaration: &FoldedConfigDeclaration,
    section: &NamedConfigSectionSchema,
    field_indexes: &SchemaFieldIndexes,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) -> Result<(), CompilerError> {
    let location = declaration.location.clone();
    let view = ConfigSchemaView {
        schema: &section.schema,
        field_indexes,
    };
    let mut diagnostics = ValueDiagnosticContext {
        location: &location,
        string_table,
        errors,
    };

    if let ValueCheck::Valid(ValidatedConfigValue::Record(fields)) = validate_record_fields(
        &view,
        &mut diagnostics,
        section.schema.root(),
        &declaration.value,
        Some(&declaration.direct_field_locations),
    ) && section.name == "html"
    {
        apply_html_section_fields(config, fields, &location)?;
    }

    Ok(())
}

/// Apply the validated fields of one grouped html section record.
///
/// WHAT: routes each owned field through the html application path in authored order, so the
///       last authored spelling of a field owns the stored value.
/// WHY: the HTML builder's typed section is the only settings store for html keys.
fn apply_html_section_fields(
    config: &mut Config,
    fields: Vec<ValidatedRecordField>,
    location: &SourceLocation,
) -> Result<(), CompilerError> {
    for field in fields {
        apply_html_section_field(config, field, location)?;
    }
    Ok(())
}

/// Apply one validated html section field to the config.
///
/// WHAT: stores each html builder field on the typed [`HtmlSectionConfig`] result. The HTML
///       builder's readers consume that section directly.
/// WHY: html values must not be stringified into a settings map. An unmatched validated field
///       means the schema and typed storage drifted, which is a compiler bug.
fn apply_html_section_field(
    config: &mut Config,
    field: ValidatedRecordField,
    location: &SourceLocation,
) -> Result<(), CompilerError> {
    let name = field.name;
    match (name.as_str(), field.value) {
        ("origin", ValidatedConfigValue::String(text)) => {
            config.html_section.origin = Some(text);
        }
        ("page_url_style", ValidatedConfigValue::String(text)) => {
            config.html_section.page_url_style = Some(text);
        }
        ("redirect_index_html", ValidatedConfigValue::Bool(enabled)) => {
            config.html_section.redirect_index_html = Some(enabled);
        }
        ("html_lang", ValidatedConfigValue::String(text)) => {
            config.html_section.html_lang = Some(text);
        }
        ("html_title_prefix", ValidatedConfigValue::String(text)) => {
            config.html_section.html_title_prefix = Some(text);
        }
        ("html_title_postfix", ValidatedConfigValue::String(text)) => {
            config.html_section.html_title_postfix = Some(text);
        }
        ("html_favicon", ValidatedConfigValue::String(text)) => {
            config.html_section.html_favicon = Some(text);
        }
        ("html_inject_charset", ValidatedConfigValue::Bool(enabled)) => {
            config.html_section.html_inject_charset = Some(enabled);
        }
        ("html_inject_viewport", ValidatedConfigValue::Bool(enabled)) => {
            config.html_section.html_inject_viewport = Some(enabled);
        }
        ("html_inject_color_scheme", ValidatedConfigValue::Bool(enabled)) => {
            config.html_section.html_inject_color_scheme = Some(enabled);
        }
        ("html_inject_core_css", ValidatedConfigValue::Bool(enabled)) => {
            config.html_section.html_inject_core_css = Some(enabled);
        }
        ("html_body_style", ValidatedConfigValue::String(text)) => {
            config.html_section.html_body_style = Some(text);
        }
        ("dev_output", ValidatedConfigValue::String(text)) => {
            config.html_section.dev_output = Some(text);
        }
        ("release_output", ValidatedConfigValue::String(text)) => {
            config.html_section.release_output = Some(text);
        }
        (unknown, _) => {
            return Err(CompilerError::compiler_error(format!(
                "html section applied unknown validated field '{unknown}'"
            )));
        }
    }

    config.setting_locations.insert(name, location.clone());
    Ok(())
}

// -------------------------
//  Recursive Value Validation
// -------------------------

/// Shared schema state for one recursive value-validation walk.
///
/// WHAT: bundles the schema and its transient field-name index.
/// WHY: the recursive helpers read schema state through one shared view while the diagnostic
/// lane stays separately mutable.
struct ConfigSchemaView<'a> {
    schema: &'a ConfigSchema,
    field_indexes: &'a SchemaFieldIndexes,
}

/// Mutable diagnostic lane for one recursive value-validation walk.
///
/// WHAT: the value location plus the error lane with the string table.
/// WHY: nested diagnostics keep underlining the declaration's value location without threading
/// the location and error lane through every helper as bare parameters.
struct ValueDiagnosticContext<'a> {
    location: &'a SourceLocation,
    string_table: &'a mut StringTable,
    errors: &'a mut Vec<CompilerDiagnostic>,
}

/// A config value that has been validated against its schema field.
///
/// WHY: carrying the validated shape lets `apply_validated_config_value` dispatch cleanly
/// without re-inspecting the folded value.
#[derive(Debug, PartialEq, Eq)]
enum ValidatedConfigValue {
    String(String),
    Int(i32),
    Float(crate::compiler_frontend::folded_value::FiniteFloat),
    Bool(bool),
    Char(char),
    OptionNone,
    Record(Vec<ValidatedRecordField>),
    Collection(Vec<ValidatedConfigValue>),
    /// An open-record field retained without a compiler-owned schema leaf.
    Preserved(Box<PreservedConfigField>),
}

/// Canonical type, folded value and initializer location for one open project field.
#[derive(Debug, PartialEq, Eq)]
struct PreservedConfigField {
    type_identity: CanonicalTypeIdentity,
    value: PublicFoldedValue,
    location: SourceLocation,
}

/// One validated field of a record value, in authored field order.
#[derive(Debug, PartialEq, Eq)]
struct ValidatedRecordField {
    name: String,
    value: ValidatedConfigValue,
    location: SourceLocation,
}

/// The outcome of checking one folded value against its field shape.
enum ValueCheck {
    /// The value matched and produced its validated form.
    Valid(ValidatedConfigValue),
    /// The value's own shape or domain is wrong; the caller reports the field's reason.
    Mismatch,
    /// Nested record contents failed and their diagnostics are already pushed.
    Reported,
}

/// Validate one folded value against one field shape and extract its validated value.
///
/// Shape mismatches and closed-domain violations return [`ValueCheck::Mismatch`] so the caller
/// reports the owning field's reason; nested record policy failures push their diagnostics
/// directly and return [`ValueCheck::Reported`].
fn validate_value(
    view: &ConfigSchemaView<'_>,
    diagnostics: &mut ValueDiagnosticContext<'_>,
    key: Option<StringId>,
    shape: &ConfigFieldShape,
    field: &ConfigSchemaField,
    value: &PublicFoldedValue,
) -> ValueCheck {
    match shape {
        ConfigFieldShape::String => match extract_string_value(value) {
            Some(text) if string_in_closed_domain(&text, field.allowed_strings) => {
                ValueCheck::Valid(ValidatedConfigValue::String(text))
            }

            _ => ValueCheck::Mismatch,
        },

        ConfigFieldShape::Int => match extract_int_value(value) {
            Some(int_value) => ValueCheck::Valid(ValidatedConfigValue::Int(int_value)),

            None => ValueCheck::Mismatch,
        },

        ConfigFieldShape::Float => match extract_float_value(value) {
            Some(float_value) => ValueCheck::Valid(ValidatedConfigValue::Float(float_value)),

            None => ValueCheck::Mismatch,
        },

        ConfigFieldShape::Bool => match extract_bool_value(value) {
            Some(bool_value) => ValueCheck::Valid(ValidatedConfigValue::Bool(bool_value)),

            None => ValueCheck::Mismatch,
        },

        ConfigFieldShape::Char => match extract_char_value(value) {
            Some(char_value) => ValueCheck::Valid(ValidatedConfigValue::Char(char_value)),

            None => ValueCheck::Mismatch,
        },

        ConfigFieldShape::Record(node_id) => {
            validate_record_fields(view, diagnostics, *node_id, value, None)
        }

        ConfigFieldShape::Collection(element) => {
            validate_collection(view, diagnostics, key, element, field, value)
        }

        ConfigFieldShape::Optional(inner) => match value {
            PublicFoldedValue::OptionNone => ValueCheck::Valid(ValidatedConfigValue::OptionNone),

            PublicFoldedValue::OptionSome(present) => {
                validate_value(view, diagnostics, key, inner, field, present)
            }

            // A bare value is accepted as a present optional.
            _ => validate_value(view, diagnostics, key, inner, field, value),
        },
    }
}

/// Collections report the element shape, or keep nested diagnostics already pushed by
/// record-valued elements. A scalar is not promoted to a one-element collection.
fn validate_collection(
    view: &ConfigSchemaView<'_>,
    diagnostics: &mut ValueDiagnosticContext<'_>,
    key: Option<StringId>,
    element_shape: &ConfigFieldShape,
    field: &ConfigSchemaField,
    value: &PublicFoldedValue,
) -> ValueCheck {
    let PublicFoldedValue::Collection(elements) = value else {
        return ValueCheck::Mismatch;
    };

    let mut values = Vec::with_capacity(elements.len());
    for element_value in elements {
        match validate_value(view, diagnostics, key, element_shape, field, element_value) {
            ValueCheck::Valid(validated) => values.push(validated),
            ValueCheck::Mismatch => {
                push_shape_failure(diagnostics, key, element_shape, field);
                return ValueCheck::Reported;
            }

            ValueCheck::Reported => return ValueCheck::Reported,
        }
    }

    ValueCheck::Valid(ValidatedConfigValue::Collection(values))
}

/// Validate one record value against one schema node, field by field.
///
/// Known authored fields validate against their schema fields; unknown names follow the node's
/// [`UnknownFieldPolicy`]. Omitted required fields are rejected, and omitted optional fields
/// with a schema default contribute that default to the validated record.
fn validate_record_fields(
    view: &ConfigSchemaView<'_>,
    diagnostics: &mut ValueDiagnosticContext<'_>,
    node_id: ConfigSchemaNodeId,
    value: &PublicFoldedValue,
    field_locations: Option<&[SourceLocation]>,
) -> ValueCheck {
    let PublicFoldedValue::Record(authored_fields) = value else {
        return ValueCheck::Mismatch;
    };
    let field_locations = field_locations.filter(|locations| !locations.is_empty());
    debug_assert!(
        field_locations
            .map(|locations| locations.len() == authored_fields.len())
            .unwrap_or(true),
        "direct field locations must align with folded record fields"
    );

    let node = view.schema.node(node_id);
    let record_name = diagnostics.string_table.intern(node.name);
    let mut validated_fields = Vec::with_capacity(authored_fields.len());
    let mut failed = false;

    for (index, authored_field) in authored_fields.iter().enumerate() {
        let field_location = field_locations
            .and_then(|locations| locations.get(index))
            .cloned()
            .unwrap_or_else(|| diagnostics.location.clone());

        match view.field_indexes.lookup(node_id, &authored_field.name) {
            Some(field_id) => {
                let field = view.schema.field(field_id);
                let field_key = diagnostics.string_table.intern(&authored_field.name);

                match validate_value(
                    view,
                    diagnostics,
                    Some(field_key),
                    &field.shape,
                    field,
                    &authored_field.value,
                ) {
                    ValueCheck::Valid(validated) => {
                        validated_fields.push(ValidatedRecordField {
                            name: authored_field.name.clone(),
                            value: validated,
                            location: field_location,
                        });
                    }

                    ValueCheck::Mismatch => {
                        failed = true;
                        push_shape_failure(diagnostics, Some(field_key), &field.shape, field);
                    }

                    ValueCheck::Reported => failed = true,
                }
            }

            None => match node.unknown_fields {
                UnknownFieldPolicy::Preserve => {
                    match project_supported_metadata(&authored_field.value) {
                        Some(value) => {
                            validated_fields.push(ValidatedRecordField {
                                name: authored_field.name.clone(),
                                value: ValidatedConfigValue::Preserved(Box::new(
                                    PreservedConfigField {
                                        type_identity: authored_field.type_identity.clone(),
                                        value,
                                        location: field_location.clone(),
                                    },
                                )),
                                location: field_location,
                            });
                        }

                        None => {
                            failed = true;
                            diagnostics.errors.push(config_diagnostic(
                                None,
                                InvalidConfigReason::InvalidConfigValueShape {
                                    expected: diagnostics.string_table.intern(
                                        "a folded scalar, optional, nested record, collection or template string",
                                    ),
                                },
                                field_location,
                            ));
                        }
                    }
                }

                UnknownFieldPolicy::Reject => {
                    failed = true;
                    diagnostics.errors.push(config_diagnostic(
                        None,
                        InvalidConfigReason::UnknownRecordField {
                            record: record_name,
                            field: diagnostics.string_table.intern(&authored_field.name),
                        },
                        field_location,
                    ));
                }
            },
        }
    }

    for field_id in node.field_ids() {
        let field = view.schema.field(*field_id);

        if authored_fields
            .iter()
            .any(|authored| authored.name == field.name)
        {
            continue;
        }

        if field.required {
            failed = true;
            diagnostics.errors.push(config_diagnostic(
                None,
                InvalidConfigReason::MissingRequiredRecordField {
                    record: record_name,
                    field: diagnostics.string_table.intern(field.name),
                },
                diagnostics.location.clone(),
            ));
            continue;
        }

        if let Some(default) = &field.default {
            let field_key = diagnostics.string_table.intern(field.name);

            match validate_value(
                view,
                diagnostics,
                Some(field_key),
                &field.shape,
                field,
                default,
            ) {
                ValueCheck::Valid(validated_default) => {
                    validated_fields.push(ValidatedRecordField {
                        name: field.name.to_string(),
                        value: validated_default,
                        location: diagnostics.location.clone(),
                    });
                }

                ValueCheck::Mismatch => {
                    failed = true;
                    push_shape_failure(diagnostics, Some(field_key), &field.shape, field);
                }

                ValueCheck::Reported => failed = true,
            }
        }
    }

    if failed {
        return ValueCheck::Reported;
    }

    ValueCheck::Valid(ValidatedConfigValue::Record(validated_fields))
}

fn push_shape_failure(
    diagnostics: &mut ValueDiagnosticContext<'_>,
    key: Option<StringId>,
    shape: &ConfigFieldShape,
    field: &ConfigSchemaField,
) {
    diagnostics.errors.push(config_diagnostic(
        key,
        shape_failure_reason(shape, field, diagnostics.string_table),
        diagnostics.location.clone(),
    ));
}

fn string_in_closed_domain(text: &str, allowed_strings: Option<&'static [&'static str]>) -> bool {
    match allowed_strings {
        Some(allowed) => allowed.contains(&text),
        None => true,
    }
}

/// The diagnostic reason for one shape or domain failure on a schema field.
///
/// String fields with a closed domain report the allowed set; every other shape reports its
/// own description, mirroring the long-standing closed-set diagnostic text.
fn shape_failure_reason(
    shape: &ConfigFieldShape,
    field: &ConfigSchemaField,
    string_table: &mut StringTable,
) -> InvalidConfigReason {
    let expected = match (shape, field.allowed_strings) {
        (ConfigFieldShape::String, Some(allowed)) => format_closed_string_set_expected(allowed),
        _ => shape.describe(),
    };

    InvalidConfigReason::InvalidConfigValueShape {
        expected: string_table.intern(&expected),
    }
}

/// Convert a supported open-project field into ordinary folded data.
///
/// Concrete template wrappers become strings. Unresolved slots, structural children and
/// other non-concrete composition are rejected so `@project` never exports a template
/// transducer.
fn project_supported_metadata(value: &PublicFoldedValue) -> Option<PublicFoldedValue> {
    match value {
        PublicFoldedValue::Int(value) => Some(PublicFoldedValue::Int(*value)),
        PublicFoldedValue::Float(value) => Some(PublicFoldedValue::Float(value.clone())),
        PublicFoldedValue::Bool(value) => Some(PublicFoldedValue::Bool(*value)),
        PublicFoldedValue::Char(value) => Some(PublicFoldedValue::Char(*value)),
        PublicFoldedValue::String(value) => Some(PublicFoldedValue::String(value.clone())),
        PublicFoldedValue::OptionNone => Some(PublicFoldedValue::OptionNone),
        PublicFoldedValue::ConstTemplate(template) => concrete_template_string(template)
            .map(|text| PublicFoldedValue::String(OwnedFoldedString::Text(text))),
        PublicFoldedValue::OptionSome(inner) => project_supported_metadata(inner)
            .map(|value| PublicFoldedValue::OptionSome(Box::new(value))),
        PublicFoldedValue::Collection(items) => {
            let mut converted = Vec::with_capacity(items.len());
            for item in items {
                converted.push(project_supported_metadata(item)?);
            }
            Some(PublicFoldedValue::Collection(converted))
        }
        PublicFoldedValue::Record(fields) => {
            let mut converted = Vec::with_capacity(fields.len());
            for field in fields {
                converted.push(PublicFoldedField {
                    name: field.name.clone(),
                    type_identity: field.type_identity.clone(),
                    value: project_supported_metadata(&field.value)?,
                });
            }
            Some(PublicFoldedValue::Record(converted))
        }
        PublicFoldedValue::Choice { .. } | PublicFoldedValue::Range { .. } => None,
    }
}

/// Extract a string from a folded config value.
///
/// WHY: core path/metadata keys and backend string keys must not accept bool/int/float/char
/// by accidental stringification. Concrete text, including text-only piece sequences, is owned
/// by [`OwnedFoldedString::into_text`]. Folded `[:text]` wrappers still apply as strings here.
fn extract_string_value(value: &PublicFoldedValue) -> Option<String> {
    match value {
        PublicFoldedValue::String(value) => value.clone().into_text(),
        PublicFoldedValue::ConstTemplate(template) => concrete_template_string(template),
        _ => None,
    }
}

fn concrete_template_string(template: &PublicConstTemplate) -> Option<String> {
    if !matches!(template.kind, PublicConstTemplateKind::Wrapper)
        || !template.conditional_child_wrappers.is_empty()
    {
        return None;
    }

    let mut text = String::new();
    for piece in &template.pieces {
        match piece {
            PublicConstTemplatePiece::Text(owned) => text.push_str(&owned.clone().into_text()?),
            _ => return None,
        }
    }

    Some(text)
}

/// Format a human-readable expected-value description for a closed string set.
///
/// WHY: the diagnostic renderer needs a concrete message that lists the allowed strings
/// so users know exactly which values are accepted.
fn format_closed_string_set_expected(allowed: &[&str]) -> String {
    if allowed.len() == 1 {
        format!("\"{}\"", allowed[0])
    } else {
        let quoted: Vec<String> = allowed.iter().map(|s| format!("\"{}\"", s)).collect();
        format!("one of: {}", quoted.join(", "))
    }
}

/// Extract an integer value.
///
/// WHY: numeric config keys must not accept floats, bools, or strings through coercion or
/// stringification. The folded declaration boundary already resolved coercions to their
/// inner value.
fn extract_int_value(value: &PublicFoldedValue) -> Option<i32> {
    match value {
        PublicFoldedValue::Int(value) => Some(*value),
        _ => None,
    }
}

/// Extract a finite floating-point value without accepting integer promotion.
fn extract_float_value(
    value: &PublicFoldedValue,
) -> Option<crate::compiler_frontend::folded_value::FiniteFloat> {
    match value {
        PublicFoldedValue::Float(value) => Some(value.clone()),
        _ => None,
    }
}

/// Extract a character value without accepting string coercion.
fn extract_char_value(value: &PublicFoldedValue) -> Option<char> {
    match value {
        PublicFoldedValue::Char(value) => Some(*value),
        _ => None,
    }
}

/// Extract a boolean value.
///
/// WHY: backend bool keys require actual boolean literals, not string representations.
fn extract_bool_value(value: &PublicFoldedValue) -> Option<bool> {
    match value {
        PublicFoldedValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn validate_template_const_loop_iteration_limit(
    value: i32,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<usize, Vec<CompilerDiagnostic>> {
    if value <= 0 {
        return Err(vec![config_diagnostic(
            Some(string_table.intern(TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY)),
            InvalidConfigReason::InvalidProjectSettingValue {
                value: string_table.intern(&value.to_string()),
                expected: string_table.intern("a positive integer"),
            },
            location.clone(),
        )]);
    }

    if value > MAX_TEMPLATE_CONST_LOOP_ITERATIONS as i32 {
        return Err(vec![config_diagnostic(
            Some(string_table.intern(TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY)),
            InvalidConfigReason::InvalidProjectSettingValue {
                value: string_table.intern(&value.to_string()),
                expected: string_table.intern("an integer no greater than 1000000"),
            },
            location.clone(),
        )]);
    }

    Ok(value as usize)
}

fn config_diagnostic(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(key, reason, location)
}

// -------------------------
//  Directory Output Setting Validation
// -------------------------

/// Validate directory output settings after config values are applied.
///
/// WHAT: rejects the effective development and release output folders that are empty,
/// absolute, parent-traversing, current-directory, equal to the project root, inside
/// `entry_root`, or equal to each other.
/// WHY: directory output roots must be safe and distinct before any output writing or
/// cleanup runs. Single-file output stays separate because it writes to the command
/// working directory.
pub(crate) fn validate_directory_output_settings(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<ValidatedDirectoryOutputSettings, Vec<CompilerDiagnostic>> {
    let mut errors = Vec::new();

    let project_root = &config.entry_dir;
    // The transitional empty or "." entry root form means the entry root covers the whole
    // project, so output containment is validated against the project root only. When
    // entry_root is a strict subdirectory, enforce that the output is outside it.
    let resolved_entry_root =
        if config.entry_root.as_os_str().is_empty() || config.entry_root == Path::new(".") {
            None
        } else {
            Some(project_root.join(&config.entry_root))
        };

    let (dev_setting, release_setting) = effective_output_settings(config);

    let dev = validate_one_output_folder(
        dev_setting.key,
        &dev_setting.folder,
        project_root,
        resolved_entry_root.as_deref(),
        config,
        string_table,
        &mut errors,
    );
    let release = validate_one_output_folder(
        release_setting.key,
        &release_setting.folder,
        project_root,
        resolved_entry_root.as_deref(),
        config,
        string_table,
        &mut errors,
    );

    // Only check distinctness when both folders individually passed validation.
    if let (Some(dev), Some(release)) = (&dev, &release) {
        validate_output_folders_distinct(
            dev,
            release,
            &dev_setting,
            &release_setting,
            config,
            string_table,
            &mut errors,
        );
    }

    let (Some(dev), Some(release)) = (dev, release) else {
        return Err(errors);
    };

    if errors.is_empty() {
        Ok(ValidatedDirectoryOutputSettings { dev, release })
    } else {
        Err(errors)
    }
}

/// One effective output root with the diagnostic key of the authoring shape that owns it.
struct EffectiveOutputSetting {
    key: &'static str,
    folder: PathBuf,
}

/// Resolve the development and release output roots this config will actually use.
///
/// WHAT: a validated grouped html section owns the output roots through its typed settings,
/// with the section schema's defaults already applied for omitted fields; an absent section
/// resolves through the same section defaults.
/// WHY: dual-mode output resolution must validate the roots that will actually be written
/// and point diagnostics at the authoring shape that provided each value.
fn effective_output_settings(config: &Config) -> (EffectiveOutputSetting, EffectiveOutputSetting) {
    let dev = EffectiveOutputSetting {
        key: "dev_output",
        folder: PathBuf::from(
            config
                .html_section
                .dev_output
                .as_deref()
                .unwrap_or(HtmlSectionConfig::DEFAULT_DEV_OUTPUT),
        ),
    };

    let release = EffectiveOutputSetting {
        key: "release_output",
        folder: PathBuf::from(
            config
                .html_section
                .release_output
                .as_deref()
                .unwrap_or(HtmlSectionConfig::DEFAULT_RELEASE_OUTPUT),
        ),
    };

    (dev, release)
}

/// Validate one output folder, returning the validated folder when it is valid.
fn validate_one_output_folder(
    key: &str,
    folder: &Path,
    project_root: &Path,
    resolved_entry_root: Option<&Path>,
    config: &Config,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) -> Option<ValidatedOutputFolder> {
    let location = config.setting_location_or_config_file(key, string_table);

    match classify_output_folder(folder, project_root, resolved_entry_root) {
        Ok(mut valid) => {
            if let Err(reason) =
                validate_output_folder_containment(&valid, project_root, resolved_entry_root)
            {
                let folder_id = Some(string_table.intern(&folder.to_string_lossy()));
                errors.push(output_folder_diagnostic(
                    key,
                    folder_id,
                    reason,
                    location,
                    string_table,
                ));
                return None;
            }

            valid.location = location;
            Some(valid)
        }
        Err(reason) => {
            let folder_id = (!matches!(reason, InvalidOutputFolderReason::Empty))
                .then(|| string_table.intern(&folder.to_string_lossy()));
            errors.push(output_folder_diagnostic(
                key,
                folder_id,
                reason,
                location,
                string_table,
            ));
            None
        }
    }
}

/// Reject development and release output roots that share one output identity.
fn validate_output_folders_distinct(
    dev: &ValidatedOutputFolder,
    release: &ValidatedOutputFolder,
    dev_setting: &EffectiveOutputSetting,
    release_setting: &EffectiveOutputSetting,
    config: &Config,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) {
    // Both folders passed classification, so their relative paths are guaranteed valid.
    let dev_identity = output_path_identity(&dev.relative_path)
        .expect("dev folder was already validated as a relative output path");
    let release_identity = output_path_identity(&release.relative_path)
        .expect("release folder was already validated as a relative output path");

    let canonical_roots_match = canonical_output_root_for_identity(&dev.resolved_path)
        .ok()
        .zip(canonical_output_root_for_identity(&release.resolved_path).ok())
        .is_some_and(|(dev_root, release_root)| dev_root == release_root);

    if dev_identity == release_identity || canonical_roots_match {
        let location = config.setting_location_or_config_file(dev_setting.key, string_table);
        errors.push(CompilerDiagnostic::invalid_config_reason(
            Some(string_table.intern(dev_setting.key)),
            InvalidConfigReason::OutputFoldersNotDistinct {
                dev_folder: string_table.intern(&dev_setting.folder.to_string_lossy()),
                release_folder: string_table.intern(&release_setting.folder.to_string_lossy()),
            },
            location,
        ));
    }
}

fn assign_entry_root(
    config: &mut Config,
    value: String,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    if entry_root_escapes_project(&value, &config.entry_dir) {
        return Err(vec![config_diagnostic(
            Some(string_table.intern("entry_root")),
            InvalidConfigReason::InvalidProjectSettingValue {
                value: string_table.intern(&value),
                expected: string_table.intern(
                    "a relative directory strictly below the project root, with no parent-directory segments",
                ),
            },
            location.clone(),
        )]);
    }

    config.entry_root = PathBuf::from(value);
    Ok(())
}

fn is_retired_flat_config_key(key: &str) -> bool {
    matches!(
        key,
        "dev_folder" | "output_folder" | "package_folders" | "project_name"
    )
}

fn entry_root_escapes_project(value: &str, project_root: &Path) -> bool {
    if value.is_empty() || value == "." {
        return true;
    }

    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return true;
    }

    let Ok(canonical_project) = project_root.canonicalize() else {
        return false;
    };
    let Ok(canonical_entry) = project_root.join(path).canonicalize() else {
        return false;
    };

    canonical_entry == canonical_project
        || !canonical_entry.is_dir()
        || canonical_entry.strip_prefix(&canonical_project).is_err()
}

fn output_folder_diagnostic(
    key: &str,
    folder: Option<StringId>,
    reason: InvalidOutputFolderReason,
    location: SourceLocation,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(
        Some(string_table.intern(key)),
        InvalidConfigReason::InvalidOutputFolder { folder, reason },
        location,
    )
}

#[cfg(test)]
#[path = "tests/validation_tests.rs"]
mod tests;
