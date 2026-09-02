//! Recursive config schema validation tests.
//!
//! WHAT: exercises the recursive validator against hand-built schemas and folded values:
//! nested records, required fields, schema defaults, closed string domains, collection
//! handling and the unknown-field policies of record nodes. The grouped `project #= |...|`
//! record path runs through the production entry point against the real builder surface
//! schemas, covering application to `Config`. Unregistered top-level names stay private
//! helpers; a missing grouped `project` or required builder section is rejected. The grouped
//! `html #= |...|` section path covers the same production dispatch against the HTML builder's
//! closed section schema, its typed section storage that the builder's readers consume
//! directly, the grouped output roots that drive directory output resolution, and the
//! inactive-section behaviour when no builder registered the schema.
//! WHY: these invariants have no end-to-end fixture until grouped records and builder
//! sections are validated, so the schema walk and the grouped-record dispatch are pinned here.

use super::{
    ConfigApplyError, ConfigSchemaView, SchemaFieldIndexes, ValidatedConfigValue, ValueCheck,
    validate_and_apply_config_declarations, validate_directory_output_settings, validate_value,
};
use crate::build_system::build::BackendBuilder;
use crate::builder_surface::BuilderSurface;
use crate::builder_surface::config_schema::{
    ConfigFieldShape, ConfigSchema, ConfigSchemaField, UnknownFieldPolicy,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, InvalidConfigReason, InvalidOutputFolderReason,
};
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplateKind,
    PublicConstTemplatePiece, PublicConstTemplateSlot, PublicFoldedField, PublicFoldedValue,
    PublicTemplateSlotKey,
};
use crate::compiler_frontend::single_source_compilation::{
    CompiledConfigSource, ConfigCompilationRequest, FoldedConfigDeclaration, compile_config_source,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::routing::{PageUrlStyle, parse_html_site_config};
use crate::projects::settings::{Config, HtmlSectionConfig};
use std::path::{Path, PathBuf};

// -------------------------
//  Test Support
// -------------------------

fn text(text: &str) -> PublicFoldedValue {
    PublicFoldedValue::String(OwnedFoldedString::Text(text.to_owned()))
}

fn record(fields: Vec<(&str, PublicFoldedValue)>) -> PublicFoldedValue {
    PublicFoldedValue::Record(
        fields
            .into_iter()
            .map(|(name, value)| PublicFoldedField {
                name: name.to_owned(),
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                value,
            })
            .collect(),
    )
}

fn grouped_project() -> (&'static str, PublicFoldedValue) {
    (
        "project",
        record(vec![("name", text("docs")), ("entry_root", text("src"))]),
    )
}

fn with_required_project(
    rest: Vec<(&'static str, PublicFoldedValue)>,
) -> Vec<(&'static str, PublicFoldedValue)> {
    let mut declarations = vec![grouped_project()];
    declarations.extend(rest);
    declarations
}

fn test_location() -> SourceLocation {
    let mut string_table = StringTable::new();
    let scope =
        InternedPath::try_from_filesystem_path(Path::new("project/config.moth"), &mut string_table)
            .expect("the test path is UTF-8");
    SourceLocation::new(scope, Default::default(), Default::default())
}

/// Validate one folded value against one schema field through the production recursion.
fn validate_field(
    schema: &ConfigSchema,
    field: &ConfigSchemaField,
    value: &PublicFoldedValue,
    string_table: &mut StringTable,
) -> (Option<ValidatedConfigValue>, Vec<CompilerDiagnostic>) {
    let field_indexes = SchemaFieldIndexes::build(schema);
    let view = ConfigSchemaView {
        schema,
        field_indexes: &field_indexes,
    };
    let location = test_location();
    let mut errors = Vec::new();
    let mut diagnostics = super::ValueDiagnosticContext {
        location: &location,
        string_table,
        errors: &mut errors,
    };

    let validated = match validate_value(&view, &mut diagnostics, None, &field.shape, field, value)
    {
        ValueCheck::Valid(validated) => Some(validated),

        ValueCheck::Mismatch => {
            super::push_shape_failure(&mut diagnostics, None, &field.shape, field);
            None
        }

        ValueCheck::Reported => None,
    };

    (validated, errors)
}

fn validated_string(validated: &ValidatedConfigValue) -> &str {
    match validated {
        ValidatedConfigValue::String(text) => text,
        other => panic!("expected a validated string value, got {other:?}"),
    }
}

fn validated_record(validated: &ValidatedConfigValue) -> Vec<(&str, &ValidatedConfigValue)> {
    match validated {
        ValidatedConfigValue::Record(fields) => fields
            .iter()
            .map(|field| (field.name.as_str(), &field.value))
            .collect(),
        other => panic!("expected a validated record value, got {other:?}"),
    }
}

fn first_invalid_reason(diagnostics: &[CompilerDiagnostic]) -> &InvalidConfigReason {
    let Some(diagnostic) = diagnostics.first() else {
        panic!("expected at least one diagnostic, got none");
    };

    match &diagnostic.payload {
        DiagnosticPayload::InvalidConfig { reason, .. } => reason,
        payload => panic!("expected an InvalidConfig payload, got {payload:?}"),
    }
}

/// A schema with one record field `site` whose node declares `title` and a closed `channel`.
///
/// Each test rebuilds the schema so the case controls the node's unknown-field policy itself.
fn site_record_schema(unknown_fields: UnknownFieldPolicy) -> ConfigSchema {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let site_node = schema
        .add_node("site record", unknown_fields)
        .expect("schema is unfrozen");
    let root = schema.root();

    schema
        .register_field(root, ConfigSchemaField::record("site", site_node))
        .expect("schema is unfrozen");
    schema
        .register_field(site_node, ConfigSchemaField::string("title"))
        .expect("schema is unfrozen");
    schema
        .register_field(
            site_node,
            ConfigSchemaField::closed_string("channel", &["alpha", "beta"]),
        )
        .expect("schema is unfrozen");

    schema
}

fn site_field(schema: &ConfigSchema) -> &ConfigSchemaField {
    root_field(schema, 0)
}

fn root_field(schema: &ConfigSchema, index: usize) -> &ConfigSchemaField {
    schema.field(schema.node(schema.root()).field_ids()[index])
}

// -------------------------
//  Nested Record Validation
// -------------------------

#[test]
fn validates_nested_record_fields_recursively() {
    let schema = site_record_schema(UnknownFieldPolicy::Reject);
    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        site_field(&schema),
        &record(vec![
            ("title", text("Moth docs")),
            ("channel", text("alpha")),
        ]),
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    let site_fields = validated_record(validated.as_ref().expect("the record should validate"));
    assert_eq!(site_fields[0].0, "title");
    assert_eq!(validated_string(site_fields[0].1), "Moth docs");
    assert_eq!(site_fields[1].0, "channel");
    assert_eq!(validated_string(site_fields[1].1), "alpha");
}

#[test]
fn rejects_wrong_record_shape_on_record_field() {
    let schema = site_record_schema(UnknownFieldPolicy::Reject);
    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        site_field(&schema),
        &PublicFoldedValue::Int(3),
        &mut string_table,
    );

    assert!(
        validated.is_none(),
        "a non-record value must not validate as a record"
    );
    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::InvalidConfigValueShape { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn rejects_shape_failure_inside_nested_record() {
    let schema = site_record_schema(UnknownFieldPolicy::Preserve);
    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        site_field(&schema),
        &record(vec![("title", PublicFoldedValue::Int(7))]),
        &mut string_table,
    );

    assert!(
        validated.is_none(),
        "a nested string field must reject an int"
    );
    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::InvalidConfigValueShape { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
}

// -------------------------
//  Unknown-Field Policy
// -------------------------

#[test]
fn rejects_unknown_field_in_closed_record() {
    let schema = site_record_schema(UnknownFieldPolicy::Reject);
    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        site_field(&schema),
        &record(vec![("title", text("Moth docs")), ("mystery", text("?"))]),
        &mut string_table,
    );

    assert!(
        validated.is_none(),
        "a closed record must reject unknown fields"
    );
    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::UnknownRecordField { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn allows_unknown_field_in_open_record() {
    let schema = site_record_schema(UnknownFieldPolicy::Preserve);
    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        site_field(&schema),
        &record(vec![
            ("title", text("Moth docs")),
            ("mystery", text("kept")),
        ]),
        &mut string_table,
    );

    assert!(
        errors.is_empty(),
        "unknown fields must be preserved: {errors:?}"
    );
    let site_fields =
        validated_record(validated.as_ref().expect("the open record should validate"));
    assert_eq!(
        site_fields.len(),
        2,
        "unknown fields must stay in the record"
    );
    assert_eq!(site_fields[0].0, "title");
    assert_eq!(site_fields[1].0, "mystery");
}

// -------------------------
//  Required Fields and Defaults
// -------------------------

#[test]
fn rejects_missing_required_record_field() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let project_node = schema
        .add_node("project record", UnknownFieldPolicy::Preserve)
        .expect("schema is unfrozen");

    schema
        .register_field(
            schema.root(),
            ConfigSchemaField::record("project", project_node),
        )
        .expect("schema is unfrozen");
    schema
        .register_field(project_node, ConfigSchemaField::string("name").required())
        .expect("schema is unfrozen");

    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        root_field(&schema, 0),
        &record(vec![("version", text("1.0"))]),
        &mut string_table,
    );

    assert!(
        validated.is_none(),
        "a record without its required field must fail"
    );
    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::MissingRequiredRecordField { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn contributes_schema_default_for_omitted_optional_field() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let site_node = schema
        .add_node("site record", UnknownFieldPolicy::Reject)
        .expect("schema is unfrozen");

    schema
        .register_field(schema.root(), ConfigSchemaField::record("site", site_node))
        .expect("schema is unfrozen");
    schema
        .register_field(site_node, ConfigSchemaField::string("title"))
        .expect("schema is unfrozen");
    schema
        .register_field(
            site_node,
            ConfigSchemaField::string("channel").default(text("alpha")),
        )
        .expect("schema is unfrozen");

    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        root_field(&schema, 0),
        &record(vec![("title", text("Moth docs"))]),
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    let site_fields = validated_record(validated.as_ref().expect("the record should validate"));
    assert_eq!(
        site_fields.len(),
        2,
        "the schema default must join the record"
    );
    assert_eq!(site_fields[1].0, "channel");
    assert_eq!(validated_string(site_fields[1].1), "alpha");
}

// -------------------------
//  Closed Domains and Optionals
// -------------------------

#[test]
fn enforces_closed_string_domain_inside_record() {
    let schema = site_record_schema(UnknownFieldPolicy::Preserve);
    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        site_field(&schema),
        &record(vec![("channel", text("gamma"))]),
        &mut string_table,
    );

    assert!(
        validated.is_none(),
        "a value outside the closed domain must fail"
    );
    let InvalidConfigReason::InvalidConfigValueShape { expected } = first_invalid_reason(&errors)
    else {
        panic!("expected a value-shape diagnostic, got {errors:?}");
    };
    assert_eq!(
        string_table.resolve(*expected),
        "one of: \"alpha\", \"beta\"",
        "closed-domain failures must report the allowed set"
    );
}

#[test]
fn accepts_optional_none_and_bare_values() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();

    schema
        .register_field(
            root,
            ConfigSchemaField::optional("origin", ConfigFieldShape::String),
        )
        .expect("schema is unfrozen");

    let mut string_table = StringTable::new();

    // An authored `none` stays absent.
    let (validated, errors) = validate_field(
        &schema,
        root_field(&schema, 0),
        &PublicFoldedValue::OptionNone,
        &mut string_table,
    );
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert!(
        matches!(validated, Some(ValidatedConfigValue::OptionNone)),
        "an authored none must validate as absent, got {validated:?}"
    );

    // A bare string is accepted as a present optional.
    let (validated, errors) = validate_field(
        &schema,
        root_field(&schema, 0),
        &text("https://moth.dev"),
        &mut string_table,
    );
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(
        validated_string(validated.as_ref().expect("the bare value should validate")),
        "https://moth.dev"
    );
}

