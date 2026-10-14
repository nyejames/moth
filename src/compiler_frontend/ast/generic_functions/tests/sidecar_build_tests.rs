//! Requester contracts survive detachment and translate into an independent generated domain.
use super::*;
use crate::compiler_frontend::ast::AstBuildContext;
use crate::compiler_frontend::ast::const_values::store::{ConstStringValue, ConstValuePayload};
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironmentBuilder,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicConstTemplatePiece};
use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::semantic_identity::{
    ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity, ModuleRootRole,
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;
use std::cell::RefCell;
use std::sync::Arc;

#[test]
fn requester_template_contract_slots_use_nonidentity_generated_domain_remap() {
    let (mut build_result, mut path_fork, mut string_table) =
        parse_single_file_ast_build_result(concat!(
            "probe type T, E |value T, err E, label String = [:requester-default[$slot]]| -> T, E, E!:\n",
            "    return value, err\n",
            ";\n",
        ))
        .expect("generic native-channel signature should build");
    let identity = ModulePrivateExecutableIdentity::new(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("requester-contract"),
            "main".to_owned(),
            ModuleRootRole::Normal,
        ),
        "@page.moth".to_owned(),
        ModulePrivateExecutableCategory::GenericFunction,
        "probe".to_owned(),
        None,
    );
    let template = build_result
        .materialisation_context
        .as_mut()
        .expect("generic build should retain its context")
        .generic_function_templates_mut()
        .values_mut()
        .next()
        .expect("probe template should be retained");
    template.declaration_identity = Some(GeneratedDeclarationIdentity::ModulePrivate(
        identity.clone(),
    ));
    let method_path = template.function_path;
    let requester_template = template.clone();
    let capture = build_result
        .materialisation_context
        .take()
        .expect("generic build should retain its context");
    let signatures = Rc::downgrade(&capture.context().resolved_function_signatures_by_path);
    let fields = Rc::downgrade(&capture.context().resolved_struct_fields_by_path);
    let requester = capture
        .finish_preparation(
            &build_result.ast.const_values,
            &ModuleResourceTable::new(),
            &path_fork,
        )
        .expect("requester preparation should retain the template identity");
    drop(build_result);
    assert!(signatures.upgrade().is_none());
    assert!(fields.upgrade().is_none());
    let entry_dir = path_fork
        .parent(requester_template.source_file)
        .unwrap_or(PathId::ROOT);
    let resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let build_context = AstBuildContext {
        external_package_registry: Arc::clone(&requester.external_package_registry),
        style_directives: &requester.style_directives,
        string_table: &mut string_table,
        path_fork: &mut path_fork,
        entry_dir,
        root_role: ModuleRootRole::Support,
        build_profile: requester.build_profile,
        file_value_resolution: None,
        config_resolution: None,
        build_config_values: Arc::new(Default::default()),
        template_const_loop_iteration_limit: requester.template_const_loop_iteration_limit,
        capacity_estimate: Default::default(),
        #[cfg(feature = "timers")]
        timing_context: None,
        #[cfg(feature = "timers")]
        timing_metric_family:
            crate::compiler_frontend::ast::module_ast::build_context::AstTimingMetricFamily::Generated,
    };
    let (mut phase_context, strings, paths) =
        AstPhaseContext::from_build_context(build_context, Arc::new(Default::default())).unwrap();
    let builtin_manifest =
        crate::compiler_frontend::builtins::error_type::register_builtin_error_types(
            paths, strings,
        );
    let mut module_symbols = ModuleSymbols::empty();
    module_symbols
        .builtin_visible_symbol_paths
        .extend(builtin_manifest.visible_symbol_paths);
    module_symbols.compiler_owned_declarations = builtin_manifest.declarations;
    module_symbols
        .resolved_struct_fields_by_path
        .extend(builtin_manifest.resolved_struct_fields_by_path);
    module_symbols
        .struct_source_by_path
        .extend(builtin_manifest.struct_source_by_path);
    let mut generated = AstModuleEnvironmentBuilder::new(&mut phase_context, paths)
        .build(
            &[],
            AstEnvironmentInput {
                module_symbols,
                binding_environment: HeaderBindingEnvironment::default(),
                source_token_owners: Default::default(),
            },
            strings,
        )
        .expect("the fresh generated environment should build");
    // Force a distinct type domain and displace both generic slots. An inherited snapshot
    // would let the raw requester IDs accidentally satisfy this invariant.
    generated.type_environment = TypeEnvironment::new();
    for index in 0..8 {
        generated
            .type_environment
            .register_synthetic_generic_parameter(
                strings.intern(&format!("generated-padding-{index}")),
            );
    }
    Rc::make_mut(&mut generated.lookups)
        .generic_function_templates_by_path
        .clear();
    let requester_contract = requester
        .generic_callable_for_path(&method_path)
        .expect("detachment retains the requester template contract");
    let mut value_services = GeneratedValueMaterialisationServices {
        external_registry: &requester.external_package_registry,
        template_ir_store: &phase_context.template_ir_store,
        module_resources: resources,
        expression_builder: &mut phase_context.expression_builder,
        const_values: &mut phase_context.const_values,
    };
    install_generated_requester_template(
        method_path,
        requester_contract,
        &requester,
        &mut generated,
        &mut value_services,
        &requester,
        strings,
        paths,
    )
    .expect("requester slots should translate into the generated domain");
    let contract = generated_requester_template_contract(
        method_path,
        SourceFunctionTarget::ModulePrivate {
            identity,
            local_path: method_path,
        },
        &generated,
    )
    .expect("installed template should supply its generated-domain contract");
    let installed = &generated.lookups.generic_function_templates_by_path[&method_path];
    let generated_parameters = generated
        .type_environment
        .generic_parameters(installed.generic_parameter_list_id)
        .expect("installed template should own generated parameters");
    let slots = generated_parameters
        .parameters
        .iter()
        .map(|parameter| {
            generated
                .type_environment
                .type_id_for_generic_parameter(parameter.id)
                .expect("generated parameter should have a local type handle")
        })
        .collect::<Vec<_>>();
    assert_eq!(contract.returns, slots);
    assert_eq!(contract.error_return, Some(slots[1]));
    for (generated_slot, requester_slot) in contract
        .returns
        .iter()
        .zip(requester_template.signature.success_return_type_ids())
    {
        assert_ne!(
            *generated_slot, requester_slot,
            "fixture must remap every success slot"
        );
    }
    assert_ne!(
        contract.error_return,
        requester_template.signature.error_return_type_id(),
        "fixture must remap the error slot",
    );
    let default = installed.signature.parameters[2]
        .default
        .expect("requester defaults survive into the independent generated domain");
    assert_eq!(
        installed.signature.parameters[2].type_id,
        builtin_type_ids::STRING,
    );
    assert!(matches!(
        phase_context.const_values.payload(default),
        Some(ConstValuePayload::Template {
            template,
            folded: Some(ConstStringValue::Text(text)),
        }) if strings.resolve(*text) == "requester-default" && matches!(
            template.pieces.first(),
            Some(PublicConstTemplatePiece::Text(OwnedFoldedString::Text(text))) if text == "requester-default"
        )
    ));
    assert!(
        installed.body_tokens.is_none(),
        "the template must remain non-executable"
    );
}
