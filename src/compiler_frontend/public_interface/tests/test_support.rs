//! Shared test fixtures for the focused public-interface test modules.
//!
//! WHAT: owns the construction helpers used by two or more focused public-interface test
//! modules: stable module/trait/struct origins, export bindings, interned paths, synthetic
//! generic-parameter registration, trait-root construction, struct registration, nominal and
//! trait origin index maps, constant/free-function/choice origins, struct roots, receiver
//! entries, default source locations and the immutable value mode.
//! WHY: these fixtures are genuinely shared across the trait-projection, direct-projection,
//! evidence-projection, local-finalization, declaration-record and folded-value test owners.
//! One-owner fixtures remain in their owning module; only cross-owner helpers live here.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::{
    ReceiverMethodEntry, ResolvedPublicTraitRoot, ResolvedPublicTypeRoot,
    ResolvedPublicTypeRootKind, ResolvedTraitRequirementFact,
};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterListId, NominalTypeId, TypeId};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginConstantId, OriginDeclarationId, OriginFunctionId,
    OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::FxHashMap;

pub(crate) fn module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    )
}

pub(crate) fn trait_origin(name: &str) -> OriginTraitId {
    OriginTraitId::new(module_origin(), name.to_owned())
}

pub(crate) fn struct_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(module_origin(), name.to_owned(), OriginTypeCategory::Struct)
}

pub(crate) fn trait_binding(name: &str) -> ExportBinding {
    ExportBinding::new(
        module_origin(),
        name.to_owned(),
        OriginDeclarationId::Trait(trait_origin(name)),
    )
}

pub(crate) fn path(name: &str, string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str(name, string_table)
}

pub(crate) fn this_type(env: &mut TypeEnvironment, string_table: &mut StringTable) -> TypeId {
    env.register_synthetic_generic_parameter(string_table.intern("This"))
}

pub(crate) fn trait_root(
    name: &str,
    this_type: TypeId,
    requirements: Vec<ResolvedTraitRequirementFact>,
    string_table: &mut StringTable,
) -> ResolvedPublicTraitRoot {
    ResolvedPublicTraitRoot {
        canonical_path: path(name, string_table),
        this_type,
        requirements,
        incompatible_trait_ids: Vec::new(),
    }
}

pub(crate) fn empty_fields() -> Box<[FieldDefinition]> {
    Box::new([])
}

pub(crate) fn register_struct(
    env: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
    fields: Box<[FieldDefinition]>,
    generic_parameters: Option<GenericParameterListId>,
) -> (NominalTypeId, TypeId) {
    let path = InternedPath::from_single_str(name, string_table);
    env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields,
        generic_parameters,
        const_record: false,
    })
}

pub(crate) fn nominal_origins_map(
    entries: Vec<(&str, OriginTypeId)>,
    string_table: &mut StringTable,
) -> FxHashMap<InternedPath, OriginTypeId> {
    let mut map = FxHashMap::default();
    for (name, origin) in entries {
        map.insert(path(name, string_table), origin);
    }
    map
}

pub(crate) fn trait_origins_map(
    entries: Vec<(&str, OriginTraitId)>,
    string_table: &mut StringTable,
) -> FxHashMap<InternedPath, OriginTraitId> {
    let mut map = FxHashMap::default();
    for (name, origin) in entries {
        map.insert(path(name, string_table), origin);
    }
    map
}

pub(crate) fn constant_origin(name: &str) -> OriginConstantId {
    OriginConstantId::new(module_origin(), name.to_owned())
}

pub(crate) fn free_function_origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module_origin(), name.to_owned())
}

pub(crate) fn choice_origin(name: &str) -> OriginTypeId {
    OriginTypeId::new(module_origin(), name.to_owned(), OriginTypeCategory::Choice)
}

pub(crate) fn struct_root(
    name: &str,
    type_id: TypeId,
    fields: Vec<Declaration>,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: path(name, string_table),
        kind: ResolvedPublicTypeRootKind::Struct { type_id, fields },
    }
}

pub(crate) fn receiver_entry(
    function_path: InternedPath,
    receiver: ReceiverKey,
    signature: FunctionSignature,
) -> ReceiverMethodEntry {
    ReceiverMethodEntry {
        function_path,
        receiver,
        source_file: InternedPath::new(),
        receiver_mutable: false,
        signature,
    }
}

pub(crate) fn default_location() -> SourceLocation {
    SourceLocation::default()
}

pub(crate) fn immutable() -> ValueMode {
    ValueMode::ImmutableOwned
}

// ---------------------------------------------------------------------------