#[test]
fn rejects_a_scalar_for_a_string_collection() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();

    schema
        .register_field(root, ConfigSchemaField::string_collection("folders"))
        .expect("schema is unfrozen");

    let mut string_table = StringTable::new();
    let folders_field = root_field(&schema, 0);

    let (validated, errors) =
        validate_field(&schema, folders_field, &text("lib"), &mut string_table);
    assert!(validated.is_none());
    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::InvalidConfigValueShape { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );

    let (validated, errors) = validate_field(
        &schema,
        folders_field,
        &PublicFoldedValue::Collection(vec![text("lib"), text("packages")]),
        &mut string_table,
    );
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    match validated {
        Some(ValidatedConfigValue::Collection(values)) => {
            assert_eq!(
                values.iter().map(validated_string).collect::<Vec<_>>(),
                vec!["lib", "packages"]
            )
        }
        other => panic!("expected a validated string collection, got {other:?}"),
    }
}

#[test]
fn rejects_int_element_in_string_collection_with_shape_reason() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();

    schema
        .register_field(root, ConfigSchemaField::string_collection("folders"))
        .expect("schema is unfrozen");

    let mut string_table = StringTable::new();

    let (validated, errors) = validate_field(
        &schema,
        root_field(&schema, 0),
        &PublicFoldedValue::Collection(vec![text("lib"), PublicFoldedValue::Int(3)]),
        &mut string_table,
    );

    assert!(
        validated.is_none(),
        "an int element must fail the collection"
    );
    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::InvalidConfigValueShape { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
    assert_eq!(
        errors.len(),
        1,
        "generic collection failures report one shape diagnostic"
    );
}

