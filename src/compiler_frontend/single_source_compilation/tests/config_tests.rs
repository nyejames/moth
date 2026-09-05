//! Project config compilation service tests.
//!
//! WHAT: the service's standalone contract — one authored source in, owned folded declarations
//!       and authored key-name spans out.
//! WHY:  the config dialect's rejections are owned by the `config_*` integration cases, which run a
//!       whole build and assert exact diagnostic codes. What those cases cannot show is that config
//!       compilation is a compiler entry point at all: that it needs no `Config`, no build-system
//!       state and no filesystem access to produce the values Stage 0 applies, and that its
//!       dialect rejections happen inside the service before any declaration is handed back.

use super::{CompiledConfigSource, ConfigCompilationRequest, compile_config_source};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, DiagnosticPayload, InvalidConfigReason, InvalidExpressionReason,
    TypeAnnotationContext,
};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

fn compile_project_source(
    source_code: &str,
    inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet,
    globals: &crate::compiler_frontend::build_config::BuilderConfigGlobalSet,
) -> Result<(CompiledConfigSource, StringTable), CompilerMessages> {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code,
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: inputs,
            builder_config_globals: globals,
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )?;
    Ok((compiled, string_table))
}

#[test]
fn compiles_one_authored_source_to_folded_declarations_and_key_spans() {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
            builder_config_globals: &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .expect("an authored config source should compile to folded declarations");

    let expected_scope =
        InternedPath::try_from_filesystem_path(Path::new("project/config.moth"), &mut string_table)
            .expect("the authored path is UTF-8");

    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the authored project record should reach the folded declarations");
    assert!(matches!(project.value, PublicFoldedValue::Record(_)));
    assert_eq!(project.name_location.scope, expected_scope);
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("project value must be a record");
    };
    assert_eq!(
        fields.len(),
        project.direct_field_locations.len(),
        "direct field locations must align with folded record fields"
    );
    assert!(
        fields.iter().any(|field| field.name == "name"),
        "folded record fields must retain compiler-owned project fields"
    );
}

#[test]
fn projects_authored_anonymous_const_records() {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "labels #= |\n    first = \"a\",\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(
            ),
            builder_config_globals:
                &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .expect("an authored anonymous const record should project at the config boundary");

    let labels = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "labels")
        .expect("the authored record should reach the folded declarations");
    let PublicFoldedValue::Record(fields) = &labels.value else {
        panic!("anonymous const records must project as PublicFoldedValue::Record");
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "first");
    assert_eq!(
        fields[0].value,
        PublicFoldedValue::String(OwnedFoldedString::Text("a".to_owned()))
    );
}

#[test]
fn rejects_config_local_nominal_values_with_structured_diagnostics() {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "Inner = |\n    x Int,\n|\nOuter = |\n    inner Inner,\n|\nouter #= Outer(Inner(1))\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
            builder_config_globals: &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .err()
    .expect("config-local nominal values should not become CompilerError");

    let CompilerMessages { diagnostics, .. } = &messages;
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::NamedTypeUnsupported,
                ..
            }
        )
    }));
}

#[test]
fn rejects_authored_plain_bindings_inside_the_service() {
    // `entry_root = "src"` is a plain runtime binding: the service rejects the start-body
    // statement itself instead of handing an AST node to build-side validation.
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "entry_root = \"src\"\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(
            ),
            builder_config_globals:
                &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .err()
    .expect("a plain config binding should be rejected by the compiler service");

    let CompilerMessages { diagnostics, .. } = &messages;
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::PlainBindingUnsupported,
                ..
            }
        )
    }));
}

#[test]
fn rejects_nested_record_literal_inside_a_grouped_project_record() {
    // Nested `|...|` literals are rejected by the shared record grammar inside the service:
    // record-valued children must be declared first and referenced by name.
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "project #= |\n    name = \"docs\",\n    child = | value = 1 |,\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(
            ),
            builder_config_globals:
                &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .err()
    .expect("a nested record literal should be rejected by the compiler service");

    let CompilerMessages { diagnostics, .. } = &messages;
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidExpression {
                reason: InvalidExpressionReason::NestedAnonymousConstRecord,
            }
        )
    }));
}

