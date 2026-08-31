//! Direct Moth template compile orchestration.
//!
//! WHAT: turns one normalized request into ordered source units, compiles each through the
//! compiler's direct Moth template service, and packages the folded documents and warnings.
//! WHY: source collection, the project's style vocabulary and the output shape are project policy.
//! The stage sequence that folds template source into a `content` string is compiler-owned, so this
//! module composes no frontend stage itself.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::single_source_compilation::{
    MothTemplateCompilationRequest, compile_moth_template_source,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::moth_template::input::{
    MothTemplateCompileRequest, MothTemplateSourceUnit,
};
use crate::projects::html_project::moth_template::output::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput,
};
use crate::projects::html_project::style_directives::html_project_style_directives;

pub(crate) fn compile_moth_template(
    request: MothTemplateCompileRequest,
    string_table: &mut StringTable,
) -> Result<MothTemplateCompileOutput, CompilerMessages> {
    let sources = request.collect_sources(string_table)?;

    // The project's directive vocabulary is the same for every source in one request, so it is
    // merged once rather than per document.
    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    let mut documents = Vec::with_capacity(sources.len());
    let mut warnings = Vec::new();

    for source in sources {
        let MothTemplateSourceUnit {
            source_path,
            relative_path,
            source_text,
        } = source;

        match compile_moth_template_source(
            MothTemplateCompilationRequest {
                source_path: &source_path,
                source_code: source_text,
                style_directives: &style_directives,
            },
            string_table,
        ) {
            Ok(folded) => {
                let folded_warnings = folded.warnings;
                let content = match folded.content.into_text() {
                    Some(content) => content,
                    None => {
                        // WHY: Phase 5's direct `.mtf` service parity consumer will replace this
                        //      text-only accessor with structural rendering through the normal
                        //      link plan. The current direct lane provides no file-value
                        //      resolution services, so its content cannot carry pieces yet.
                        let mut messages = CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(
                                "Moth template content did not fold to a string.",
                            ),
                            string_table,
                        );
                        // Every warning gathered so far belongs to the report, in source order:
                        // earlier documents first, then this one's.
                        messages.prepend_diagnostics_preserving_context(
                            warnings.iter().cloned().chain(folded_warnings),
                        );
                        return Err(messages);
                    }
                };
                warnings.extend(folded_warnings);
                documents.push(CompiledMothTemplateDocument {
                    source_path,
                    relative_path,
                    content,
                });
            }
            Err(mut messages) => {
                messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
                return Err(messages);
            }
        }
    }

    Ok(MothTemplateCompileOutput {
        documents,
        warnings,
    })
}