#[test]
fn accepts_int_collection_and_nested_optional_elements() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();

    schema
        .register_field(
            root,
            ConfigSchemaField::collection("counts", ConfigFieldShape::Int),
        )
        .expect("schema is unfrozen");
    schema
        .register_field(
            root,
            ConfigSchemaField::collection(
                "maybe_titles",
                ConfigFieldShape::Optional(Box::new(ConfigFieldShape::String)),
            ),
        )
        .expect("schema is unfrozen");

    let mut string_table = StringTable::new();
    let counts_field = root_field(&schema, 0);
    let titles_field = root_field(&schema, 1);

    let (validated, errors) = validate_field(
        &schema,
        counts_field,
        &PublicFoldedValue::Collection(vec![PublicFoldedValue::Int(1), PublicFoldedValue::Int(2)]),
        &mut string_table,
    );
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    match validated {
        Some(ValidatedConfigValue::Collection(values)) => {
            assert_eq!(
                values,
                vec![ValidatedConfigValue::Int(1), ValidatedConfigValue::Int(2)]
            )
        }
        other => panic!("expected a validated int collection, got {other:?}"),
    }

    let (validated, errors) = validate_field(
        &schema,
        titles_field,
        &PublicFoldedValue::Collection(vec![text("home"), PublicFoldedValue::OptionNone]),
        &mut string_table,
    );
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    match validated {
        Some(ValidatedConfigValue::Collection(values)) => {
            assert_eq!(validated_string(&values[0]), "home");
            assert!(matches!(values[1], ValidatedConfigValue::OptionNone));
        }
        other => panic!("expected optional string collection elements, got {other:?}"),
    }
}

