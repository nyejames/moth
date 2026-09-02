//! Project config compilation service tests.
//!
//! WHAT: the service's standalone contract — one authored source in, owned folded declarations
//!       and authored key-name spans out.
//! WHY:  the config dialect's rejections are owned by the `config_*` integration cases, which run a
//!       whole build and assert exact diagnostic codes. What those cases cannot show is that config
//!       compilation is a compiler entry point at all: that it needs no `Config`, no build-system
//!       state and no filesystem access to produce the values Stage 0 applies, and that its
//!       dialect rejections happen inside the service before any declaration is handed back.

use super::{ConfigCompilationRequest, compile_config_source};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidConfigReason, InvalidExpressionReason,
};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

#[test]
fn compiles_one_authored_source_to_folded_declarations_and_key_spans() {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            source_code: "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
            source_code: "labels #= |\n    first = \"a\",\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
            source_code: "Inner = |\n    x Int,\n|\nOuter = |\n    inner Inner,\n|\nouter #= Outer(Inner(1))\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
            source_code: "entry_root = \"src\"\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
            source_code: "project #= |\n    name = \"docs\",\n    child = | value = 1 |,\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
            source_code: "project #= |\n    name = \"docs\",\n    alias = name,\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
    // Builder and tooling section fields cannot declare `#Config`; the later
    // build-configuration-values plan permits the qualifier only on direct project fields and
    // module-wide source contracts. The qualifier is not part of the config record grammar, so
    // the compiler service rejects the field before any folded value exists and the folded
    // boundary never sees a qualified section field.
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            source_code: "html #= |\n    origin #Config of String = \"/docs\",\n|\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
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
            DiagnosticPayload::InvalidExpression {
                reason: InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            }
        )
    }));
}
