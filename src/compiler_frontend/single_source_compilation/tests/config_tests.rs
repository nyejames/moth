//! Project config compilation service tests.
//!
//! WHAT: the service's standalone contract — one authored source in, owned folded declarations,
//!       authored key-name spans and the live source-span builder out.
//! WHY:  the config dialect's rejections are owned by the `config_*` integration cases, which run a
//!       whole build and assert exact diagnostic codes. What those cases cannot show is that config
//!       compilation is a compiler entry point at all: that it needs no `Config`, no build-system
//!       state and no filesystem access to produce the values Stage 0 applies, and that its
//!       dialect rejections happen inside the service before any declaration is handed back.

use super::{CompiledConfigSource, ConfigCompilationRequest, compile_config_source};
use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, DiagnosticPayload, InvalidConfigReason, TypeAnnotationContext,
};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::source::ExtendedSpanBuilder;
use crate::compiler_frontend::source::{SourceDatabase, SourceId, SourceKind};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};
use std::path::Path;

fn compile_project_source(
    source_code: &str,
    inputs: &crate::compiler_frontend::build_config::BuildConfigInputSet,
    globals: &crate::compiler_frontend::build_config::BuilderConfigGlobalSet,
) -> Result<(CompiledConfigSource, StringTable), CompilerMessages> {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let outcome = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
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
    );
    Ok((outcome.result?, string_table))
}

#[test]
fn compiles_one_authored_source_to_folded_declarations_and_key_spans() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
            source_code:
                "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
    .result
    .expect("an authored config source should compile to folded declarations");

    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the authored project record should reach the folded declarations");
    assert!(matches!(project.value, PublicFoldedValue::Record(_)));
    assert!(project.name_span.is_some());
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("project value must be a record");
    };
    assert_eq!(
        fields.len(),
        project.direct_field_spans.len(),
        "direct field spans must align with folded record fields"
    );
    assert!(
        fields.iter().any(|field| field.name == "name"),
        "folded record fields must retain compiler-owned project fields"
    );
}

#[test]
fn projects_authored_anonymous_const_records() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
            source_code: "labels #= (\n    first = \"a\",\n)\n",
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
    .result
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
fn diagnosed_late_config_stage_retains_tokenizer_span_builder() {
    let long_value = "x".repeat(1500);
    let source_code = format!("value #= \"{long_value}\"\nentry_root = \"src\"\n");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let authored_path = Path::new("project/config.moth");
    let canonical_path = Path::new("/project/config.moth");
    let mut source_files = SourceDatabase::empty();
    let file_id = source_files
        .insert(
            canonical_path.to_path_buf(),
            SourceKind::Compiler(SourceFileKind::Moth),
            canonical_path,
            None,
            &mut string_table,
        )
        .expect("the config source should register");
    source_files
        .retain_text(file_id, source_code)
        .expect("the snapshot should load");
    let source_code = source_files
        .retained_text(file_id)
        .expect("the snapshot should remain owned");
    let authored_scope = path_fork
        .try_intern_filesystem_path(authored_path, &mut string_table)
        .expect("the authored path should be UTF-8");
    // A reference lexer pass captures the literal's local span; the span rows land in a
    // throwaway reference builder, so the service's independent builder must re-encode the
    // same bytes for the retained-span assertion below.
    let mut reference_builder = ExtendedSpanBuilder::new();
    let reference_tokens = tokenize(
        source_code,
        authored_scope,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        file_id,
        &mut reference_builder,
    )
    .expect("the config should tokenize before its later dialect rejection");
    let literal_span = (0..reference_tokens.tokens.len())
        .filter_map(|index| {
            let token_index =
                TokenIndex::try_from_index(index).expect("the reference token index should fit");
            let token = reference_tokens
                .tokens
                .token(token_index)
                .expect("the reference token index should resolve");
            if token.tag() != TokenTag::STRING_SLICE_LITERAL {
                return None;
            }
            assert_eq!(
                token
                    .string_spelling(&string_table)
                    .expect("the string literal payload should resolve"),
                Some(long_value.as_str())
            );
            Some(token.span())
        })
        .next()
        .expect("the reference lexer should produce the long literal");
    drop(reference_tokens);
    drop(reference_builder);

    let outcome = compile_config_source(
        ConfigCompilationRequest {
            authored_path,
            file_id,
            source_code,
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
    );

    let messages = outcome
        .result
        .err()
        .expect("a later config dialect rejection should produce diagnostics");
    assert!(messages.diagnostics().any(|diagnostic| {
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::PlainBindingUnsupported,
                ..
            }
        )
    }));
    assert_eq!(outcome.file_id, file_id);
    let builder = &outcome.span_builder;
    let literal_range = literal_span.resolve_with(builder.resolver());
    assert_eq!(
        source_code.get(literal_range.start() as usize..literal_range.end() as usize),
        Some(format!("\"{long_value}\"").as_str())
    );
}