// -------------------------
//  Grouped Project Record Application
// -------------------------
fn apply_result(result: Result<(), ConfigApplyError>) -> Result<(), Vec<CompilerDiagnostic>> {
    match result {
        Ok(()) => Ok(()),
        Err(ConfigApplyError::Diagnostics(errors)) => Err(errors),
        Err(ConfigApplyError::Compiler(error)) => panic!("{}", error.msg),
    }
}

/// Run the production entry point over hand-built folded declarations against the real
/// mandatory-core schema surface.
///
/// Returns the applied config and the collected diagnostics so each case asserts both sides
/// of the grouped-record dispatch: what validates and what reaches `Config`.
fn validate_and_apply(
    declarations: Vec<(&str, PublicFoldedValue)>,
    string_table: &mut StringTable,
) -> (Config, Vec<CompilerDiagnostic>) {
    let surface = BuilderSurface::with_mandatory_core();

    validate_and_apply_with_surface(declarations, &surface, string_table)
}

/// Run the production entry point over hand-built folded declarations against one builder
/// surface, so html-section cases can use the HTML builder's registered schemas.
fn validate_and_apply_with_surface(
    declarations: Vec<(&str, PublicFoldedValue)>,
    surface: &BuilderSurface,
    string_table: &mut StringTable,
) -> (Config, Vec<CompilerDiagnostic>) {
    let source = CompiledConfigSource {
        declarations: declarations
            .into_iter()
            .map(|(name, value)| FoldedConfigDeclaration {
                name: string_table.intern(name),
                value,
                location: test_location(),
                name_location: test_location(),
                direct_field_locations: Vec::new(),
            })
            .collect(),
    };
    let mut config = Config::new(PathBuf::from("project"));

    match apply_result(validate_and_apply_config_declarations(
        &mut config,
        &source,
        &surface.config_schemas,
        string_table,
    )) {
        Ok(()) => (config, Vec::new()),

        Err(errors) => (config, errors),
    }
}

#[test]
fn applies_grouped_project_record_to_config_fields() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![(
            "project",
            record(vec![
                ("entry_root", text("src")),
                ("name", text("docs")),
                ("version", text("1.2.3")),
                ("author", text("Ada")),
                ("license", text("MIT")),
                (
                    "template_const_loop_iteration_limit",
                    PublicFoldedValue::Int(500),
                ),
            ]),
        )],
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.project_name, "docs");
    assert_eq!(config.entry_root, PathBuf::from("src"));
    assert_eq!(config.version, "1.2.3");
    assert_eq!(config.author, "Ada");
    assert_eq!(config.license, "MIT");
    assert_eq!(config.template_const_loop_iteration_limit, 500);

    // Every compiler-owned field records its setting location.
    for field_name in [
        "name",
        "entry_root",
        "version",
        "author",
        "license",
        "template_const_loop_iteration_limit",
    ] {
        assert!(
            config.setting_locations.contains_key(field_name),
            "field '{field_name}' should record its setting location"
        );
    }
}

