//! Hidden-invariant tests for consumer-local imported nominal projection.

use super::environment::builder::imported_nominal_path;
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::keywords::is_valid_identifier;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn imported_nominal_paths_preserve_package_origin_and_root_role() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut scratch = Vec::new();
    let project_normal = origin(
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "shared"),
        ModuleRootRole::Normal,
    );
    let project_support = origin(
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "shared"),
        ModuleRootRole::Support,
    );
    let builder_normal = origin(
        StablePackageIdentity::source_package(PackageOrigin::Builder, "shared"),
        ModuleRootRole::Normal,
    );

    let normal_path =
        imported_nominal_path(&project_normal, &mut string_table, &mut path_fork);
    let support_path =
        imported_nominal_path(&project_support, &mut string_table, &mut path_fork);
    let builder_path =
        imported_nominal_path(&builder_normal, &mut string_table, &mut path_fork);

    assert_eq!(
        path_fork.render_portable(normal_path, &string_table, &mut scratch),
        "<imported>/project/shared/normal/cards/Card"
    );
    assert_eq!(
        path_fork.render_portable(support_path, &string_table, &mut scratch),
        "<imported>/project/shared/support/cards/Card"
    );
    assert_eq!(
        path_fork.render_portable(builder_path, &string_table, &mut scratch),
        "<imported>/builder/shared/normal/cards/Card"
    );

    let mut formerly_colliding_authored_path =
        path_fork
            .try_intern_portable_path("__imported", &mut string_table)
            .expect("test path fits");
    for component in ["project", "shared", "normal", "cards", "Card"] {
        formerly_colliding_authored_path = path_fork
            .try_intern_child(formerly_colliding_authored_path, string_table.intern(component))
            .expect("test path fits");
    }
    assert_ne!(normal_path, formerly_colliding_authored_path);
    assert!(!is_valid_identifier("<imported>"));

    let mut type_environment = TypeEnvironment::new();
    let normal_type_id = register_struct(&mut type_environment, normal_path);
    let support_type_id = register_struct(&mut type_environment, support_path);
    let builder_type_id = register_struct(&mut type_environment, builder_path);
    for (origin, type_id) in [
        (project_normal, normal_type_id),
        (project_support, support_type_id),
        (builder_normal, builder_type_id),
    ] {
        let identity = CanonicalTypeIdentity::SourceNominal(origin);
        type_environment
            .register_canonical_identity(identity.clone(), type_id)
            .expect("distinct stable origins should register independently");
        assert_eq!(
            type_environment.canonical_identity_for_type_id(type_id),
            Some(&identity)
        );
    }
}


fn origin(package: StablePackageIdentity, role: ModuleRootRole) -> OriginTypeId {
    OriginTypeId::new(
        StableModuleOriginIdentity::from_portable_path(package, "cards".to_owned(), role),
        "Card".to_owned(),
        OriginTypeCategory::Struct,
    )
}

fn register_struct(
    type_environment: &mut TypeEnvironment,
    path: PathId,
) -> crate::compiler_frontend::datatypes::ids::TypeId {
    type_environment
        .register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path,
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        })
        .1
}
