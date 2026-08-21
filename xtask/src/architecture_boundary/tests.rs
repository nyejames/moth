//! Self-tests for the compiler/build dependency-direction rules.
//!
//! Each rule is proved against fixture text rather than against whatever the tree happens to
//! contain, so a rule keeps its meaning when the tree changes.

use super::{BoundaryRule, audit_architecture_boundary_fragment};

fn rules(relative: &str, content: &str) -> Vec<BoundaryRule> {
    audit_architecture_boundary_fragment(relative, content)
        .into_iter()
        .map(|(rule, _)| rule)
        .collect()
}

#[test]
fn reports_a_build_file_that_binds_module_headers_itself() {
    let findings = audit_architecture_boundary_fragment(
        "src/build_system/create_project_modules/compilation.rs",
        "let headers = bind_module_headers(prepared, registry)?;\n",
    );

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].0, BoundaryRule::ExternalStageOrchestration);
    assert!(
        findings[0].1.contains("bind_module_headers"),
        "the finding should name what it found: {}",
        findings[0].1
    );
}

#[test]
fn reports_a_project_file_that_constructs_an_ast_or_lowers_hir() {
    assert_eq!(
        rules(
            "src/projects/html_project/example.rs",
            "Ast::new(AstBuildInput { headers }, context)\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
    assert_eq!(
        rules(
            "src/projects/html_project/example.rs",
            "use crate::compiler_frontend::hir::hir_builder::lower_module;\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
}

#[test]
fn accepts_stage_zero_source_preparation() {
    // Deciding which source belongs to a module and when to prepare it is scheduling policy, and
    // the canonical architecture boundary records it as an allowed direction.
    let source = "let prepared = prepare_header_syntax(outputs, string_table)?;\n\
                  let tokens = tokenize(&source, &scope, mode, directives, table, None)?;\n\
                  CompilerFrontend::prepare_file_frontend_local(&context, input, table)\n";

    assert!(
        rules(
            "src/build_system/create_project_modules/module_preparation.rs",
            source
        )
        .is_empty()
    );
}

#[test]
fn accepts_a_stage_owner_named_in_a_comment() {
    // Naming the consumer of the data a module produces is how a handoff is documented.
    assert!(
        rules(
            "src/build_system/create_project_modules/module_preparation.rs",
            "    ///      `bind_module_headers` consumes the retained syntax later.\n"
        )
        .is_empty()
    );
}

#[test]
fn reports_the_facade_wrapper_around_a_banned_owner() {
    // `CompilerFrontend::sort_headers` is a one-line wrapper over `resolve_module_dependencies`,
    // so banning only the wrapped function would leave the rule bypassable.
    assert_eq!(
        rules(
            "src/build_system/create_project_modules/compilation.rs",
            "let sorted = compiler.sort_headers(headers)?;\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
}

#[test]
fn reports_build_code_completing_generated_semantics() {
    // The style guide names this one directly: no build-owned function installs call summaries,
    // rewrites HIR or reruns a compiler analysis.
    assert_eq!(
        rules(
            "src/build_system/create_project_modules/generated_store.rs",
            "install_exact_concrete_call_summaries(&mut context, &hir, &borrow)?;\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
    assert_eq!(
        rules(
            "src/build_system/create_project_modules/generated_store.rs",
            "let analysis = run_generated_summary_convergence(&mut transaction)?;\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
}

#[test]
fn reports_build_code_projecting_a_public_interface() {
    assert_eq!(
        rules(
            "src/build_system/create_project_modules/compilation.rs",
            "let seed = build_direct_export_seed(&headers, &symbols)?;\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
}

#[test]
fn reports_the_compiler_reaching_into_a_container_through_a_braced_import() {
    assert_eq!(
        rules(
            "src/compiler_frontend/module_compilation/service.rs",
            "use crate::projects::settings::{Config, IMPLICIT_START_FUNC_NAME};\n"
        ),
        vec![BoundaryRule::CompilerDependencyOnBuild]
    );
    assert_eq!(
        rules(
            "src/compiler_frontend/module_compilation/service.rs",
            "use crate::build_system::{create_project_modules, output};\n"
        ),
        vec![BoundaryRule::CompilerDependencyOnBuild]
    );
}

#[test]
fn the_integration_runner_is_production_code_and_is_not_exempt() {
    // `src/compiler_tests/` also holds `integration_test_runner`, which ships without
    // `#[cfg(test)]`. Exempting the directory would blind the rule to a production subtree.
    assert_eq!(
        rules(
            "src/compiler_tests/integration_test_runner/runner.rs",
            "let headers = bind_module_headers(prepared, registry)?;\n"
        ),
        vec![BoundaryRule::ExternalStageOrchestration]
    );
}

#[test]
fn accepts_a_test_that_drives_a_stage_directly() {
    let source = "let headers = bind_module_headers(prepared, registry)?;\n";

    assert!(rules("src/build_system/tests/module_preparation_tests.rs", source).is_empty());
    assert!(rules("src/compiler_tests/frontend_pipeline_tests.rs", source).is_empty());
}

#[test]
fn accepts_the_compiler_running_its_own_stages() {
    assert!(
        rules(
            "src/compiler_frontend/module_compilation/service.rs",
            "let headers = bind_module_headers(prepared, registry)?;\n"
        )
        .is_empty()
    );
}

#[test]
fn reports_the_compiler_reaching_into_the_build_system_or_project_config() {
    assert_eq!(
        rules(
            "src/compiler_frontend/module_compilation/service.rs",
            "use crate::build_system::create_project_modules::generated_store::Store;\n"
        ),
        vec![BoundaryRule::CompilerDependencyOnBuild]
    );
    assert_eq!(
        rules(
            "src/compiler_frontend/module_compilation/service.rs",
            "fn options(config: &settings::Config) -> FrontendOptions {}\n"
        ),
        vec![BoundaryRule::CompilerDependencyOnBuild]
    );
}

#[test]
fn accepts_the_compiler_reading_an_authored_name_constant() {
    // The settings module is not banned wholesale; the configuration container is.
    assert!(
        rules(
            "src/compiler_frontend/headers/module_symbols.rs",
            "use crate::projects::settings::IMPLICIT_START_FUNC_NAME;\n"
        )
        .is_empty()
    );
}

#[test]
fn a_name_embedded_in_a_longer_identifier_is_not_a_hit() {
    assert!(
        rules(
            "src/build_system/example.rs",
            "let checked = deferred_check_borrows_report;\n"
        )
        .is_empty()
    );
}