#[test]
fn rejects_grouped_project_record_missing_required_name() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![("project", record(vec![("entry_root", text("src"))]))],
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::MissingRequiredRecordField { record, field } = reason else {
        panic!("expected a missing-required-field diagnostic, got {reason:?}");
    };
    assert_eq!(string_table.resolve(*record), "project record");
    assert_eq!(string_table.resolve(*field), "name");

    // The rejected record applies none of its fields.
    assert_eq!(config.project_name, "");
    assert_eq!(config.entry_root, PathBuf::from(""));
}

#[test]
fn allows_extra_metadata_fields_on_grouped_project_record() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![(
            "project",
            record(vec![
                ("name", text("docs")),
                ("metadata", record(vec![("channel", text("alpha"))])),
                ("custom_note", text("open project metadata")),
            ]),
        )],
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.project_name, "docs");
    assert_eq!(config.extra_project_fields.len(), 2);
    assert_eq!(config.extra_project_fields[0].name, "metadata");
    assert_eq!(config.extra_project_fields[1].name, "custom_note");
    assert_eq!(
        config.extra_project_fields[1].type_identity,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)
    );
    assert_eq!(
        config.extra_project_fields[1].value,
        text("open project metadata")
    );
}

#[test]
fn rejects_wrong_shape_inside_grouped_project_record() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![("project", record(vec![("name", PublicFoldedValue::Int(5))]))],
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::InvalidConfigValueShape { expected } = reason else {
        panic!("expected a value-shape diagnostic, got {reason:?}");
    };
    assert_eq!(string_table.resolve(*expected), "a string value");
    assert_eq!(
        config.project_name, "",
        "a rejected record field must not apply to the config"
    );
}

#[test]
fn rejects_legacy_flat_project_selector_on_file_root() {
    let mut string_table = StringTable::new();

    let (_, errors) = validate_and_apply(vec![("project", text("html"))], &mut string_table);
    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::InvalidConfigValueShape { expected } = reason else {
        panic!("expected a record-shape diagnostic for the legacy selector, got {reason:?}");
    };
    assert_eq!(string_table.resolve(*expected), "a record value");
}

#[test]
fn skips_declare_first_helper_records_instead_of_unknown_key() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![
            ("project_metadata", record(vec![("channel", text("alpha"))])),
            (
                "project",
                record(vec![
                    ("name", text("docs")),
                    ("metadata", record(vec![("channel", text("alpha"))])),
                ]),
            ),
        ],
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.project_name, "docs");
}

#[test]
fn rejects_retired_flat_config_keys_instead_of_treating_them_as_helpers() {
    let mut string_table = StringTable::new();

    for key in [
        "dev_folder",
        "output_folder",
        "package_folders",
        "project_name",
    ] {
        let (_, errors) = validate_and_apply(
            vec![
                (
                    "project",
                    record(vec![("name", text("docs")), ("entry_root", text("src"))]),
                ),
                (key, text("preview")),
            ],
            &mut string_table,
        );
        assert!(
            matches!(
                first_invalid_reason(&errors),
                InvalidConfigReason::UnknownKey { .. }
            ),
            "retired key {key} must be rejected, got {errors:?}"
        );
    }
}

#[test]
fn rejects_grouped_project_entry_root_that_escapes_the_project() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![(
            "project",
            record(vec![
                ("name", text("docs")),
                ("entry_root", text("../sibling")),
            ]),
        )],
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::InvalidProjectSettingValue { expected, .. } = reason else {
        panic!("expected an entry-root containment diagnostic, got {reason:?}");
    };
    assert!(
        string_table
            .resolve(*expected)
            .contains("strictly below the project root"),
        "unexpected expected-text: {}",
        string_table.resolve(*expected)
    );
    assert_eq!(
        config.entry_root,
        PathBuf::from(""),
        "a rejected entry_root must not apply"
    );
}

