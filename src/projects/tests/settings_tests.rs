//! Focused tests for the project config to compiler-options projection.

use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::paths::path_format::OutputPathStyle;
use crate::projects::settings::Config;

#[test]
fn frontend_options_use_the_configured_origin_and_loop_limit() {
    let mut config = Config::default();
    config
        .settings
        .insert(String::from("origin"), String::from("/moth"));
    config.template_const_loop_iteration_limit = 42;

    let options = config.frontend_options();

    assert_eq!(options.path_format_config.origin, "/moth");
    assert_eq!(
        options.path_format_config.output_style,
        OutputPathStyle::Portable
    );
    assert_eq!(options.template_const_loop_iteration_limit, 42);
}

#[test]
fn frontend_options_fall_back_to_the_root_origin() {
    let options = Config::default().frontend_options();

    assert_eq!(options.path_format_config.origin, "/");
    assert_eq!(
        options.template_const_loop_iteration_limit,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS
    );
}
