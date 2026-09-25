//! Numeric profile gate tests for JavaScript lowering.
//!
//! WHAT: pins the temporary JS backend gate that rejects every non-standard boundary
//!       `NumericProfile` at the lowering entry while the standard profile keeps lowering.
//! WHY: fixed-width `Int`/`Float` lowering arrives with the numeric plan's Phase 4, which removes
//!       this check; until then a non-standard profile must fail at entry instead of lowering
//!       numbers under the default widths.

use super::support::*;
use crate::compiler_frontend::compiler_messages::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::numeric_profile::{FloatPrecision, IntWidth};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::terminators::HirTerminator;

#[test]
fn rejects_every_non_standard_numeric_profile_and_lowers_standard() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(0, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "entry_start",
        vec![block],
        function,
        &[],
    );

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("the standard numeric profile should still lower");

    // Every profile except Int32/Float64 must fail, so a gate that only compared one
    // dimension cannot lower the remaining non-standard combinations under default widths.
    for profile in [
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
    ] {
        let error = lower_hir_to_js(
            &module,
            &BorrowCheckReport::default(),
            &string_table,
            JsLoweringConfig::direct_js(false, profile),
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .expect_err("a non-standard numeric profile must be rejected at the JS lowering entry");

        assert_eq!(
            error.error_type,
            ErrorType::Compiler,
            "{profile} must be rejected as an internal lowering-gate failure"
        );
    }
}