#[cfg(unix)]
#[test]
fn rejects_grouped_project_entry_root_symlink_outside_the_project() {
    use std::os::unix::fs::symlink;
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().join("project");
    let outside = _temp.path().join("outside");
    std::fs::create_dir_all(&root).expect("should create project dir");
    std::fs::create_dir_all(&outside).expect("should create outside dir");
    symlink(&outside, root.join("sources")).expect("should create entry_root symlink");

    let mut string_table = StringTable::new();
    let source = CompiledConfigSource {
        declarations: vec![FoldedConfigDeclaration {
            name: string_table.intern("project"),
            value: record(vec![
                ("name", text("docs")),
                ("entry_root", text("sources")),
            ]),
            location: test_location(),
            name_location: test_location(),
            direct_field_locations: Vec::new(),
        }],
    };
    let mut config = Config::new(root.clone());
    let surface = BuilderSurface::with_mandatory_core();
    let errors = apply_result(validate_and_apply_config_declarations(
        &mut config,
        &source,
        &surface.config_schemas,
        &mut string_table,
    ))
    .expect_err("a symlink out of the project must fail");

    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::InvalidProjectSettingValue { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn rejects_grouped_project_entry_root_that_is_a_regular_file() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().join("project");
    std::fs::create_dir_all(&root).expect("should create project dir");
    std::fs::write(root.join("README.md"), b"not a directory").expect("should write file");

    let mut string_table = StringTable::new();
    let source = CompiledConfigSource {
        declarations: vec![FoldedConfigDeclaration {
            name: string_table.intern("project"),
            value: record(vec![
                ("name", text("docs")),
                ("entry_root", text("README.md")),
            ]),
            location: test_location(),
            name_location: test_location(),
            direct_field_locations: Vec::new(),
        }],
    };
    let mut config = Config::new(root);
    let surface = BuilderSurface::with_mandatory_core();
    let errors = apply_result(validate_and_apply_config_declarations(
        &mut config,
        &source,
        &surface.config_schemas,
        &mut string_table,
    ))
    .expect_err("a file-valued entry_root must fail");

    assert!(
        matches!(
            first_invalid_reason(&errors),
            InvalidConfigReason::InvalidProjectSettingValue { .. }
        ),
        "unexpected diagnostics: {errors:?}"
    );
    assert_eq!(config.entry_root, PathBuf::from(""));
}

// -------------------------
//  Grouped HTML Section Application
// -------------------------

/// The HTML builder surface whose closed section schema owns the grouped `html` record.
fn html_builder_surface() -> BuilderSurface {
    HtmlProjectBuilder::new().frontend_surface()
}

#[test]
fn applies_grouped_html_section_to_typed_storage() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![(
            "html",
            record(vec![
                ("origin", text("/docs")),
                ("page_url_style", text("no_trailing_slash")),
                ("redirect_index_html", PublicFoldedValue::Bool(false)),
                ("html_lang", text("en-GB")),
                ("html_inject_core_css", PublicFoldedValue::Bool(false)),
            ]),
        )]),
        &surface,
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");

    // The typed section result keeps every validated value; omitted fields stay `None`.
    assert_eq!(config.html_section.origin.as_deref(), Some("/docs"));
    assert_eq!(
        config.html_section.page_url_style.as_deref(),
        Some("no_trailing_slash")
    );
    assert_eq!(config.html_section.redirect_index_html, Some(false));
    assert_eq!(config.html_section.html_lang.as_deref(), Some("en-GB"));
    assert_eq!(config.html_section.html_inject_core_css, Some(false));
    assert_eq!(config.html_section.html_title_prefix, None);

    // The site-config reader consumes the typed section directly.
    let site_config = parse_html_site_config(&config, &mut string_table)
        .expect("section should drive site config");
    assert_eq!(site_config.origin, "/docs");
    assert_eq!(site_config.page_url_style, PageUrlStyle::NoTrailingSlash);
    assert!(!site_config.redirect_index_html);

    // Every authored field records the section's location for downstream value diagnostics.
    for field_name in [
        "origin",
        "page_url_style",
        "redirect_index_html",
        "html_lang",
        "html_inject_core_css",
    ] {
        assert!(
            config.setting_locations.contains_key(field_name),
            "field '{field_name}' should record its setting location"
        );
    }
}

#[test]
fn rejects_unknown_field_in_closed_grouped_html_section() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![(
            "html",
            record(vec![("origin", text("/docs")), ("custom_key", text("no"))]),
        )]),
        &surface,
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::UnknownRecordField { record, field } = reason else {
        panic!("expected an unknown-record-field diagnostic, got {reason:?}");
    };
    assert_eq!(string_table.resolve(*record), "html section");
    assert_eq!(string_table.resolve(*field), "custom_key");

    // The rejected section applies nothing.
    assert_eq!(config.html_section.origin, None);
}

