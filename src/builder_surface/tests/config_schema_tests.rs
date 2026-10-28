//! Config schema construction tests.
//!
//! WHAT: covers the schema engine's data invariants: dense node and field identifiers, root
//! identity, node field registration, unknown-field policy storage and shape descriptions.
//! WHY: the validator relies on these facts being total and deterministic; the construction
//! surface is the one place that could break them.

use super::super::BuilderSurface;
use super::super::config_schema::{
    ConfigFieldShape, ConfigSchema, ConfigSchemaField, ConfigSchemas, NamedConfigSectionSchema,
    ProjectFieldConfigPolicy, UnknownFieldPolicy,
};
use crate::build_system::build::BackendBuilder;
use crate::builder_surface::definition::BuilderSurfaceValidationError;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

#[test]
fn registers_root_and_fields_with_dense_ids() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();

    assert_eq!(root.0, 0, "the root must be the first registered node");

    let nested = schema
        .add_node("nested record", UnknownFieldPolicy::Preserve)
        .expect("schema is unfrozen");
    assert_eq!(nested.0, 1, "node identifiers must be dense");

    let first = schema
        .register_field(root, ConfigSchemaField::string("entry_root"))
        .expect("schema is unfrozen");
    let second = schema
        .register_field(nested, ConfigSchemaField::string("lang"))
        .expect("schema is unfrozen");

    assert_eq!(first.0, 0, "field identifiers must be dense across nodes");
    assert_eq!(second.0, 1, "field identifiers must be dense across nodes");

    assert_eq!(schema.node(root).field_ids(), &[first]);
    assert_eq!(schema.node(nested).field_ids(), &[second]);
    assert_eq!(schema.field(first).name, "entry_root");
    assert_eq!(schema.field(second).name, "lang");
}

#[test]
fn registers_runtime_section_and_field_names() {
    let runtime_section_name = format!("builder_{}", 17);
    let runtime_field_name = format!("setting_{}", 23);

    let mut section_schema =
        ConfigSchema::new("runtime section record", UnknownFieldPolicy::Reject);
    section_schema
        .register_field(
            section_schema.root(),
            ConfigSchemaField::string(runtime_field_name.clone()),
        )
        .expect("section schema is unfrozen");
    section_schema
        .validate_and_freeze()
        .expect("section schema is valid");

    let mut project_schema = ConfigSchema::new("project record", UnknownFieldPolicy::Preserve);
    project_schema
        .validate_and_freeze()
        .expect("project schema is valid");

    let mut schemas = ConfigSchemas::new(project_schema);
    schemas.register_project_section(NamedConfigSectionSchema {
        name: runtime_section_name.clone().into(),
        schema: section_schema,
        required: false,
    });

    let registered_section = schemas
        .project_section(&runtime_section_name)
        .expect("runtime section name is registered");
    assert!(
        registered_section
            .schema
            .node(registered_section.schema.root())
            .field_ids()
            .iter()
            .any(|field_id| {
                registered_section.schema.field(*field_id).name == runtime_field_name
            }),
        "schema must find a field by its runtime name"
    );
}

#[test]
fn keeps_unknown_field_policy_on_nodes() {
    let mut schema = ConfigSchema::new("closed root", UnknownFieldPolicy::Reject);
    let open = schema
        .add_node("open node", UnknownFieldPolicy::Preserve)
        .expect("schema is unfrozen");

    assert_eq!(
        schema.node(schema.root()).unknown_fields,
        UnknownFieldPolicy::Reject
    );
    assert_eq!(
        schema.node(open).unknown_fields,
        UnknownFieldPolicy::Preserve
    );
}

#[test]
fn project_policy_snapshot_preserves_known_policies_and_unknown_fallback() {
    let policies = BuilderSurface::with_mandatory_core()
        .config_schemas
        .project()
        .project_field_config_policies();

    assert_eq!(
        policies.policy_for("name"),
        ProjectFieldConfigPolicy::FixedOnly
    );
    assert_eq!(
        policies.policy_for("entry_root"),
        ProjectFieldConfigPolicy::FixedOnly
    );
    assert_eq!(
        policies.policy_for("version"),
        ProjectFieldConfigPolicy::Configurable
    );
    assert_eq!(
        policies.policy_for("template_const_loop_iteration_limit"),
        ProjectFieldConfigPolicy::FixedOnly
    );
    assert_eq!(
        policies.policy_for("unregistered_metadata"),
        ProjectFieldConfigPolicy::Configurable
    );
}

