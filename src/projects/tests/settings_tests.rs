//! Focused tests for the project config to compiler-options projection.

use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::projects::settings::Config;

#[test]
fn frontend_options_use_the_configured_loop_limit() {
    let mut config = Config::default();
    config
        .settings
        .insert(String::from("origin"), String::from("/moth"));
    config.template_const_loop_iteration_limit = 42;

    let options = config.frontend_options();

    assert_eq!(options.template_const_loop_iteration_limit, 42);
}

#[test]
fn frontend_options_fall_back_to_the_default_loop_limit() {
    let options = Config::default().frontend_options();

    assert_eq!(
        options.template_const_loop_iteration_limit,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS
    );
}