#[test]
fn private_helpers_do_not_satisfy_required_html_section() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();
    let (config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![
            ("origin", text("/moth")),
            ("redirect_index_html", PublicFoldedValue::Bool(false)),
        ]),
        &surface,
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    assert!(
        matches!(
            reason,
            InvalidConfigReason::MissingActiveBuilderSection { .. }
        ),
        "former top-level builder names are private helpers, got {reason:?}"
    );
    assert_eq!(config.html_section.origin, None);
}

#[test]
fn requires_the_active_html_builder_section() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (_, errors) =
        validate_and_apply_with_surface(vec![grouped_project()], &surface, &mut string_table);

    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::MissingActiveBuilderSection { section } = reason else {
        panic!("expected a missing-builder-section diagnostic, got {reason:?}");
    };
    assert_eq!(string_table.resolve(*section), "html");
}

#[test]
fn skips_grouped_html_section_when_no_builder_registered_it() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        with_required_project(vec![("html", record(vec![("origin", text("/docs"))]))]),
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.html_section, HtmlSectionConfig::default());
}

#[test]
fn applies_authored_grouped_html_section_from_compiled_config_source() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            source_code: "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\n\nhtml #= |\n    origin = \"/docs\",\n    html_lang = \"en-GB\",\n    dev_output = \"site/dev\",\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
        },
        &mut string_table,
    )
    .expect("an authored html section should compile to folded declarations");

    let mut config = Config::new(PathBuf::from("project"));
    apply_result(validate_and_apply_config_declarations(
        &mut config,
        &compiled,
        &surface.config_schemas,
        &mut string_table,
    ))
    .expect("a grouped html section should validate and apply");

    assert_eq!(config.html_section.origin.as_deref(), Some("/docs"));
    assert_eq!(config.html_section.html_lang.as_deref(), Some("en-GB"));
    assert_eq!(config.html_section.dev_output.as_deref(), Some("site/dev"));
}

// -------------------------
//  Grouped HTML Output Roots
// -------------------------

#[test]
fn grouped_html_section_omitted_output_fields_take_schema_defaults() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![("html", record(vec![("origin", text("/docs"))]))]),
        &surface,
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.html_section.dev_output.as_deref(), Some("dev"));
    assert_eq!(
        config.html_section.release_output.as_deref(),
        Some("release")
    );
}

#[test]
fn applies_custom_grouped_html_output_roots_to_typed_storage() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![(
            "html",
            record(vec![
                ("dev_output", text("site/dev")),
                ("release_output", text("site/public")),
            ]),
        )]),
        &surface,
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.html_section.dev_output.as_deref(), Some("site/dev"));
    assert_eq!(
        config.html_section.release_output.as_deref(),
        Some("site/public")
    );
}

#[test]
fn grouped_html_output_roots_drive_directory_output_settings() {
    let _project_root = tempfile::tempdir().expect("should create project root");
    let project_root = _project_root.path().to_path_buf();
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (mut config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![(
            "html",
            record(vec![
                ("dev_output", text("site/dev")),
                ("release_output", text("site/public")),
            ]),
        )]),
        &surface,
        &mut string_table,
    );
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");

    config.entry_dir = project_root.clone();

    let settings = validate_directory_output_settings(&config, &mut string_table)
        .expect("custom grouped output roots should validate");

    assert_eq!(settings.dev.relative_path, PathBuf::from("site/dev"));
    assert_eq!(
        settings.release.resolved_path,
        project_root.join("site/public")
    );
}

