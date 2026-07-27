//! Tests for public export boundary resolution.
//!
//! WHAT: covers module-root-relative effective path derivation for grouped and direct imports.
//! WHY: the boundary owner must classify same-module imports from nested module roots as
//! `NotAPublicExportBoundary` so they fall through to direct source resolution, while
//! cross-module imports must still resolve through the target module's public surface.

use super::*;
use crate::compiler_frontend::headers::module_symbols::{
    ModuleRootBoundary, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};

fn intern_path(components: &[&str], string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_components(
        components
            .iter()
            .map(|component| string_table.intern(component))
            .collect(),
    )
}

fn empty_exports_for_roots(
    roots: &[InternedPath],
) -> FxHashMap<InternedPath, FxHashSet<PublicExportEntry>> {
    let mut map = FxHashMap::default();
    for root in roots {
        map.entry(root.clone()).or_default();
    }
    map
}

/// A nested module root importing its own ordinary implementation file via a canonical
/// module-root-relative path must be classified as same-module.
#[test]
fn nested_module_root_same_module_import_bypasses_public_surface() {
    let mut string_table = StringTable::new();

    let entry_root = intern_path(&["entry-root"], &mut string_table);
    let helper_root = intern_path(&["helper-root"], &mut string_table);
    let helper_mod_file = intern_path(&["helper", "#mod.moth"], &mut string_table);
    let importer_file = helper_mod_file.clone();

    let mut file_module_membership = FxHashMap::default();
    file_module_membership.insert(helper_mod_file.clone(), helper_root.clone());
    file_module_membership.insert(entry_root.clone(), entry_root.clone());

    let boundaries = vec![ModuleRootBoundary {
        import_prefix: intern_path(&["helper"], &mut string_table),
        module_root: helper_root.clone(),
        root_file: helper_mod_file.clone(),
    }];

    let module_root_public_exports = empty_exports_for_roots(std::slice::from_ref(&helper_root));

    let input = PublicExportResolutionInput {
        importer_file: &importer_file,
        header_path: &intern_path(&["impl", "helper"], &mut string_table),
        source_package_public_exports: &FxHashMap::default(),
        file_package_membership: &FxHashMap::default(),
        module_root_public_exports: &module_root_public_exports,
        file_module_membership: &file_module_membership,
        module_root_boundaries: &boundaries,
        string_table: &string_table,
    };

    let result = resolve_public_export_boundary(&input);
    assert!(
        matches!(
            result,
            Some(PublicExportLookupResult::NotAPublicExportBoundary)
        ),
        "nested module root importing its own file should bypass the public surface"
    );
}

/// An entry-root importer targeting a child module must resolve through that child's
/// public surface, not bypass it.
#[test]
fn cross_module_child_import_resolves_through_public_surface() {
    let mut string_table = StringTable::new();

    let entry_root = intern_path(&["entry-root"], &mut string_table);
    let child_root = intern_path(&["child-root"], &mut string_table);
    let child_mod_file = intern_path(&["child", "#mod.moth"], &mut string_table);
    let page_file = intern_path(&["page.moth"], &mut string_table);

    let mut file_module_membership = FxHashMap::default();
    file_module_membership.insert(page_file.clone(), entry_root.clone());
    file_module_membership.insert(child_mod_file.clone(), child_root.clone());

    let boundaries = vec![ModuleRootBoundary {
        import_prefix: intern_path(&["child"], &mut string_table),
        module_root: child_root.clone(),
        root_file: child_mod_file.clone(),
    }];

    let greet_name = string_table.intern("greet");
    let greet_source = intern_path(&["child", "greet.moth"], &mut string_table);
    let mut child_exports = FxHashSet::default();
    child_exports.insert(PublicExportEntry {
        export_name: greet_name,
        target: PublicExportTarget::Source(greet_source.clone()),
    });

    let mut module_root_public_exports = FxHashMap::default();
    module_root_public_exports.insert(child_root.clone(), child_exports);

    let input = PublicExportResolutionInput {
        importer_file: &page_file,
        header_path: &intern_path(&["child", "greet"], &mut string_table),
        source_package_public_exports: &FxHashMap::default(),
        file_package_membership: &FxHashMap::default(),
        module_root_public_exports: &module_root_public_exports,
        file_module_membership: &file_module_membership,
        module_root_boundaries: &boundaries,
        string_table: &string_table,
    };

    let result = resolve_public_export_boundary(&input);
    assert!(
        matches!(result, Some(PublicExportLookupResult::ExportedSource { ref path, .. }) if path == &greet_source),
        "cross-module child import should resolve through the child public surface"
    );
}

