//! Config schema construction tests.
//!
//! WHAT: covers the schema engine's data invariants: dense node and field identifiers, root
//! identity, node field registration, unknown-field policy storage and shape descriptions.
//! WHY: the validator relies on these facts being total and deterministic; the construction
//! surface is the one place that could break them.

use super::super::BuilderSurface;
use super::super::config_schema::{
    ConfigFieldShape, ConfigSchema, ConfigSchemaField, ConfigSchemas, NamedConfigSectionSchema,
    ProjectFieldConfigPolicies, ProjectFieldConfigPolicy, UnknownFieldPolicy,
};

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
fn project_policy_snapshot_preserves_required_optional_and_unsupported_shapes() {
    let policies = BuilderSurface::with_mandatory_core()
        .config_schemas
        .project()
        .project_field_config_policies();

    assert_eq!(
        policies.shape_for("name"),
        Some(&ConfigFieldShape::String),
        "required scalar fields retain their scalar shape"
    );
    assert_eq!(
        policies.shape_for("version"),
        Some(&ConfigFieldShape::Optional(Box::new(
            ConfigFieldShape::String
        ))),
        "optional primitive fields retain their optional wrapper"
    );
    assert_eq!(
        policies.shape_for("template_const_loop_iteration_limit"),
        Some(&ConfigFieldShape::Int)
    );
    assert_eq!(policies.shape_for("unregistered_metadata"), None);

    let default_policies = ProjectFieldConfigPolicies::default();
    assert_eq!(
        default_policies.policy_for("unregistered_metadata"),
        ProjectFieldConfigPolicy::FixedOnly,
        "the standalone policy default remains conservative"
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
        error.msg.contains("frozen config schema"),
        "mutation after freeze is a compiler error, got {}",
        error.msg
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
    assert_eq!(ConfigFieldShape::Int.describe(), "an integer value");
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
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    schema
        .register_field(
            root,
            ConfigSchemaField::string("title")
                .default(crate::compiler_frontend::folded_value::PublicFoldedValue::Int(1)),
        )
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("an Int default cannot freeze on a String field");
    assert!(
        error.msg.contains("default does not match"),
        "got {}",
        error.msg
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
        error.msg.contains("duplicate field name 'title'"),
        "got {}",
        error.msg
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
        error.msg.contains("both required and have a default"),
        "got {}",
        error.msg
    );
}

#[test]
fn validate_and_freeze_rejects_closed_domain_on_int_field() {
    let mut schema = ConfigSchema::new("test record", UnknownFieldPolicy::Reject);
    let root = schema.root();
    let mut field = ConfigSchemaField::int("count");
    field.allowed_strings = Some(&["one"]);
    schema
        .register_field(root, field)
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("closed strings belong on String fields");
    assert!(
        error
            .msg
            .contains("closed string domain on a non-string shape"),
        "got {}",
        error.msg
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
        error.msg.contains("unknown record node"),
        "got {}",
        error.msg
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
        name: "html",
        schema: first,
        required: true,
    });
    schemas.register_project_section(NamedConfigSectionSchema {
        name: "html",
        schema: second,
        required: false,
    });

    let error = schemas
        .validate()
        .expect_err("duplicate project section names are compiler errors");
    assert!(
        error
            .msg
            .contains("project config section name 'html' is registered more than once"),
        "got {}",
        error.msg
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
        name: "html",
        schema: project_html,
        required: true,
    });
    schemas.register_entry_section(NamedConfigSectionSchema {
        name: "html",
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
        name: "project",
        schema: section,
        required: false,
    });

    let error = schemas
        .validate()
        .expect_err("project is reserved in both registries");
    assert!(
        error
            .msg
            .contains("entry config section name 'project' is reserved"),
        "got {}",
        error.msg
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
        .register_field(nested, ConfigSchemaField::int("count").required())
        .expect("schema is unfrozen");
    schema
        .register_field(
            schema.root(),
            ConfigSchemaField::record("site", nested).default(PublicFoldedValue::Record(vec![
                PublicFoldedField {
                    name: "count".to_owned(),
                    type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
                    value: PublicFoldedValue::String(OwnedFoldedString::Text("nope".to_owned())),
                },
            ])),
        )
        .expect("schema is unfrozen");

    let error = schema
        .validate_and_freeze()
        .expect_err("nested record defaults must match the referenced node");
    assert!(
        error.msg.contains("default does not match"),
        "got {}",
        error.msg
    );
}
