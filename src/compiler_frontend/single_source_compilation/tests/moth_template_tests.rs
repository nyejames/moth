//! Direct Moth template compilation service tests.
//!
//! WHAT: the service's standalone contract — one in-memory template source in, one folded `content`
//!       string plus that source's warnings out.
//! WHY:  input normalization, ordering and duplicate-path diagnostics are owned by the HTML
//!       project's direct-API tests, which drive real files through the whole request shape. What
//!       those cannot show is that folding a template is a compiler entry point needing no project
//!       request, no builder and no filesystem.

use super::{MothTemplateCompilationRequest, compile_moth_template_source};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

#[test]
fn folds_one_in_memory_template_source_to_its_content_constant() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let folded = compile_moth_template_source(
        MothTemplateCompilationRequest {
            source_path: Path::new("/templates/intro.mtf"),
            source_code: String::from("# Intro"),
            style_directives: &style_directives,
        },
        &mut string_table,
    )
    .expect("an in-memory template source should fold");

    assert_eq!(folded.content, "<h1>Intro</h1>");
    assert!(folded.warnings.is_empty());
}