#[test]
fn records_field_facts_on_registration() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();

    schema
        .register_field(
            root,
            ConfigSchemaField::closed_string("channel", &["alpha", "beta"]).required(),
        )
        .expect("schema is unfrozen");
    schema
        .register_field(
            root,
            ConfigSchemaField::string_with_default("title", "home"),
        )
        .expect("schema is unfrozen");

    let channel = schema.field(schema.node(root).field_ids()[0]);
    assert_eq!(channel.name, "channel");
    assert!(channel.required);
    assert_eq!(
        channel.allowed_strings,
        Some(&["alpha", "beta"] as &[&str]),
        "closed-domain facts must stay on the field record"
    );

    let title = schema.field(schema.node(root).field_ids()[1]);
    assert!(!title.required);
    assert!(
        title.default.is_some(),
        "defaults must stay on the field record"
    );
}

#[test]
fn frozen_schema_rejects_further_registration() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    schema
        .validate_and_freeze()
        .expect("an empty schema is valid");

    let error = schema
        .add_node("nested", UnknownFieldPolicy::Reject)
        .expect_err("frozen schemas must not grow");
    assert!(
        error.message.contains("frozen config schema"),
        "mutation after freeze is a compiler error, got {}",
        error.message
    );
    assert!(schema.is_frozen());
}

#[test]
fn describes_shape_vocabulary_for_diagnostics() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let nested = schema
        .add_node("nested", UnknownFieldPolicy::Reject)
        .expect("schema is unfrozen");

    assert_eq!(ConfigFieldShape::String.describe(), "a string value");
    assert_eq!(
        ConfigFieldShape::FixedScalar(FixedScalar::I32).describe(),
        "a value of type I32"
    );
    assert_eq!(ConfigFieldShape::Bool.describe(), "a boolean value");
    assert_eq!(
        ConfigFieldShape::Record(nested).describe(),
        "a record value"
    );
    assert_eq!(
        ConfigSchemaField::string_collection("folders")
            .shape
            .describe(),
        "a collection of strings"
    );
    assert_eq!(
        ConfigSchemaField::optional("origin", ConfigFieldShape::String)
            .shape
            .describe(),
        "a string value or none"
    );
}

#[test]
fn validate_and_freeze_rejects_incompatible_default() {
    use crate::compiler_frontend::folded_value::PublicFoldedValue;
    use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    schema
        .register_field(
            root,
            ConfigSchemaField::string("title").default(PublicFoldedValue::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 1).expect("a test value fits I32"),
            )),
        )
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("an exact integer default cannot freeze on a String field");
    assert!(
        error.message.contains("default does not match"),
        "got {}",
        error.message
    );
    assert!(!schema.is_frozen());
}

#[test]
fn validate_and_freeze_rejects_duplicate_field_names() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    schema
        .register_field(root, ConfigSchemaField::string("title"))
        .expect("schema is unfrozen");
    schema
        .register_field(root, ConfigSchemaField::string("title"))
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("duplicate field names must not freeze");
    assert!(
        error.message.contains("duplicate field name 'title'"),
        "got {}",
        error.message
    );
}

#[test]
fn validate_and_freeze_rejects_required_field_with_default() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    schema
        .register_field(
            root,
            ConfigSchemaField::string_with_default("title", "home").required(),
        )
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("required plus default is contradictory");
    assert!(
        error.message.contains("both required and have a default"),
        "got {}",
        error.message
    );
}

#[test]
fn validate_and_freeze_rejects_closed_domain_on_int_field() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    let mut field = ConfigSchemaField::fixed_scalar("count", FixedScalar::I32);
    field.allowed_strings = Some(&["one"]);
    schema
        .register_field(root, field)
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("closed strings belong on String fields");
    assert!(
        error
            .message
            .contains("closed string domain on a non-string shape"),
        "got {}",
        error.message
    );
}

#[test]
fn validate_and_freeze_rejects_unknown_record_node() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    schema
        .register_field(
            root,
            ConfigSchemaField::record(
                "site",
                crate::builder_surface::config_schema::ConfigSchemaNodeId(9),
            ),
        )
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("unknown record nodes must not freeze");
    assert!(
        error.message.contains("unknown record node"),
        "got {}",
        error.message
    );
}

#[test]
fn config_schemas_reject_duplicate_project_section_names() {
    let mut first = ConfigSchema::new("html section", UnknownFieldPolicy::Reject);
    first
        .validate_and_freeze()
        .expect("section schema is valid");
    let mut second = ConfigSchema::new("html section", UnknownFieldPolicy::Reject);
    second
        .validate_and_freeze()
        .expect("section schema is valid");

    let mut schemas = ConfigSchemas::new({
        let mut project = ConfigSchema::new("project record", UnknownFieldPolicy::Preserve);
        project
            .validate_and_freeze()
            .expect("project schema is valid");
        project
    });
    schemas.register_project_section(NamedConfigSectionSchema {
        name: "html".into(),
        schema: first,
        required: true,
    });
    schemas.register_project_section(NamedConfigSectionSchema {
        name: "html".into(),
        schema: second,
        required: false,
    });

    let error = schemas
        .validate()
        .expect_err("duplicate project section names are compiler errors");
    assert!(
        error
            .message
            .contains("project config section name 'html' is registered more than once"),
        "got {}",
        error.message
    );
}