#[test]
fn rejects_implicit_sibling_field_reference_inside_a_grouped_project_record() {
    // Record fields resolve through the enclosing constant scope only: a sibling field name
    // is not a constant, so reusing it must be rejected instead of resolving implicitly.
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "project #= |\n    name = \"docs\",\n    alias = name,\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(
            ),
            builder_config_globals:
                &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .err()
    .expect("an implicit sibling field reference should be rejected by the compiler service");

    let CompilerMessages { diagnostics, .. } = &messages;
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::UnknownName { name, .. } if string_table.resolve(*name) == "name"
        )
    }));
}

#[test]
fn rejects_config_qualifier_on_builder_section_fields() {
    // Builder and tooling section fields cannot declare `#Config`; the compiler-owned direct
    // project qualifier is intentionally limited to grouped project fields. The service rejects
    // this before any folded section value reaches build-side validation.
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            file_id: None,
            source_code: "html #= |\n    origin #Config of String = \"/docs\",\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
            build_config_inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet::new(
            ),
            builder_config_globals:
                &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
            project_field_config_policies: surface
                .config_schemas
                .project()
                .project_field_config_policies(),
        },
        &mut string_table,
    )
    .err()
    .expect(
        "a #Config qualifier on a builder section field should be rejected by the compiler service",
    );

    let CompilerMessages { diagnostics, .. } = &messages;
    assert!(diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierInvalidProjectPlacement,
                ..
            }
        )
    }));
}

#[test]
fn direct_project_config_qualifier_uses_explicit_typed_input() {
    let mut inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    inputs
        .insert(
            crate::compiler_frontend::build_config::BuildConfigInputEntry::new(
                crate::compiler_frontend::build_config::BuildInputName::new("version")
                    .expect("version is a valid build input name"),
                crate::compiler_frontend::build_config::PrimitiveBuildValue::String(
                    "2.0".to_owned(),
                ),
                crate::compiler_frontend::build_config::BuildConfigValueLocation::Command(
                    crate::compiler_frontend::build_config::BuildCommandLocation::new(0),
                ),
            ),
        )
        .expect("the explicit input name is unique");
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of String,\n|\n",
        &inputs,
        &globals,
    )
    .expect("an explicit typed input should satisfy the project contract");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };
    assert!(matches!(
        fields.iter().find(|field| field.name == "version").map(|field| &field.value),
        Some(PublicFoldedValue::String(OwnedFoldedString::Text(value))) if value == "2.0"
    ));
}

#[test]
fn direct_project_config_qualifier_uses_builder_global_before_default() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let mut globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    globals
        .insert(
            crate::compiler_frontend::build_config::BuildInputName::new("version")
                .expect("version is a valid build input name"),
            crate::compiler_frontend::build_config::PrimitiveBuildValue::String(
                "builder".to_owned(),
            ),
        )
        .expect("builder global name should be platform-neutral");
    let (compiled, string_table) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of String = \"authored\",\n|\n",
        &inputs,
        &globals,
    )
    .expect("a builder global should satisfy the project contract");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };
    assert!(matches!(
        fields.iter().find(|field| field.name == "version").map(|field| &field.value),
        Some(PublicFoldedValue::String(OwnedFoldedString::Text(value))) if value == "builder"
    ));
}

#[test]
fn direct_project_config_qualifier_uses_authored_default() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of String = \"authored\",\n|\n",
        &inputs,
        &globals,
    )
    .expect("an authored default should satisfy the project contract");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };
    assert!(matches!(
        fields.iter().find(|field| field.name == "version").map(|field| &field.value),
        Some(PublicFoldedValue::String(OwnedFoldedString::Text(value))) if value == "authored"
    ));
}