/// A cross-module import requesting a symbol not exported by the child's public surface
/// must be rejected with `NotExported`.
#[test]
fn cross_module_child_import_missing_symbol_is_not_exported() {
    let mut string_table = StringTable::new();

    let entry_root = intern_path(&["entry-root"], &mut string_table);
    let child_root = intern_path(&["child-root"], &mut string_table);
    let child_mod_file = intern_path(&["child", "#mod.moth"], &mut string_table);
    let page_file = intern_path(&["page.moth"], &mut string_table);

    let mut file_module_membership = FxHashMap::default();
    file_module_membership.insert(page_file.clone(), entry_root.clone());
    file_module_membership.insert(child_mod_file.clone(), child_root.clone());

    let boundaries = vec![ModuleRootBoundary {
        import_prefix: intern_path(&["child"], &mut string_table),
        module_root: child_root.clone(),
        root_file: child_mod_file.clone(),
    }];

    let module_root_public_exports = empty_exports_for_roots(&[child_root]);

    let input = PublicExportResolutionInput {
        importer_file: &page_file,
        header_path: &intern_path(&["child", "private"], &mut string_table),
        source_package_public_exports: &FxHashMap::default(),
        file_package_membership: &FxHashMap::default(),
        module_root_public_exports: &module_root_public_exports,
        file_module_membership: &file_module_membership,
        module_root_boundaries: &boundaries,
        string_table: &string_table,
    };

    let result = resolve_public_export_boundary(&input);
    assert!(
        matches!(result, Some(PublicExportLookupResult::NotExported { .. })),
        "cross-module import of a non-exported symbol should be rejected"
    );
}

/// A deeply nested source-package module root importing its own ordinary file with a
/// multi-component canonical path must be classified as same-module.
#[test]
fn source_package_nested_module_root_same_module_import_bypasses_public_surface() {
    let mut string_table = StringTable::new();

    let entry_root = intern_path(&["entry-root"], &mut string_table);
    let utils_root = intern_path(&["lib", "utils-root"], &mut string_table);
    let utils_mod_file = intern_path(&["lib", "utils", "#mod.moth"], &mut string_table);

    let mut file_module_membership = FxHashMap::default();
    file_module_membership.insert(utils_mod_file.clone(), utils_root.clone());
    file_module_membership.insert(entry_root.clone(), entry_root.clone());

    let boundaries = vec![ModuleRootBoundary {
        import_prefix: intern_path(&["lib", "utils"], &mut string_table),
        module_root: utils_root.clone(),
        root_file: utils_mod_file.clone(),
    }];

    let module_root_public_exports = empty_exports_for_roots(&[utils_root]);

    let input = PublicExportResolutionInput {
        importer_file: &utils_mod_file,
        header_path: &intern_path(&["internal", "empty_values"], &mut string_table),
        source_package_public_exports: &FxHashMap::default(),
        file_package_membership: &FxHashMap::default(),
        module_root_public_exports: &module_root_public_exports,
        file_module_membership: &file_module_membership,
        module_root_boundaries: &boundaries,
        string_table: &string_table,
    };

    let result = resolve_public_export_boundary(&input);
    assert!(
        matches!(
            result,
            Some(PublicExportLookupResult::NotAPublicExportBoundary)
        ),
        "source-package nested module root importing its own file should bypass the public surface"
    );
}

/// An importer with no module root membership should still produce a sensible result
/// rather than panicking.
#[test]
fn importer_without_module_membership_uses_empty_prefix() {
    let mut string_table = StringTable::new();

    let child_root = intern_path(&["child-root"], &mut string_table);
    let child_mod_file = intern_path(&["child", "#mod.moth"], &mut string_table);
    let unknown_importer = intern_path(&["unknown.moth"], &mut string_table);

    let boundaries = vec![ModuleRootBoundary {
        import_prefix: intern_path(&["child"], &mut string_table),
        module_root: child_root.clone(),
        root_file: child_mod_file.clone(),
    }];

    let module_root_public_exports = empty_exports_for_roots(&[child_root]);

    let input = PublicExportResolutionInput {
        importer_file: &unknown_importer,
        header_path: &intern_path(&["child", "greet"], &mut string_table),
        source_package_public_exports: &FxHashMap::default(),
        file_package_membership: &FxHashMap::default(),
        module_root_public_exports: &module_root_public_exports,
        file_module_membership: &FxHashMap::default(),
        module_root_boundaries: &boundaries,
        string_table: &string_table,
    };

    let result = resolve_public_export_boundary(&input);
    assert!(
        matches!(result, Some(PublicExportLookupResult::NotExported { .. })),
        "unknown importer targeting a child module should see NotExported for a missing symbol"
    );
}