#[test]
fn rejects_config_local_nominal_values_with_structured_diagnostics() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
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
    .result
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
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
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
    .result
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
fn rejects_implicit_sibling_field_reference_inside_a_grouped_project_record() {
    // Record fields resolve through the enclosing constant scope only: a sibling field name
    // is not a constant, so reusing it must be rejected instead of resolving implicitly.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
            source_code: "project #= (\n    name = \"docs\",\n    alias = name,\n)\n",
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
    .result
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
    // Builder and tooling section fields cannot declare `#Config`; only top-level source
    // compile-time declarations carry contracts. The service rejects this before any folded
    // section value reaches build-side validation.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let messages = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            file_id: SourceId::COMPILATION_ROOT,
            source_code: "origin #Config of String = \"/docs\"\nhtml #= (\n    origin = origin,\n)\n",
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
    .result
    .expect(
        "a top-level contract referenced by a builder section field folds without a qualifier error",
    );

    let compiled = messages;
    let html = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "html")
        .expect("the builder section should fold");
    let PublicFoldedValue::Record(fields) = &html.value else {
        panic!("the builder section should fold to a record");
    };
    assert!(matches!(
        fields.iter().find(|field| field.name == "origin").map(|field| &field.value),
        Some(PublicFoldedValue::String(OwnedFoldedString::Text(value))) if value == "/docs"
    ));
}

