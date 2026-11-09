//! Guidance and portable path presentation contracts.

use crate::compiler_frontend::compiler_errors::{InfrastructureDetail, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::render::infrastructure::format_error_guidance_lines;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, relative_display_path_from_root, special_file_name_from_path,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

#[test]
fn guidance_lines_name_the_stage_then_each_rendered_detail() {
    let error =
        InfrastructureFailure::internal(CompilerStage::AstConstruction, "bad compiler state")
            .with_detail(InfrastructureDetail::PrimarySuggestion(
                "Do the thing".into(),
            ))
            .with_detail(InfrastructureDetail::OutputRejectionReason(
                "kept for diagnostics only".into(),
            ));

    assert_eq!(
        format_error_guidance_lines(&error),
        ["Stage: AST construction", "Do the thing"]
    );
}

#[test]
fn guidance_lines_name_the_stage_without_details() {
    let error =
        InfrastructureFailure::internal(CompilerStage::BorrowChecking, "bad compiler state");
    assert_eq!(
        format_error_guidance_lines(&error),
        ["Stage: Borrow checking"]
    );
}

#[test]
fn special_file_renderer_names_only_the_build_only_config_source() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let extensionless_config = path_fork
        .try_intern_portable_path("input/config", &mut string_table)
        .expect("test path fits");
    let explicit_config = path_fork
        .try_intern_portable_path("input/config.moth", &mut string_table)
        .expect("test path fits");
    let ordinary_plus_source = path_fork
        .try_intern_portable_path("input/+notes", &mut string_table)
        .expect("test path fits");
    let context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);

    assert_eq!(
        special_file_name_from_path(extensionless_config, context),
        "config.moth"
    );
    assert_eq!(
        special_file_name_from_path(explicit_config, context),
        "config.moth"
    );
    assert_eq!(
        special_file_name_from_path(ordinary_plus_source, context),
        "special file"
    );
}

#[test]
fn relative_display_path_strips_root_prefix() {
    let root = Path::new("/workspace/project");
    let scope = Path::new("/workspace/project/src/main.moth");

    let relative = relative_display_path_from_root(scope, root);

    assert_eq!(relative, "src/main.moth");
}
