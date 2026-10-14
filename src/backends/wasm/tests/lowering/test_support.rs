use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowDropSite, BorrowDropSiteKind, BorrowFacts,
};
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(crate) use crate::compiler_frontend::tests::hir_fixture_support::{
    bool_expression, expression, local, statement, string_expression,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

#[derive(Clone, Copy)]
pub(crate) struct TypeIds {
    pub unit: TypeId,
    pub boolean: TypeId,
    pub string: TypeId,
}

/// Registers the type surface this backend's tests exercise.
///
/// Deliberately local to each backend: the JS lane registers aggregate and IO-input-handle
/// types beyond this Wasm fixture's scalar surface, so the two `TypeIds` shapes are not
/// equivalent and must not be merged.
pub(crate) fn build_type_environment() -> (TypeEnvironment, TypeIds) {
    let env = TypeEnvironment::new();
    let builtins = env.builtins();

    let unit = builtins.none;
    let boolean = builtins.bool;
    let string = builtins.string;

    (
        env,
        TypeIds {
            unit,
            boolean,
            string,
        },
    )
}

/// Deterministic `TypeId` for one exact fixed scalar in this lane's fixture environment.
///
/// WHY: tests materialise exact scalars, so every fixture names its scalar rather than reading a
///      removed compilation-boundary width.
pub(crate) fn fixed_scalar_type(scalar: FixedScalar) -> TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

/// Materialise one exact fixed-scalar constant whose semantic type is its own scalar.
///
/// WHY: deriving the type from the value keeps the payload width and the expression type in
///      agreement by construction.
pub(crate) fn fixed_scalar_expression(
    value: FixedScalarValue,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    expression(
        HirExpressionKind::FixedScalar(value),
        fixed_scalar_type(value.scalar()),
        region,
        ValueKind::Const,
        expressions,
    )
}

/// Materialise one `I32` constant, the default exact integer of the scalar fixtures.
pub(crate) fn i32_expression(
    value: i32,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    fixed_scalar_expression(
        FixedScalarValue::signed(FixedScalar::I32, i64::from(value)).expect("i32 test value fits"),
        region,
        expressions,
    )
}

/// Assembles a module from this backend's fixture shape.
///
/// Deliberately local to each backend: the two signatures and their naming, region and choice
/// seeding differ, so this is not the same operation under one name. Only the HIR node
/// constructors are shared, from `compiler_frontend::tests::hir_fixture_support`.
pub(crate) fn build_module(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    expressions: HirExpressionStore,
    functions: Vec<(HirFunction, PathId, HirFunctionOrigin)>,
    blocks: Vec<HirBlock>,
    start_function: FunctionId,
) -> HirModule {
    let mut module = HirModule::new();
    module.expressions = expressions;
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
            let local_path = path_fork
                .try_intern_portable_path(&format!("local_{}", local.id.0), string_table)
                .expect("test path fits");
            module.side_table.bind_local_name(local.id, local_path);
        }
    }

    module.expressions.freeze();
    module
}

pub(crate) fn default_borrow_facts() -> BorrowFacts {
    BorrowFacts::default()
}

/// Empty proof table: every statement retains its full runtime check contract.
///
/// WHY: existing lowering tests exercise the retained-check behavior, so they pass the contract
///       `NumericProofs::default()` (empty = retain all checks) before publication analysis.
pub(crate) fn default_numeric_proofs() -> NumericProofs {
    NumericProofs::default()
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
    expressions: &mut HirExpressionStore,
    local_id: LocalId,
    ty: TypeId,
    region: RegionId,
) -> HirValueId {
    expression(
        HirExpressionKind::Load(HirPlace::local(local_id)),
        ty,
        region,
        ValueKind::Place,
        expressions,
    )
}