#[test]
fn declaration_config_bootstrap_resolves_explicit_input_before_project_fold() {
    let mut inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    inputs
        .insert(
            crate::compiler_frontend::build_config::BuildConfigInputEntry::new(
                crate::compiler_frontend::build_config::BuildInputName::new("release_version")
                    .expect("release_version is valid"),
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
        "release_version #Config of String = \"authored\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("the declaration-owned input should fold before the project field");
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
    let input_record = compiled
        .resolution_records
        .iter()
        .find(|record| record.input_name.as_str() == "release_version")
        .expect("the declaration-owned input record should be retained");
    assert_eq!(
        input_record.origin,
        crate::compiler_frontend::build_config::BuildConfigValueOrigin::ExplicitInput
    );
}

#[test]
fn declaration_config_bootstrap_validates_default_before_explicit_provider() {
    let mut inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    inputs
        .insert(
            crate::compiler_frontend::build_config::BuildConfigInputEntry::new(
                crate::compiler_frontend::build_config::BuildInputName::new("release_version")
                    .expect("release_version is valid"),
                crate::compiler_frontend::build_config::PrimitiveBuildValue::String(
                    "provided".to_owned(),
                ),
                crate::compiler_frontend::build_config::BuildConfigValueLocation::Command(
                    crate::compiler_frontend::build_config::BuildCommandLocation::new(0),
                ),
            ),
        )
        .expect("the explicit input name is unique");
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "release_version #Config of String = 1\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n",
        &inputs,
        &globals,
    ) else {
        panic!("an invalid authored fallback must fail before an explicit provider is applied");
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
fn declaration_config_bootstrap_uses_builder_global_and_validates_default() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let mut globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    globals
        .insert(
            crate::compiler_frontend::build_config::BuildInputName::new("release_version")
                .expect("release_version is valid"),
            crate::compiler_frontend::build_config::PrimitiveBuildValue::String(
                "builder".to_owned(),
            ),
        )
        .expect("the builder global name is unique");
    let (compiled, string_table) = compile_project_source(
        "release_version #Config of String = \"authored\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("the builder global should override a valid authored default");
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

    let Err(messages) = compile_project_source(
        "release_version #Config of String = 1\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n",
        &inputs,
        &globals,
    ) else {
        panic!("an invalid authored fallback must fail even when a builder global exists");
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
fn declaration_config_bootstrap_supports_required_omission_and_optional_absence() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "release_version #Config of String = \"authored\"\n\
         optional_author #Config of String?\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
             author = optional_author,\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("a defaulted required input and optional absence should fold");
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
fn declaration_config_bootstrap_resolves_required_omission_and_rejects_missing_input() {
    let mut inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    inputs
        .insert(
            crate::compiler_frontend::build_config::BuildConfigInputEntry::new(
                crate::compiler_frontend::build_config::BuildInputName::new("release_version")
                    .expect("release_version is valid"),
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
        "release_version #Config of String\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("an explicit input should satisfy an omitted required declaration");
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

    let empty_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let Err(messages) = compile_project_source(
        "release_version #Config of String\n\
         project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n",
        &empty_inputs,
        &globals,
    ) else {
        panic!("an omitted required declaration must reject a missing input");
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
fn declaration_config_bootstrap_preserves_multiple_folded_input_dependencies() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "first_value #Config of Int = 1\n\
         second_value #Config of Int = 2\n\
         project #= (\n\
             name = \"docs\",\n\
             aggregate = first_value + second_value,\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("a folded project field should retain every declaration-owned dependency");
    let dependency = compiled
        .project_field_dependencies
        .iter()
        .find(|dependency| string_table.resolve(dependency.field_name) == "aggregate")
        .expect("the aggregate field dependency should be retained");
    let input_names = dependency
        .input_names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(input_names, vec!["first_value", "second_value"]);
}
#[test]
fn declaration_config_bootstrap_tracks_template_fold_provenance() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "release_version #Config of String = \"1.2\"\n\
         project #= (\n\
             name = \"docs\",\n\
             aggregate = [: v-[release_version]],\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("a template-folded project field should retain declaration-owned provenance");
    let project = compiled
        .declarations
        .iter()
        .find(|declaration| string_table.resolve(declaration.name) == "project")
        .expect("the project declaration should be present");
    let PublicFoldedValue::Record(fields) = &project.value else {
        panic!("the project declaration should fold to a record");
    };
    assert!(matches!(
        fields.iter().find(|field| field.name == "aggregate").map(|field| &field.value),
        Some(PublicFoldedValue::String(OwnedFoldedString::Text(value))) if value == " v-1.2"
    ));
    let dependency = compiled
        .project_field_dependencies
        .iter()
        .find(|dependency| string_table.resolve(dependency.field_name) == "aggregate")
        .expect("the template-folded aggregate field dependency should be retained");
    let input_names = dependency
        .input_names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(input_names, vec!["release_version"]);
}

#[test]
fn declaration_config_bootstrap_rejects_fixed_project_dependence_through_multiple_inputs() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "first_value #Config of Int = 1\n\
         second_value #Config of Int = 2\n\
         project #= (\n\
             name = \"docs\",\n\
             template_const_loop_iteration_limit = first_value + second_value,\n\
         )\n",
        &inputs,
        &globals,
    ) else {
        panic!("fixed-only project fields must reject multiple input dependence");
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
fn declaration_config_bootstrap_rejects_forward_references_in_same_file() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "project #= (\n\
             name = \"docs\",\n\
             version = release_version,\n\
         )\n\
         release_version #Config of String = \"authored\"\n",
        &inputs,
        &globals,
    ) else {
        panic!("same-file forward references must remain invalid");
    };
    assert!(messages.diagnostics().next().is_some());
}

#[test]
fn declaration_config_bootstrap_rejects_fixed_project_dependence_through_nested_alias() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let result = compile_project_source(
        "source_root #Config of String = \"src\"\n\
         helper #= (root = source_root,)\n\
         project #= (\n\
             name = \"docs\",\n\
             entry_root = helper.root,\n\
         )\n",
        &inputs,
        &globals,
    );
    let messages = match result {
        Err(messages) => messages,
        Ok(_) => {
            panic!(
                "fixed-only fields must reject input dependence through aliases and projections"
            );
        }
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
fn declaration_config_uses_explicit_typed_input() {
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
        "version #Config of String\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn declaration_config_uses_builder_global_before_default() {
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
        "version #Config of String = \"authored\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn declaration_config_uses_authored_default() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "version #Config of String = \"authored\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn declaration_config_optional_absence_folds_to_option_none() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "author #Config of String?\n\
         project #= (\n\
             name = \"docs\",\n\
             author = author,\n\
         )\n",
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
fn declaration_config_open_metadata_supports_all_primitive_and_optional_contracts() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "custom_string #Config of String = \"text\"\n\
         custom_int #Config of Int = 7\n\
         custom_float #Config of Float = 1.25\n\
         custom_bool #Config of Bool = true\n\
         custom_char #Config of Char = 'c'\n\
         custom_string_optional #Config of String? = none\n\
         custom_int_optional #Config of Int? = 9\n\
         project #= (\n\
             name = \"docs\",\n\
             custom_string = custom_string,\n\
             custom_int = custom_int,\n\
             custom_float = custom_float,\n\
             custom_bool = custom_bool,\n\
             custom_char = custom_char,\n\
             custom_string_optional = custom_string_optional,\n\
             custom_int_optional = custom_int_optional,\n\
         )\n",
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
fn declaration_config_required_reports_missing_input() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "version #Config of String\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn declaration_config_rejects_fixed_fields() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "name #Config of String = \"override\"\n\
         project #= (\n\
             name = name,\n\
         )\n",
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
fn declaration_config_rejects_fixed_entry_root_field() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "entry_root #Config of String = \"src\"\n\
         project #= (\n\
             name = \"docs\",\n\
             entry_root = entry_root,\n\
         )\n",
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
    assert!(
        diagnostic.primary_span.is_some(),
        "fixed-field diagnostics should retain their authored source span"
    );

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
fn declaration_config_rejects_nominal_contract_types() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "version #Config of Version = \"1.0\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn declaration_config_rejects_mismatched_typed_input() {
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
        "version #Config of String\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn declaration_config_validates_authored_default_before_provider() {
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
        "version #Config of String = 7\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
fn config_source_contract_is_accepted_in_project_config_file() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, table) = compile_project_source(
        "version #Config of String = \"source\"\n\
         project #= (\n\
             name = \"docs\",\n\
         )\n",
        &inputs,
        &globals,
    )
    .expect("top-level config declarations are valid bootstrap inputs");
    let version = compiled
        .declarations
        .iter()
        .find(|declaration| table.resolve(declaration.name) == "version")
        .expect("the top-level input declaration should be projected");
    assert!(matches!(
        &version.value,
        PublicFoldedValue::String(OwnedFoldedString::Text(value)) if value == "source"
    ));
}

#[test]
fn config_qualifier_requires_adjacent_hash_and_config_tokens() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let Err(messages) = compile_project_source(
        "version # Config of String = \"v\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
        "version #Config of = \"v\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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
        "version\n\
         #Config of String = \"v\"\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
        &inputs,
        &globals,
    );
    assert!(
        result.is_err(),
        "a config qualifier must stay attached to its field name"
    );
}

#[test]
fn declaration_config_accepts_folded_optional_default() {
    let inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let globals = crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new();
    let (compiled, string_table) = compile_project_source(
        "fallback #String? = \"fallback\"\n\
         version #Config of String? = fallback\n\
         project #= (\n\
             name = \"docs\",\n\
             version = version,\n\
         )\n",
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

#[test]
fn preparation_config_diagnostics_retain_their_original_source_spans() {
    let cases = [
        (
            "value #= 1\nvalue #= 2\n",
            "invalid_config.duplicate_key",
            "value",
        ),
        (
            "@core/math sin\n",
            "invalid_dependency_clause.dependency_clause_not_allowed",
            "@core/math",
        ),
        (
            "logo #= @assets/logo.svg\n",
            "invalid_config.file_value_path_unsupported",
            "@assets/logo.svg",
        ),
        (
            "helper ||:\n;\n",
            "invalid_config.function_unsupported",
            "helper",
        ),
    ];
    let long_value = "🦋".repeat(600);
    for (suffix, expected_reason, expected_text) in cases {
        let source = format!("padding #= \"{long_value}\"\n{suffix}");
        let mut strings = StringTable::new();
        let _path_fork = PathInternerFork::empty();
        let surface = BuilderSurface::with_mandatory_core();
        let directives = StyleDirectiveRegistry::built_ins();
        let authored_path = Path::new("project/config.moth");
        let canonical_path = Path::new("/project/config.moth");
        let sources = SourceDatabase::build([canonical_path], canonical_path, None, &mut strings)
            .expect("the config source registers before compilation");
        let file_id = sources.get_by_canonical_path(canonical_path).unwrap().id;
        let outcome = compile_config_source(
            ConfigCompilationRequest {
                authored_path,
                file_id,
                source_code: &source,
                style_directives: &directives,
                binding_packages: &surface.binding_packages,
                build_config_inputs:
                    &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
                builder_config_globals:
                    &crate::compiler_frontend::build_config::BuilderConfigGlobalSet::new(),
                project_field_config_policies: surface
                    .config_schemas
                    .project()
                    .project_field_config_policies(),
            },
            &mut strings,
        );
        let messages = outcome
            .result
            .err()
            .expect("preparation must reject this config surface");
        let diagnostics = messages.diagnostics().collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "fixture {suffix:?}");
        let diagnostic = diagnostics[0];
        assert_eq!(
            diagnostic.identity().reason_key,
            Some(expected_reason),
            "fixture {suffix:?}"
        );
        let span = diagnostic
            .primary_span
            .expect("preparation must retain the primary source span");
        assert_eq!(span.source(), file_id);
        let range = span.resolve_with(outcome.span_builder.resolver_for(file_id));
        let expected_start = source.rfind(expected_text).unwrap() as u32;
        assert_eq!(
            (range.start(), range.end()),
            (expected_start, expected_start + expected_text.len() as u32),
            "fixture {suffix:?}"
        );
        assert!(
            diagnostic.labels.is_empty(),
            "config retains its labelless primary presentation"
        );
        assert!(
            !outcome.span_builder.is_empty(),
            "the original table must retain the long token"
        );
    }
}