#[test]
fn rejects_parent_traversal_in_grouped_html_output_root() {
    let mut string_table = StringTable::new();
    let surface = html_builder_surface();

    let (config, errors) = validate_and_apply_with_surface(
        with_required_project(vec![(
            "html",
            record(vec![("dev_output", text("../outside"))]),
        )]),
        &surface,
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(
        config.html_section.dev_output.as_deref(),
        Some("../outside")
    );

    let output_errors = validate_directory_output_settings(&config, &mut string_table)
        .expect_err("a parent-traversing output root must be rejected");

    let reason = first_invalid_reason(&output_errors);
    let InvalidConfigReason::InvalidOutputFolder { folder, reason } = reason else {
        panic!("expected an invalid-output-folder diagnostic, got {reason:?}");
    };

    assert_eq!(*reason, InvalidOutputFolderReason::ParentDirectorySegment);
    assert_eq!(
        string_table.resolve(folder.expect("the rejected folder is interned")),
        "../outside"
    );
}

#[test]
fn accepts_scalar_helper_constants() {
    let mut string_table = StringTable::new();

    let (config, errors) = validate_and_apply(
        vec![("default_channel", text("alpha")), grouped_project()],
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.project_name, "docs");
}

#[test]
fn rejects_an_invalid_moth_project_identifier() {
    let mut string_table = StringTable::new();

    let (_, errors) = validate_and_apply(
        vec![("project", record(vec![("name", text("not a package"))]))],
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    let InvalidConfigReason::InvalidProjectSettingValue { expected, .. } = reason else {
        panic!("expected a project-identifier diagnostic, got {reason:?}");
    };
    assert_eq!(
        string_table.resolve(*expected),
        "a valid Moth project identifier"
    );
}

#[test]
fn converts_concrete_project_template_metadata_to_string() {
    let mut string_table = StringTable::new();
    let template = PublicFoldedValue::ConstTemplate(PublicConstTemplate {
        kind: PublicConstTemplateKind::Wrapper,
        pieces: vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Text(
            "hello".to_owned(),
        ))],
        conditional_child_wrappers: Vec::new(),
    });

    let (config, errors) = validate_and_apply(
        vec![(
            "project",
            record(vec![("name", text("docs")), ("greeting", template)]),
        )],
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.extra_project_fields.len(), 1);
    assert_eq!(config.extra_project_fields[0].name, "greeting");
    assert_eq!(config.extra_project_fields[0].value, text("hello"));
}

#[test]
fn converts_text_only_string_pieces_in_project_template_metadata() {
    let mut string_table = StringTable::new();
    let template = PublicFoldedValue::ConstTemplate(PublicConstTemplate {
        kind: PublicConstTemplateKind::Wrapper,
        pieces: vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Pieces(
            vec![
                OwnedFoldedStringPiece::Text("open".to_owned()),
                OwnedFoldedStringPiece::Text(" note".to_owned()),
            ],
        ))],
        conditional_child_wrappers: Vec::new(),
    });

    let (config, errors) = validate_and_apply(
        vec![(
            "project",
            record(vec![("name", text("docs")), ("note", template)]),
        )],
        &mut string_table,
    );

    assert!(errors.is_empty(), "unexpected diagnostics: {errors:?}");
    assert_eq!(config.extra_project_fields.len(), 1);
    assert_eq!(config.extra_project_fields[0].value, text("open note"));
}

#[test]
fn rejects_unresolved_project_template_metadata() {
    let mut string_table = StringTable::new();
    let template = PublicFoldedValue::ConstTemplate(PublicConstTemplate {
        kind: PublicConstTemplateKind::Wrapper,
        pieces: vec![PublicConstTemplatePiece::Slot(PublicConstTemplateSlot {
            key: PublicTemplateSlotKey::Named("title".to_owned()),
            applied_child_wrappers: Vec::new(),
            child_wrappers: Vec::new(),
            skip_parent_child_wrappers: false,
        })],
        conditional_child_wrappers: Vec::new(),
    });

    let (_, errors) = validate_and_apply(
        vec![(
            "project",
            record(vec![("name", text("docs")), ("card", template)]),
        )],
        &mut string_table,
    );

    let reason = first_invalid_reason(&errors);
    assert!(
        matches!(reason, InvalidConfigReason::InvalidConfigValueShape { .. }),
        "unresolved templates must not become project metadata, got {reason:?}"
    );
}

#[test]
fn retains_compiled_project_metadata_type_and_field_location() {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            source_code: "project #= |\n    name = \"docs\",\n    custom_note = \"open\",\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
        },
        &mut string_table,
    )
    .expect("grouped project metadata should compile");

    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project record should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("project value must be a record");
    };
    assert_eq!(
        fields.len(),
        project.direct_field_locations.len(),
        "direct field locations must align with folded record fields"
    );
    let note_index = fields
        .iter()
        .position(|field| field.name == "custom_note")
        .expect("folded fields must retain extra project fields");
    assert_eq!(
        fields[note_index].type_identity,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)
    );
    assert_ne!(
        project.direct_field_locations[note_index], project.location,
        "field initializer location must not collapse to the project record"
    );

    let mut config = Config::new(PathBuf::from("project"));
    apply_result(validate_and_apply_config_declarations(
        &mut config,
        &compiled,
        &surface.config_schemas,
        &mut string_table,
    ))
    .expect("open project metadata should apply");

    assert_eq!(config.extra_project_fields.len(), 1);
    assert_eq!(config.extra_project_fields[0].name, "custom_note");
    assert_eq!(
        config.extra_project_fields[0].type_identity,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)
    );
    assert_eq!(
        config.extra_project_fields[0].location,
        project.direct_field_locations[note_index]
    );
    assert_eq!(config.extra_project_fields[0].value, text("open"));
}
