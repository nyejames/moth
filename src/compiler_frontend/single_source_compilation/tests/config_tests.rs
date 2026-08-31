//! Project config compilation service tests.
//!
//! WHAT: the service's standalone contract — one authored source in, folded AST values, authored
//!       scope identity and authored key-name spans out.
//! WHY:  the config dialect's rejections are owned by the `config_*` integration cases, which run a
//!       whole build and assert exact diagnostic codes. What those cases cannot show is that config
//!       compilation is a compiler entry point at all: that it needs no `Config`, no build-system
//!       state and no filesystem access to produce the values Stage 0 applies.

use super::{ConfigCompilationRequest, compile_config_source};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

#[test]
fn compiles_one_authored_source_to_folded_values_and_key_spans() {
    let mut string_table = StringTable::new();
    let surface = BuilderSurface::with_mandatory_core();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let compiled = compile_config_source(
        ConfigCompilationRequest {
            authored_path: Path::new("project/config.moth"),
            canonical_path: Path::new("/project/config.moth"),
            source_code: "entry_root #= \"src\"\n",
            style_directives: &style_directives,
            binding_packages: &surface.binding_packages,
        },
        &mut string_table,
    )
    .expect("an authored config source should compile to folded values");

    let expected_scope =
        InternedPath::try_from_filesystem_path(Path::new("project/config.moth"), &mut string_table)
            .expect("the authored path is UTF-8");
    assert_eq!(compiled.authored_scope, expected_scope);

    let entry_root = compiled
        .ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(&string_table) == Some("entry_root"))
        .expect("the authored key should reach folded module constants");
    let value = compiled
        .ast
        .const_values
        .string_value(entry_root.id)
        .expect("the authored key should fold to a string");
    assert_eq!(string_table.resolve(value), "src");

    // The key-name span is the service's own output: it is captured before AST consumes the
    // headers, so nothing downstream can rebuild it.
    let key_location = compiled
        .authored_key_name_locations
        .get(entry_root.path)
        .expect("the authored key should carry its name span");
    assert_eq!(key_location.scope, expected_scope);
}
