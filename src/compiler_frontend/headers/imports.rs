//! Header-stage import collection.
//!
//! WHAT: parses top-level import clauses into normalized structural provider paths.
//! WHY: imports affect file-local visibility and retained declaration-ordering hints, so their
//! normalization belongs to the header stage rather than AST body parsing.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidImportPathReason};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

pub(super) fn normalize_import_dependency_path(
    import_path: &InternedPath,
    source_file: &InternedPath,
    path_location: &SourceLocation,
    string_table: &StringTable,
) -> Result<InternedPath, Box<CompilerDiagnostic>> {
    if import_path
        .as_components()
        .iter()
        .any(|component| string_table.resolve(*component).ends_with(".moth"))
    {
        return Err(Box::new(CompilerDiagnostic::explicit_moth_extension(
            import_path.to_owned(),
            path_location.clone(),
        )));
    }

    if import_path
        .as_components()
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
    {
        return Err(Box::new(CompilerDiagnostic::invalid_import_path(
            import_path.to_owned(),
            InvalidImportPathReason::ParentDirectorySegment,
            path_location.clone(),
        )));
    }

    let mut import_components = import_path.as_components().iter().copied();
    let Some(first) = import_components.next() else {
        return Ok(import_path.to_owned());
    };

    let first_segment = string_table.resolve(first);
    if first_segment != "." && first_segment != ".." {
        return Ok(import_path.to_owned());
    }

    let mut resolved_components = source_file.as_components().to_vec();
    resolved_components.pop();

    for component in import_path.as_components() {
        match string_table.resolve(*component) {
            "." => {}
            ".." => {
                resolved_components.pop();
            }
            _ => resolved_components.push(*component),
        }
    }

    Ok(InternedPath::from_components(resolved_components))
}
