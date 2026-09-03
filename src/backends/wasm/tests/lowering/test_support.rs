use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowDropSite, BorrowDropSiteKind, BorrowFacts,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(crate) use crate::compiler_frontend::tests::hir_fixture_support::{
    bool_expression, expression, int_expression, local, statement, string_expression,
    unit_expression,
};

#[derive(Clone, Copy)]
pub(crate) struct TypeIds {
    pub unit: TypeId,
    pub int: TypeId,
    pub boolean: TypeId,
    pub string: TypeId,
}

/// Registers the type surface this backend's tests exercise.
///
/// Deliberately local to each backend: the JS lane registers option, choice, collection, map,
/// fallible-carrier and IO-input-handle types that the Wasm lane does not support, so the two
/// `TypeIds` shapes are not equivalent and must not be merged.
pub(crate) fn build_type_environment() -> (TypeEnvironment, TypeIds) {
    let env = TypeEnvironment::new();
    let builtins = env.builtins();

    let unit = builtins.none;
    let int = builtins.int;
    let boolean = builtins.bool;
    let string = builtins.string;

    (
        env,
        TypeIds {
            unit,
            int,
            boolean,
            string,
        },
    )
}

/// Assembles a module from this backend's fixture shape.
///
/// Deliberately local to each backend: the two signatures and their naming, region and choice
/// seeding differ, so this is not the same operation under one name. Only the HIR node
/// constructors are shared, from `compiler_frontend::tests::hir_fixture_support`.
pub(crate) fn build_module(
    string_table: &mut StringTable,
    functions: Vec<(HirFunction, InternedPath, HirFunctionOrigin)>,
    blocks: Vec<HirBlock>,
    start_function: FunctionId,
) -> HirModule {
    let mut module = HirModule::new();
    module.functions = functions
        .iter()
        .map(|(function, _, _)| function.clone())
        .collect();
    module.blocks = blocks;
    module.start_function = Some(start_function);
    for function in &module.functions {
        module
            .function_provenance
            .insert(function.id, Default::default());
    }

    let mut max_region_id = 0u32;
    for block in &module.blocks {
        max_region_id = max_region_id.max(block.region.0);
    }

    module.regions = (0..=max_region_id)
        .map(|region_id| {
            let parent = (region_id != 0).then_some(RegionId(0));
            HirRegion::lexical(RegionId(region_id), parent)
        })
        .collect();

    for (function, path, origin) in functions {
        module.side_table.bind_function_name(function.id, path);
        module.function_origins.insert(function.id, origin);
    }

    for block in &module.blocks {
        for local in &block.locals {
            let local_path =
                InternedPath::from_single_str(&format!("local_{}", local.id.0), string_table);
            module.side_table.bind_local_name(local.id, local_path);
        }
    }

    module
}

pub(crate) fn default_borrow_facts() -> BorrowFacts {
    BorrowFacts::default()
}

pub(crate) fn borrow_facts_with_drop_site(
    block: BlockId,
    kind: BorrowDropSiteKind,
    locals: Vec<LocalId>,
) -> BorrowFacts {
    let mut facts = BorrowFacts::default();
    facts
        .advisory_drop_sites
        .insert(block, vec![BorrowDropSite { kind, locals }]);
    facts
}

pub(crate) fn load_local(
    id: u32,
    local_id: LocalId,
    ty: TypeId,
    region: RegionId,
) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Load(HirPlace::Local(local_id)),
        ty,
        region,
        ValueKind::Place,
    )
}