#[test]
fn config_schemas_allow_the_same_name_in_project_and_entry_namespaces() {
    let mut project_html = ConfigSchema::new("html section", UnknownFieldPolicy::Reject);
    project_html
        .validate_and_freeze()
        .expect("section schema is valid");
    let mut entry_html = ConfigSchema::new("html entry section", UnknownFieldPolicy::Reject);
    entry_html
        .validate_and_freeze()
        .expect("section schema is valid");

    let mut schemas = ConfigSchemas::new({
        let mut project = ConfigSchema::new("project record", UnknownFieldPolicy::Preserve);
        project
            .validate_and_freeze()
            .expect("project schema is valid");
        project
    });
    schemas.register_project_section(NamedConfigSectionSchema {
        name: "html".into(),
        schema: project_html,
        required: true,
    });
    schemas.register_entry_section(NamedConfigSectionSchema {
        name: "html".into(),
        schema: entry_html,
        required: false,
    });

    schemas
        .validate()
        .expect("project and entry html sections occupy separate namespaces");
}

#[test]
fn config_schemas_reject_reserved_project_section_name() {
    let mut section = ConfigSchema::new("reserved", UnknownFieldPolicy::Reject);
    section
        .validate_and_freeze()
        .expect("section schema is valid");

    let mut schemas = ConfigSchemas::new({
        let mut project = ConfigSchema::new("project record", UnknownFieldPolicy::Preserve);
        project
            .validate_and_freeze()
            .expect("project schema is valid");
        project
    });
    schemas.register_entry_section(NamedConfigSectionSchema {
        name: "project".into(),
        schema: section,
        required: false,
    });

    let error = schemas
        .validate()
        .expect_err("project is reserved in both registries");
    assert!(
        error
            .message
            .contains("entry config section name 'project' is reserved"),
        "got {}",
        error.message
    );
}

#[test]
fn validate_and_freeze_rejects_invalid_nested_record_defaults() {
    use crate::compiler_frontend::canonical_type_identity::{
        CanonicalBuiltinType, CanonicalTypeIdentity,
    };
    use crate::compiler_frontend::folded_value::{
        OwnedFoldedString, PublicFoldedField, PublicFoldedValue,
    };

    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let nested = schema
        .add_node("nested", UnknownFieldPolicy::Reject)
        .expect("schema is unfrozen");
    schema
        .register_field(
            nested,
            ConfigSchemaField::fixed_scalar("count", FixedScalar::I32).required(),
        )
        .expect("schema is unfrozen");
    schema
        .register_field(
            schema.root(),
            ConfigSchemaField::record("site", nested).default(PublicFoldedValue::Record(vec![
                PublicFoldedField {
                    name: "count".to_owned(),
                    type_identity: CanonicalTypeIdentity::Builtin(
                        CanonicalBuiltinType::FixedScalar(FixedScalar::I32),
                    ),
                    value: PublicFoldedValue::String(OwnedFoldedString::Text("nope".to_owned())),
                },
            ])),
        )
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("nested record defaults must match the referenced node");
    assert!(
        error.message.contains("default does not match"),
        "got {}",
        error.message
    );
}

#[test]
fn scalar_defaults_require_the_declared_exact_type() {
    use crate::compiler_frontend::folded_value::PublicFoldedValue;

    for (declared, value, accepted) in [
        (
            FixedScalar::I32,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("one fits I32"),
            true,
        ),
        (
            FixedScalar::I32,
            FixedScalarValue::signed(FixedScalar::I64, 1).expect("one fits I64"),
            false,
        ),
        (
            FixedScalar::U64,
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64 owns its maximum"),
            true,
        ),
        (
            FixedScalar::F32,
            FixedScalarValue::binary_float(FixedScalar::F32, -0.0).expect("signed zero fits F32"),
            true,
        ),
        (
            FixedScalar::F32,
            FixedScalarValue::binary_float(FixedScalar::F64, -0.0).expect("signed zero fits F64"),
            false,
        ),
    ] {
        let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
        schema
            .register_field(
                schema.root(),
                ConfigSchemaField::fixed_scalar("value", declared)
                    .default(PublicFoldedValue::FixedScalar(value)),
            )
            .expect("schema is unfrozen");
        assert_eq!(
            schema.validate_and_freeze().is_ok(),
            accepted,
            "default {value:?} must match {declared:?} exactly"
        );
    }
}

fn surface_with_section(mut schema: ConfigSchema) -> BuilderSurface {
    schema
        .validate_and_freeze()
        .expect("valid recursive schema");
    let mut surface = BuilderSurface::with_mandatory_core();
    surface
        .config_schemas
        .register_project_section(NamedConfigSectionSchema {
            name: "builder".into(),
            schema,
            required: false,
        });
    surface
}

