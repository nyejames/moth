//! Contracts for diagnostic path facts rendered through an attached path identity.

use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;

/// `MOTH-IMPORT-0026` interns the selected entry file, which callers make absolute before the
/// compile request. The render boundary must display that fact with one root separator for every
/// attached identity source.
#[test]
fn diagnostic_path_context_renders_a_rooted_filesystem_fact_with_one_root_separator() {
    #[cfg(not(windows))]
    let native = PathBuf::from("/project/src/page.mtf");
    #[cfg(not(windows))]
    let expected = "/project/src/page.mtf";

    #[cfg(windows)]
    let native = PathBuf::from(r"C:\project\src\page.mtf");
    #[cfg(windows)]
    let expected = "C:/project/src/page.mtf";

    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let path = builder
        .try_intern_filesystem_path(&native, &mut string_table)
        .expect("test path bytes are UTF-8 on this host");
    let fork_source = builder.fork_source();
    let fork = fork_source.fork_for_module();
    let table = builder.freeze();
    let frozen_strings = string_table.clone().freeze();

    let fork_context = DiagnosticRenderContext::new(&string_table).with_path_fork(&fork);
    assert_eq!(fork_context.render_path(path), expected);

    let table_context = DiagnosticRenderContext::new(&string_table).with_path_table(&table);
    assert_eq!(table_context.render_path(path), expected);

    // Retained reports resolve the same table through frozen strings.
    let frozen_context = DiagnosticRenderContext::new(&frozen_strings).with_path_table(&table);
    assert_eq!(frozen_context.render_path(path), expected);
}