#[test]
fn direct_project_optional_config_qualifier_absence_folds_to_option_none() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    author #Config of String?,\n|\n",
        &inputs,
        &globals,
    )
    .expect("optional config input may be absent");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };
    assert!(matches!(
        fields
            .iter()
            .find(|field| field.name == "author")
            .map(|field| &field.value),
        Some(PublicFoldedValue::OptionNone)
    ));
}
#[test]
fn direct_project_open_metadata_supports_all_primitive_and_optional_contracts() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    custom_string #Config of String = \"text\",\n    custom_int #Config of Int = 7,\n    custom_float #Config of Float = 1.25,\n    custom_bool #Config of Bool = true,\n    custom_char #Config of Char = 'c',\n    custom_string_optional #Config of String? = none,\n    custom_int_optional #Config of Int? = 9,\n|\n",
        &inputs,
        &globals,
    )
    .expect("open project metadata should accept primitive and optional contracts");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };

    let field_value = |name: &str| {
        fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| &field.value)
            .unwrap_or_else(|| panic!("project field '{name}' should be present"))
    };

    assert!(matches!(
        field_value("custom_string"),
        PublicFoldedValue::String(OwnedFoldedString::Text(value)) if value == "text"
    ));
    assert!(matches!(
        field_value("custom_int"),
        PublicFoldedValue::Int(7)
    ));
    assert!(matches!(
        field_value("custom_float"),
        PublicFoldedValue::Float(value) if value.value() == 1.25
    ));
    assert!(matches!(
        field_value("custom_bool"),
        PublicFoldedValue::Bool(true)
    ));
    assert!(matches!(
        field_value("custom_char"),
        PublicFoldedValue::Char('c')
    ));
    assert!(matches!(
        field_value("custom_string_optional"),
        PublicFoldedValue::OptionNone
    ));
    assert!(matches!(
        field_value("custom_int_optional"),
        PublicFoldedValue::OptionSome(value)
            if matches!(value.as_ref(), PublicFoldedValue::Int(9))
    ));
}

#[test]
fn direct_project_required_config_qualifier_reports_missing_input() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of String,\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("a required config contract without input or default must fail");
    };
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::MissingConfigInput,
                ..
            }
        )
    }));
}

#[test]
fn direct_project_config_qualifier_rejects_fixed_fields() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name #Config of String = \"override\",\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("project identity fields must remain fixed-only");
    };
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierFixedField,
                ..
            }
        )
    }));
}
#[test]
fn direct_project_config_qualifier_rejects_fixed_entry_root_field() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    entry_root #Config of String = \"src\",\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("project entry_root must remain fixed-only");
    };

    let diagnostic = messages
        .diagnostics()
        .find(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    key: Some(key),
                    reason: InvalidConfigReason::ConfigQualifierFixedField,
                } if messages.string_table.resolve(*key) == "entry_root"
            )
        })
        .expect("entry_root should produce a typed fixed-field diagnostic");
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("invalid_config.config_qualifier_fixed_field")
    );
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_portable_string(&messages.string_table),
        "project/config.moth"
    );
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 16);
    assert_eq!(diagnostic.primary_location.end_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 16);

    let DiagnosticPayload::InvalidConfig {
        key: Some(key),
        reason: InvalidConfigReason::ConfigQualifierFixedField,
    } = &diagnostic.payload
    else {
        unreachable!("the diagnostic was filtered to the fixed entry_root payload");
    };
    assert_eq!(messages.string_table.resolve(*key), "entry_root");
}

#[test]
fn direct_project_config_qualifier_rejects_nominal_contract_types() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of Version = \"1.0\",\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("nominal contract types must not enter config resolution");
    };
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierUnsupportedType,
                ..
            }
        )
    }));
}
#[test]
fn direct_project_config_qualifier_rejects_known_schema_type_mismatch() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of Int = 1,\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("a known project field must honor its schema contract shape");
    };

    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierSchemaTypeMismatch { .. },
                ..
            }
        )
    }));
}

