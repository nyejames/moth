//! Build-level runtime module emission for HTML JS.
//!
//! WHAT: emits each unique JS runtime module once per build from a pre-built emission plan.
//! WHY: runtime modules are builder-owned assets such as `@moth/runtime`; deduplicating
//!      them at the build level avoids redundant output.

use crate::build_system::build::{FileKind, OutputFile};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::external_js::runtime_glue::paths::runtime_module_output_path;
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;
use std::collections::HashSet;
use std::path::PathBuf;

/// Build-level emission of runtime modules from a pre-built emission plan.
///
/// WHAT: emits each unique runtime module once per build.
/// WHY: the plan already collected and deduplicated required specifiers, so this function
///      only handles registry lookup, output-path conflict checks, and file creation.
pub(crate) fn emit_build_runtime_modules(
    plan: &HtmlExternalRuntimeEmissionPlan,
    occupied_output_paths: &mut HashSet<PathBuf>,
    string_table: &StringTable,
) -> Result<Vec<OutputFile>, CompilerMessages> {
    let registry = RuntimeModuleRegistry::v1();
    let mut files = Vec::with_capacity(plan.runtime_module_specifiers().len());

    for specifier in plan.runtime_module_specifiers() {
        let Some(module_source) = registry.module_source(specifier) else {
            let message = format!(
                "Generated JS runtime module '{}' is required but is not registered.",
                specifier
            );
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(message),
                string_table.clone(),
            ));
        };

        let runtime_path = runtime_module_output_path(specifier);
        if !occupied_output_paths.insert(runtime_path.clone()) {
            let message = format!(
                "Generated JS runtime module output path '{}' conflicts with an existing output.",
                runtime_path.display()
            );
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(message),
                string_table.clone(),
            ));
        }

        files.push(OutputFile::new(
            runtime_path,
            FileKind::Js(module_source.to_owned()),
        ));
    }

    Ok(files)
}