#[test]
fn surface_rejects_unstorable_project_section_shapes() {
    let mut record_schema = ConfigSchema::new("builder", UnknownFieldPolicy::Reject);
    let nested = record_schema
        .add_node("nested", UnknownFieldPolicy::Preserve)
        .unwrap();
    record_schema
        .register_field(
            record_schema.root(),
            ConfigSchemaField::record("value", nested),
        )
        .unwrap();

    let mut schemas = vec![record_schema];
    for field in [
        ConfigSchemaField::fixed_scalar("value", FixedScalar::I32),
        ConfigSchemaField::fixed_scalar("value", FixedScalar::I32).default(
            PublicFoldedValue::FixedScalar(FixedScalarValue::signed(FixedScalar::I32, 1).unwrap()),
        ),
        ConfigSchemaField::char("value"),
        ConfigSchemaField::string_collection("value"),
        ConfigSchemaField::optional("value", ConfigFieldShape::FixedScalar(FixedScalar::I32))
            .default(PublicFoldedValue::OptionNone),
        ConfigSchemaField::optional(
            "value",
            ConfigFieldShape::Optional(Box::new(ConfigFieldShape::String)),
        ),
        ConfigSchemaField::optional(
            "value",
            ConfigFieldShape::Optional(Box::new(ConfigFieldShape::Bool)),
        ),
        ConfigSchemaField::bool("dev_output"),
        ConfigSchemaField::optional("release_output", ConfigFieldShape::String),
    ] {
        let mut schema = ConfigSchema::new("builder", UnknownFieldPolicy::Reject);
        schema.register_field(schema.root(), field).unwrap();
        schemas.push(schema);
    }
    schemas.push(ConfigSchema::new("builder", UnknownFieldPolicy::Preserve));

    for schema in schemas {
        let error = surface_with_section(schema)
            .validate()
            .expect_err("unsupported schema");
        assert!(
            matches!(error, BuilderSurfaceValidationError::InvalidDefinition(_)),
            "{error:?}"
        );
    }
}

#[test]
fn surface_rejects_multiple_output_root_owners() {
    let mut first = ConfigSchema::new("first", UnknownFieldPolicy::Reject);
    first
        .register_field(first.root(), ConfigSchemaField::string("dev_output"))
        .unwrap();
    let mut surface = surface_with_section(first);
    let mut second = ConfigSchema::new("second", UnknownFieldPolicy::Reject);
    second
        .register_field(second.root(), ConfigSchemaField::string("release_output"))
        .unwrap();
    second.validate_and_freeze().unwrap();
    surface
        .config_schemas
        .register_project_section(NamedConfigSectionSchema {
            name: "other".into(),
            schema: second,
            required: false,
        });
    let error = surface.validate().expect_err("output roots have one owner");
    assert!(
        matches!(error, BuilderSurfaceValidationError::InvalidDefinition(_)),
        "{error:?}"
    );
}

#[test]
fn surface_accepts_storable_sections_and_rich_entry_schemas() {
    let mut first = ConfigSchema::new("first", UnknownFieldPolicy::Reject);
    first
        .register_field(first.root(), ConfigSchemaField::string("dev_output"))
        .unwrap();
    first
        .register_field(
            first.root(),
            ConfigSchemaField::optional("value", ConfigFieldShape::String),
        )
        .unwrap();
    let mut surface = surface_with_section(first);
    let mut second = ConfigSchema::new("second", UnknownFieldPolicy::Reject);
    second
        .register_field(second.root(), ConfigSchemaField::bool("value"))
        .unwrap();
    second
        .register_field(
            second.root(),
            ConfigSchemaField::optional("enabled", ConfigFieldShape::Bool),
        )
        .unwrap();
    second.validate_and_freeze().unwrap();
    surface
        .config_schemas
        .register_project_section(NamedConfigSectionSchema {
            name: "other".into(),
            schema: second,
            required: false,
        });
    let mut entry = ConfigSchema::new("entry", UnknownFieldPolicy::Preserve);
    entry
        .register_field(entry.root(), ConfigSchemaField::string_collection("values"))
        .unwrap();
    entry
        .register_field(
            entry.root(),
            ConfigSchemaField::optional(
                "value",
                ConfigFieldShape::Optional(Box::new(ConfigFieldShape::String)),
            ),
        )
        .unwrap();
    entry.validate_and_freeze().unwrap();
    surface
        .config_schemas
        .register_entry_section(NamedConfigSectionSchema {
            name: "entry".into(),
            schema: entry,
            required: false,
        });
    surface.validate().expect("supported section shapes");
    HtmlProjectBuilder::new()
        .frontend_surface()
        .validate()
        .expect("HTML surface remains valid");
}