#[test]
fn direct_project_config_qualifier_rejects_mismatched_typed_input() {
    let mut inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    inputs
        .insert(
            crate::compiler_frontend::build_config::BuildConfigInputEntry::new(
                crate::compiler_frontend::build_config::BuildInputName::new("version")
                    .expect("version is a valid build input name"),
                crate::compiler_frontend::build_config::PrimitiveBuildValue::Int(7),
                crate::compiler_frontend::build_config::BuildConfigValueLocation::Command(
                    crate::compiler_frontend::build_config::BuildCommandLocation::new(0),
                ),
            ),
        )
        .expect("the explicit input name is unique");
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of String,\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("an Int input must not satisfy a String contract");
    };
    let diagnostic = messages
        .diagnostics()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::ConfigInputTypeMismatch { .. },
                    ..
                }
            )
        })
        .expect("an input type mismatch diagnostic should be present");
    let DiagnosticPayload::InvalidConfig { reason, .. } = &diagnostic.payload else {
        unreachable!("the diagnostic was filtered to InvalidConfig");
    };
    assert!(matches!(
        reason,
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided_argument_index: Some(0),
            ..
        }
    ));
}

#[test]
fn direct_project_config_qualifier_validates_authored_default_before_provider() {
    let mut inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    inputs
        .insert(
            crate::compiler_frontend::build_config::BuildConfigInputEntry::new(
                crate::compiler_frontend::build_config::BuildInputName::new("version")
                    .expect("version is a valid build input name"),
                crate::compiler_frontend::build_config::PrimitiveBuildValue::String(
                    "provided".to_owned(),
                ),
                crate::compiler_frontend::build_config::BuildConfigValueLocation::Command(
                    crate::compiler_frontend::build_config::BuildCommandLocation::new(2),
                ),
            ),
        )
        .expect("the explicit input name is unique");
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of String = 7,\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("an invalid authored default must not be masked by a compatible input");
    };
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigInputTypeMismatch { .. },
                ..
            }
        )
    }));
}

#[test]
fn config_source_contract_is_rejected_in_project_config_file() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "version #Config of String = \"source\"\nproject #= |\n    name = \"docs\",\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("project config files may not declare source-wide #Config contracts");
    };

    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierInvalidProjectPlacement,
                ..
            }
        )
    }));
}

#[test]
fn project_config_qualifier_is_rejected_on_non_project_record() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "nested #= |\n    version #Config of String = \"nested\",\n|\nproject #= |\n    name = \"docs\",\n    metadata = nested,\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("#Config qualifiers must be limited to direct project fields");
    };
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierInvalidProjectPlacement,
                ..
            }
        )
    }));
}

#[test]
fn config_qualifier_requires_adjacent_hash_and_config_tokens() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version # Config of String = \"v\",\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("whitespace between # and Config must be rejected");
    };
    assert!(!messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigInputTypeMismatch { .. },
                ..
            }
        )
    }));
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::CommonSyntaxMistake {
                reason: CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing
            }
        )
    }));
}

#[test]
fn config_qualifier_requires_a_contract_type() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version #Config of = \"v\",\n|\n",
        &inputs,
        &globals,
    ) else {
        panic!("a config qualifier must provide an explicit contract type");
    };

    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidTypeAnnotation {
                context: TypeAnnotationContext::BuildConfigContract,
                ..
            }
        )
    }));
}

#[test]
fn config_qualifier_does_not_cross_field_name_newline() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let result = compile_project_source(
        "project #= |\n    name = \"docs\",\n    version\n        #Config of String = \"v\",\n|\n",
        &inputs,
        &globals,
    );
    assert!(
        result.is_err(),
        "a config qualifier must stay attached to its field name"
    );
}

#[test]
fn direct_project_config_qualifier_accepts_folded_optional_default() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "fallback #String? = \"fallback\"\nproject #= |\n    name = \"docs\",\n    version #Config of String? = fallback,\n|\n",
        &inputs,
        &globals,
    )
    .expect("a folded optional helper should satisfy the direct project contract");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };
    assert!(matches!(
        fields
            .iter()
            .find(|field| field.name == "version")
            .map(|field| &field.value),
        Some(PublicFoldedValue::OptionSome(value))
            if matches!(
                value.as_ref(),
                PublicFoldedValue::String(OwnedFoldedString::Text(text)) if text == "fallback"
            )
    ));
}
