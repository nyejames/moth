//! Self-contained Ryu shortest-decimal helpers for generated core Wasm.
//!
//! The integer conversion follows the locked `ryu` 1.0.23 `f2s`/`d2s` implementation
//! (upstream Ulf Adams Ryu, Copyright 2018, Apache-2.0 OR BSL-1.0). Its proof and
//! tie-to-even interval rules are retained; only the Wasm instruction encoding differs.
//! The compact seed tables and exact split-table expansion below are adapted from
//! `ryu` 1.0.23 `d2s_small_table.rs` under the same dual license.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::runtime::strings::WasmRuntimeHelper;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

const INV_COUNT: usize = 292;
const POW_COUNT: usize = 326;
const INV_TABLE_BYTES: u32 = INV_COUNT as u32 * 16;
const TABLE_BYTES: usize = (INV_COUNT + POW_COUNT) * 16;
const FRACTION64: u64 = 0x000f_ffff_ffff_ffff;
const IMPLICIT64: u64 = 0x0010_0000_0000_0000;
const EXPONENT64: u64 = 0x7ff0_0000_0000_0000;
const ABS64: u64 = 0x7fff_ffff_ffff_ffff;
const MASK32: u64 = 0xffff_ffff;

const INV_SPLIT2: [(u64, u64); 15] = [
    (1, 2305843009213693952),
    (5955668970331000884, 1784059615882449851),
    (8982663654677661702, 1380349269358112757),
    (7286864317269821294, 2135987035920910082),
    (7005857020398200553, 1652639921975621497),
    (17965325103354776697, 1278668206209430417),
    (8928596168509315048, 1978643211784836272),
    (10075671573058298858, 1530901034580419511),
    (597001226353042382, 1184477304306571148),
    (1527430471115325346, 1832889850782397517),
    (12533209867169019542, 1418129833677084982),
    (5577825024675947042, 2194449627517475473),
    (11006974540203867551, 1697873161311732311),
    (10313493231639821582, 1313665730009899186),
    (12701016819766672773, 2032799256770390445),
];

const POW_SPLIT2: [(u64, u64); 13] = [
    (0, 1152921504606846976),
    (0, 1490116119384765625),
    (1032610780636961552, 1925929944387235853),
    (7910200175544436838, 1244603055572228341),
    (16941905809032713930, 1608611746708759036),
    (13024893955298202172, 2079081953128979843),
    (6607496772837067824, 1343575221513417750),
    (17332926989895652603, 1736530273035216783),
    (13037379183483547984, 2244412773384604712),
    (1605989338741628675, 1450417759929778918),
    (9630225068416591280, 1874621017369538693),
    (665883850346957067, 1211445438634777304),
    (14931890668723713708, 1565756531257009982),
];

const POW5_SMALL: [u64; 26] = [
    1,
    5,
    25,
    125,
    625,
    3125,
    15625,
    78125,
    390625,
    1953125,
    9765625,
    48828125,
    244140625,
    1220703125,
    6103515625,
    30517578125,
    152587890625,
    762939453125,
    3814697265625,
    19073486328125,
    95367431640625,
    476837158203125,
    2384185791015625,
    11920928955078125,
    59604644775390625,
    298023223876953125,
];

const POW5_OFFSETS: [u32; 21] = [
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x40000000, 0x59695995, 0x55545555, 0x56555515,
    0x41150504, 0x40555410, 0x44555145, 0x44504540, 0x45555550, 0x40004000, 0x96440440, 0x55565565,
    0x54454045, 0x40154151, 0x55559155, 0x51405555, 0x00000105,
];

const POW5_INV_OFFSETS: [u32; 19] = [
    0x54544554, 0x04055545, 0x10041000, 0x00400414, 0x40010000, 0x41155555, 0x00000454, 0x00010044,
    0x40000000, 0x44000041, 0x50454450, 0x55550054, 0x51655554, 0x40004000, 0x01000001, 0x00010500,
    0x51515411, 0x05555554, 0x00000000,
];

fn pow5_bits(exponent: i32) -> i32 {
    ((exponent as u32 * 1_217_359) >> 19) as i32 + 1
}

fn expand_pow5(index: u32) -> (u64, u64) {
    let base = index / POW5_SMALL.len() as u32;
    let base_index = base * POW5_SMALL.len() as u32;
    let offset = (index - base_index) as usize;
    let (low, high) = POW_SPLIT2[base as usize];
    if offset == 0 {
        return (low, high);
    }
    let small = POW5_SMALL[offset] as u128;
    let delta = (pow5_bits(index as i32) - pow5_bits(base_index as i32)) as u32;
    let low_product = small * low as u128;
    let high_product = small * high as u128;
    let correction = (POW5_OFFSETS[(index / 16) as usize] >> ((index % 16) * 2)) & 3;
    let split = (low_product >> delta) + (high_product << (64 - delta)) + correction as u128;
    (split as u64, (split >> 64) as u64)
}

fn expand_inv_pow5(index: u32) -> (u64, u64) {
    let base = index.div_ceil(POW5_SMALL.len() as u32);
    let base_index = base * POW5_SMALL.len() as u32;
    let offset = (base_index - index) as usize;
    let (low, high) = INV_SPLIT2[base as usize];
    if offset == 0 {
        return (low, high);
    }
    let small = POW5_SMALL[offset] as u128;
    let delta = (pow5_bits(base_index as i32) - pow5_bits(index as i32)) as u32;
    let low_product = small * (low - 1) as u128;
    let high_product = small * high as u128;
    let correction = (POW5_INV_OFFSETS[(index / 16) as usize] >> ((index % 16) * 2)) & 3;
    let split = (low_product >> delta) + (high_product << (64 - delta)) + 1 + correction as u128;
    (split as u64, (split >> 64) as u64)
}

/// Generate exactly the split multipliers used by locked `ryu` 1.0.23.
/// The inverse range is only extended through q=290, the maximum reachable from binary64.
pub(crate) fn ryu_table_data() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(TABLE_BYTES);
    for index in 0..INV_COUNT as u32 {
        let (low, high) = expand_inv_pow5(index);
        bytes.extend_from_slice(&low.to_le_bytes());
        bytes.extend_from_slice(&high.to_le_bytes());
    }
    for index in 0..POW_COUNT as u32 {
        let (low, high) = expand_pow5(index);
        bytes.extend_from_slice(&low.to_le_bytes());
        bytes.extend_from_slice(&high.to_le_bytes());
    }
    bytes
}

fn memarg(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

fn emit(function: &mut Function, instruction: Instruction<'_>) {
    function.instruction(&instruction);
}

fn set_i32(function: &mut Function, local: u32, value: i32) {
    emit(function, Instruction::I32Const(value));
    emit(function, Instruction::LocalSet(local));
}
fn set_i64(function: &mut Function, local: u32, value: i64) {
    emit(function, Instruction::I64Const(value));
    emit(function, Instruction::LocalSet(local));
}

fn emit_pow5_bits(function: &mut Function, exponent: u32) {
    emit(function, Instruction::LocalGet(exponent));
    emit(function, Instruction::I32Const(1_217_359));
    emit(function, Instruction::I32Mul);
    emit(function, Instruction::I32Const(19));
    emit(function, Instruction::I32ShrU);
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
}

fn emit_log10_pow2(function: &mut Function, exponent: u32) {
    emit(function, Instruction::LocalGet(exponent));
    emit(function, Instruction::I32Const(78_913));
    emit(function, Instruction::I32Mul);
    emit(function, Instruction::I32Const(18));
    emit(function, Instruction::I32ShrU);
}

fn emit_log10_pow5(function: &mut Function, exponent: u32) {
    emit(function, Instruction::LocalGet(exponent));
    emit(function, Instruction::I32Const(732_923));
    emit(function, Instruction::I32Mul);
    emit(function, Instruction::I32Const(20));
    emit(function, Instruction::I32ShrU);
}

// Local layout in FloatToDecimal (params 0..1, 30 i32 locals 2..31,
// 23 i64 locals 32..54, and two f64 locals 55..56).
const E_BITS: u32 = 2;
const E2: u32 = 3;
const E10: u32 = 4;
const Q: u32 = 5;
const K: u32 = 6;
const I: u32 = 7;
const J: u32 = 8;
const MM_SHIFT: u32 = 9;
const ACCEPT: u32 = 10;
const VM_TZ: u32 = 11;
const VR_TZ: u32 = 12;
const LAST: u32 = 13;
const REMOVED: u32 = 14;
const ROUND_UP: u32 = 15;
const MULT5_COUNT: u32 = 16;
const MULT5_RESULT: u32 = 17;
const TMP32: u32 = 18;
const F32_MANTISSA: u32 = 19;
const F32_EXPONENT: u32 = 20;
const F32_M2: u32 = 21;
const F32_MV: u32 = 22;
const F32_MM: u32 = 23;
const F32_MP: u32 = 24;
const F32_VR: u32 = 25;
const F32_VP: u32 = 26;
const F32_VM: u32 = 27;
const HALF_BITS: u32 = 28;
const HALF_DIGITS: u32 = 29;
const HALF_BASE: u32 = 30;
const HALF_CANDIDATE: u32 = 31;
const BITS: u32 = 32;
const M2: u32 = 33;
const MV: u32 = 34;
const VR: u32 = 35;
const VP: u32 = 36;
const VM: u32 = 37;
const MUL_LO: u32 = 38;
const MUL_HI: u32 = 39;
const MUL_M: u32 = 40;
const M_LO: u32 = 41;
const M_HI: u32 = 42;
const LO_LO: u32 = 43;
const LO_HI: u32 = 44;
const P00: u32 = 45;
const P01: u32 = 46;
const P10: u32 = 47;
const P11: u32 = 48;
const MID: u32 = 49;
const HIGH: u32 = 50;
const LOW: u32 = 51;
const SCRATCH64: u32 = 52;
const SCRATCH64_B: u32 = 53;
const SCRATCH64_C: u32 = 54;
const NORM: u32 = 55;
const CANDIDATE_F64: u32 = 56;
// The binary16 and Ryū branches are exclusive; reuse the i32 exponent scratch slot.
const CANDIDATE_EXP: u32 = Q;

fn table_offset(plan: &WasmEmitPlan) -> Result<u32, CompilerError> {
    plan.float_format_tables_offset.ok_or_else(|| {
        CompilerError::compiler_error("Wasm Float formatter is missing its planned Ryu tables")
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
    })
}

pub(crate) fn emit_float_to_decimal(plan: &WasmEmitPlan) -> Result<Function, CompilerError> {
    let _ = table_offset(plan)?;
    let mut function = Function::new(vec![
        (30, ValType::I32),
        (23, ValType::I64),
        (2, ValType::F64),
    ]);

    emit(&mut function, Instruction::LocalGet(1));
    emit(&mut function, Instruction::I32Const(16));
    emit(&mut function, Instruction::I32Eq);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit_half_decimal(&mut function);
    emit(&mut function, Instruction::Return);
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(1));
    emit(&mut function, Instruction::I32Const(32));
    emit(&mut function, Instruction::I32Eq);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit_ryu_f32(&mut function, table_offset(plan)?);
    emit(&mut function, Instruction::LocalGet(F32_MANTISSA));
    emit(&mut function, Instruction::I64ExtendI32U);
    emit(&mut function, Instruction::LocalGet(E10));
    emit(&mut function, Instruction::Return);
    emit(&mut function, Instruction::End);

    emit_ryu_f64(&mut function, table_offset(plan)?);
    emit(&mut function, Instruction::LocalGet(DECIMAL_RESULT));
    emit(&mut function, Instruction::LocalGet(E10));
    emit(&mut function, Instruction::Return);
    emit(&mut function, Instruction::End);
    Ok(function)
}
fn emit_load_multiplier(
    function: &mut Function,
    table_base: u32,
    index: u32,
    add_one_to_high: bool,
) {
    emit(function, Instruction::LocalGet(index));
    emit(function, Instruction::I32Const(16));
    emit(function, Instruction::I32Mul);
    emit(function, Instruction::I32Const(table_base as i32));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I64Load(memarg(0, 3)));
    emit(function, Instruction::LocalSet(MUL_LO));
    emit(function, Instruction::LocalGet(index));
    emit(function, Instruction::I32Const(16));
    emit(function, Instruction::I32Mul);
    emit(function, Instruction::I32Const(table_base as i32));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I64Load(memarg(8, 3)));
    emit(function, Instruction::LocalSet(MUL_HI));
    // Ryu's f32 inverse intrinsic uses the high split word plus one; f64 consumes the full pair.
    if add_one_to_high {
        emit(function, Instruction::LocalGet(MUL_HI));
        emit(function, Instruction::I64Const(1));
        emit(function, Instruction::I64Add);
        emit(function, Instruction::LocalSet(MUL_HI));
    }
}

fn emit_mul_shift_64(function: &mut Function, input: u32, shift: u32, output: u32) {
    emit(function, Instruction::LocalGet(input));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::LocalSet(M_LO));
    emit(function, Instruction::LocalGet(input));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalSet(M_HI));

    // high64(m * mul.low)
    emit(function, Instruction::LocalGet(MUL_LO));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::LocalSet(LO_LO));
    emit(function, Instruction::LocalGet(MUL_LO));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalSet(LO_HI));
    emit(function, Instruction::LocalGet(M_LO));
    emit(function, Instruction::LocalGet(LO_LO));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P00));
    emit(function, Instruction::LocalGet(M_LO));
    emit(function, Instruction::LocalGet(LO_HI));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P01));
    emit(function, Instruction::LocalGet(M_HI));
    emit(function, Instruction::LocalGet(LO_LO));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P10));
    emit(function, Instruction::LocalGet(M_HI));
    emit(function, Instruction::LocalGet(LO_HI));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P11));
    emit(function, Instruction::LocalGet(P00));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalGet(P01));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(P10));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(MID));
    emit(function, Instruction::LocalGet(P11));
    emit(function, Instruction::LocalGet(P01));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(P10));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(MID));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(HIGH));

    // Full 128-bit m * mul.high, then add high64(m * mul.low).
    emit(function, Instruction::LocalGet(MUL_HI));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::LocalSet(LO_LO));
    emit(function, Instruction::LocalGet(MUL_HI));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalSet(LO_HI));
    emit(function, Instruction::LocalGet(M_LO));
    emit(function, Instruction::LocalGet(LO_LO));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P00));
    emit(function, Instruction::LocalGet(M_LO));
    emit(function, Instruction::LocalGet(LO_HI));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P01));
    emit(function, Instruction::LocalGet(M_HI));
    emit(function, Instruction::LocalGet(LO_LO));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P10));
    emit(function, Instruction::LocalGet(M_HI));
    emit(function, Instruction::LocalGet(LO_HI));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P11));
    emit(function, Instruction::LocalGet(P00));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalGet(P01));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(P10));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(MID));
    emit(function, Instruction::LocalGet(P11));
    emit(function, Instruction::LocalGet(P01));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(P10));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(MID));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(SCRATCH64));
    emit(function, Instruction::LocalGet(P00));
    emit(function, Instruction::LocalGet(P01));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(P10));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(LOW));

    emit(function, Instruction::LocalGet(LOW));
    emit(function, Instruction::LocalGet(HIGH));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(SCRATCH64_B));
    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::LocalGet(LOW));
    emit(function, Instruction::I64LtU);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(SCRATCH64_C));

    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I32Const(64));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalGet(SCRATCH64_C));
    emit(function, Instruction::I64Const(64));
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I32Const(64));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64Or);
    emit(function, Instruction::LocalSet(output));
}

fn emit_mul_shift_32(function: &mut Function, input: u32, shift: u32, output: u32) {
    emit(function, Instruction::LocalGet(input));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalGet(MUL_HI));
    emit(function, Instruction::I64Const(MASK32 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P00));
    emit(function, Instruction::LocalGet(input));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalGet(MUL_HI));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(P01));
    emit(function, Instruction::LocalGet(P00));
    emit(function, Instruction::I64Const(32));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::LocalGet(P01));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I32Const(32));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(output));
}

fn emit_multiple_of_power5(
    function: &mut Function,
    value: u32,
    value_is_i64: bool,
    power: u32,
    result: u32,
) {
    emit(function, Instruction::LocalGet(value));
    if !value_is_i64 {
        emit(function, Instruction::I64ExtendI32U);
    }
    emit(function, Instruction::LocalSet(SCRATCH64));
    set_i32(function, MULT5_COUNT, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(MULT5_COUNT));
    emit(function, Instruction::LocalGet(power));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::I64Const(5));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::I64Const(5));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64));
    emit(function, Instruction::LocalGet(MULT5_COUNT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(MULT5_COUNT));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(MULT5_COUNT));
    emit(function, Instruction::LocalGet(power));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::LocalSet(result));
}

fn emit_multiple_of_power2_64(function: &mut Function, value: u32, power: u32, result: u32) {
    emit(function, Instruction::LocalGet(value));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::LocalGet(power));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::LocalSet(result));
}

fn emit_multiple_of_power2_32(function: &mut Function, value: u32, power: u32, result: u32) {
    emit(function, Instruction::LocalGet(value));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::LocalGet(power));
    emit(function, Instruction::I32Shl);
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32And);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalSet(result));
}
const DECIMAL_RESULT: u32 = SCRATCH64_C;

fn emit_ryu_f64(function: &mut Function, tables: u32) {
    emit(function, Instruction::LocalGet(0));
    emit(function, Instruction::I64ReinterpretF64);
    emit(function, Instruction::LocalSet(BITS));

    emit(function, Instruction::LocalGet(BITS));
    emit(function, Instruction::I64Const(52));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Const(0x7ff));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(E_BITS));

    emit(function, Instruction::LocalGet(BITS));
    emit(function, Instruction::I64Const(FRACTION64 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::LocalSet(M2));
    emit(function, Instruction::LocalGet(E_BITS));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, E2, -1076);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(E_BITS));
    emit(function, Instruction::I32Const(1077));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(E2));
    emit(function, Instruction::LocalGet(M2));
    emit(function, Instruction::I64Const(IMPLICIT64 as i64));
    emit(function, Instruction::I64Or);
    emit(function, Instruction::LocalSet(M2));
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(BITS));
    emit(function, Instruction::I64Const(FRACTION64 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(E_BITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::LocalSet(MM_SHIFT));
    emit(function, Instruction::LocalGet(M2));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::LocalSet(ACCEPT));
    emit(function, Instruction::LocalGet(M2));
    emit(function, Instruction::I64Const(4));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(MV));
    set_i32(function, VM_TZ, 0);
    set_i32(function, VR_TZ, 0);
    set_i32(function, LAST, 0);
    set_i32(function, REMOVED, 0);

    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    emit_log10_pow2(function, E2);
    emit(function, Instruction::LocalSet(Q));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Const(3));
    emit(function, Instruction::I32GtS);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(Q));
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::LocalSet(E10));

    emit_pow5_bits(function, Q);
    emit(function, Instruction::I32Const(124));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(K));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalGet(K));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(I));
    emit(function, Instruction::LocalGet(I));
    emit(function, Instruction::LocalSet(J));
    emit_load_multiplier(function, tables, Q, false);
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::LocalSet(MUL_M));
    emit_mul_shift_64(function, MUL_M, J, VR);
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(2));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(MUL_M));
    emit_mul_shift_64(function, MUL_M, J, VP);
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalGet(MM_SHIFT));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalSet(MUL_M));
    emit_mul_shift_64(function, MUL_M, J, VM);

    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(21));
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(5));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    emit_multiple_of_power5(function, MV, true, Q, MULT5_RESULT);
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::LocalSet(VR_TZ));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(ACCEPT));
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalGet(MM_SHIFT));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalSet(SCRATCH64_B));
    emit_multiple_of_power5(function, SCRATCH64_B, true, Q, MULT5_RESULT);
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::LocalSet(VM_TZ));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(2));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(SCRATCH64_B));
    emit_multiple_of_power5(function, SCRATCH64_B, true, Q, MULT5_RESULT);
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalSet(VP));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::Else);

    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(TMP32));
    emit_log10_pow5(function, TMP32);
    emit(function, Instruction::LocalSet(Q));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32GtS);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(Q));
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(E10));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(I));
    emit_pow5_bits(function, I);
    emit(function, Instruction::I32Const(125));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(K));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::LocalGet(K));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(J));
    emit_load_multiplier(function, tables + INV_TABLE_BYTES, I, false);

    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::LocalSet(MUL_M));
    emit_mul_shift_64(function, MUL_M, J, VR);
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(2));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(MUL_M));
    emit_mul_shift_64(function, MUL_M, J, VP);
    emit(function, Instruction::LocalGet(MV));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalGet(MM_SHIFT));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalSet(MUL_M));
    emit_mul_shift_64(function, MUL_M, J, VM);

    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, VR_TZ, 1);
    emit(function, Instruction::LocalGet(ACCEPT));
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(MM_SHIFT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::LocalSet(VM_TZ));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::LocalSet(VP));
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(63));
    emit(function, Instruction::I32LtU);
    emit(function, Instruction::If(BlockType::Empty));
    emit_multiple_of_power2_64(function, MV, Q, VR_TZ);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit_round_ryu_f64(function);
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(E10));
}

fn emit_round_ryu_f64(function: &mut Function) {
    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::I32Or);
    emit(function, Instruction::If(BlockType::Empty));

    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64_B));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::I64LeU);
    emit(function, Instruction::BrIf(1));

    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(VM_TZ));
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(VR_TZ));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(VR));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::LocalSet(VP));
    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::LocalSet(VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64_B));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64_C));
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(VR_TZ));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::LocalGet(SCRATCH64_C));
    emit(function, Instruction::LocalSet(VR));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::LocalSet(VP));
    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::LocalSet(VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Eqz);
    emit(function, Instruction::I32And);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, LAST, 4);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Eq);
    emit(function, Instruction::LocalGet(ACCEPT));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(DECIMAL_RESULT));

    emit(function, Instruction::Else);
    set_i32(function, ROUND_UP, 0);
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::I64Const(100));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(100));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::I64GtU);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(100));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I64Const(50));
    emit(function, Instruction::I64GeU);
    emit(function, Instruction::LocalSet(ROUND_UP));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(100));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(VR));
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::I64Const(100));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(VP));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(100));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(2));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::End);

    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(VP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(SCRATCH64_B));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::I64LeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I64Const(5));
    emit(function, Instruction::I64GeU);
    emit(function, Instruction::LocalSet(ROUND_UP));
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(VR));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::LocalSet(VP));
    emit(function, Instruction::LocalGet(SCRATCH64_B));
    emit(function, Instruction::LocalSet(VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::LocalGet(VM));
    emit(function, Instruction::I64Eq);
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Or);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalGet(VR));
    emit(function, Instruction::I64Add);
    emit(function, Instruction::LocalSet(DECIMAL_RESULT));
    emit(function, Instruction::End);
}
fn emit_ryu_f32(function: &mut Function, tables: u32) {
    emit(function, Instruction::LocalGet(0));
    emit(function, Instruction::F32DemoteF64);
    emit(function, Instruction::I32ReinterpretF32);
    emit(function, Instruction::LocalSet(TMP32));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(23));
    emit(function, Instruction::I32ShrU);
    emit(function, Instruction::I32Const(0xff));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(F32_EXPONENT));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(0x007f_ffff));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(F32_MANTISSA));
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(0xff));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::Unreachable);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, E2, -151);
    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::LocalSet(F32_M2));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(152));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(E2));
    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::I32Const(1 << 23));
    emit(function, Instruction::I32Or);
    emit(function, Instruction::LocalSet(F32_M2));
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::LocalSet(MM_SHIFT));
    emit(function, Instruction::LocalGet(F32_M2));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32And);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalSet(ACCEPT));
    emit(function, Instruction::LocalGet(F32_M2));
    emit(function, Instruction::I32Const(4));
    emit(function, Instruction::I32Mul);
    emit(function, Instruction::LocalSet(F32_MV));
    emit(function, Instruction::LocalGet(F32_MV));
    emit(function, Instruction::I32Const(2));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(F32_MP));
    emit(function, Instruction::LocalGet(F32_MV));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(MM_SHIFT));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(F32_MM));
    set_i32(function, VM_TZ, 0);
    set_i32(function, VR_TZ, 0);
    set_i32(function, LAST, 0);
    set_i32(function, REMOVED, 0);

    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    emit_log10_pow2(function, E2);
    emit(function, Instruction::LocalSet(Q));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::LocalSet(E10));
    emit_pow5_bits(function, Q);
    emit(function, Instruction::I32Const(60));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(K));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalGet(K));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(I));
    emit(function, Instruction::LocalGet(I));
    emit(function, Instruction::LocalSet(J));
    emit_load_multiplier(function, tables, Q, true);
    emit_mul_shift_32(function, F32_MV, J, F32_VR);
    emit_mul_shift_32(function, F32_MP, J, F32_VP);
    emit_mul_shift_32(function, F32_MM, J, F32_VM);

    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::I32And);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(TMP32));
    emit_pow5_bits(function, TMP32);
    emit(function, Instruction::I32Const(60));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(K));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(K));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(J));
    emit_load_multiplier(function, tables, TMP32, true);
    emit_mul_shift_32(function, F32_MV, J, TMP32);
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(9));
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(F32_MV));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    emit_multiple_of_power5(function, F32_MV, false, Q, MULT5_RESULT);
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::LocalSet(VR_TZ));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(ACCEPT));
    emit(function, Instruction::If(BlockType::Empty));
    emit_multiple_of_power5(function, F32_MM, false, Q, MULT5_RESULT);
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::LocalSet(VM_TZ));
    emit(function, Instruction::Else);
    emit_multiple_of_power5(function, F32_MP, false, Q, MULT5_RESULT);
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(F32_VP));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(TMP32));
    emit_log10_pow5(function, TMP32);
    emit(function, Instruction::LocalSet(Q));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::LocalGet(E2));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(E10));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(I));
    emit_pow5_bits(function, I);
    emit(function, Instruction::I32Const(61));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(K));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::LocalGet(K));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(J));
    emit_load_multiplier(function, tables + INV_TABLE_BYTES, I, false);
    emit_mul_shift_32(function, F32_MV, J, F32_VR);
    emit_mul_shift_32(function, F32_MP, J, F32_VP);
    emit_mul_shift_32(function, F32_MM, J, F32_VM);

    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::I32And);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(I));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(TMP32));
    emit_pow5_bits(function, TMP32);
    emit(function, Instruction::I32Const(61));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(K));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(K));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(J));
    emit_load_multiplier(function, tables + INV_TABLE_BYTES, TMP32, false);
    emit_mul_shift_32(function, F32_MV, J, TMP32);
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, VR_TZ, 1);
    emit(function, Instruction::LocalGet(ACCEPT));
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(MM_SHIFT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::LocalSet(VM_TZ));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(F32_VP));
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(31));
    emit(function, Instruction::I32LtU);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(Q));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(TMP32));
    emit_multiple_of_power2_32(function, F32_MV, TMP32, VR_TZ);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit_round_ryu_f32(function);
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(E10));
}

fn emit_round_ryu_f32(function: &mut Function) {
    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::I32Or);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(VM_TZ));
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(VR_TZ));
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VR));
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VP));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalSet(VR_TZ));
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VR));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(VR_TZ));
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32And);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32And);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, LAST, 4);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::LocalGet(ACCEPT));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::LocalGet(VM_TZ));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::I32And);
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(F32_MANTISSA));
    emit(function, Instruction::Else);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::I32LeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::LocalSet(LAST));
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VR));
    emit(function, Instruction::LocalGet(F32_VP));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VP));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(F32_VM));
    emit(function, Instruction::LocalGet(REMOVED));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(REMOVED));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::LocalGet(F32_VM));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::LocalGet(LAST));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::LocalGet(F32_VR));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(F32_MANTISSA));
    emit(function, Instruction::End);
}
fn emit_round_shift_even(function: &mut Function, significand: u32, shift: u32, output: u32) {
    emit(function, Instruction::LocalGet(significand));
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(output));
    emit(function, Instruction::LocalGet(significand));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::I64Sub);
    emit(function, Instruction::I64And);
    emit(function, Instruction::LocalSet(SCRATCH64));
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64GtU);
    emit(function, Instruction::LocalGet(SCRATCH64));
    emit(function, Instruction::I64Const(1));
    emit(function, Instruction::LocalGet(shift));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::I64Shl);
    emit(function, Instruction::I64Eq);
    emit(function, Instruction::LocalGet(output));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32And);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32And);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(output));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(output));
    emit(function, Instruction::End);
}

fn emit_f64_to_half_bits(function: &mut Function, value: u32, output: u32) {
    emit(function, Instruction::LocalGet(value));
    emit(function, Instruction::I64ReinterpretF64);
    emit(function, Instruction::LocalSet(BITS));
    emit(function, Instruction::LocalGet(BITS));
    emit(function, Instruction::I64Const(52));
    emit(function, Instruction::I64ShrU);
    emit(function, Instruction::I64Const(0x7ff));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(TMP32));

    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, output, 0);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(1038));
    emit(function, Instruction::I32GtU);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, output, 0x7c00);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(BITS));
    emit(function, Instruction::I64Const(FRACTION64 as i64));
    emit(function, Instruction::I64And);
    emit(function, Instruction::I64Const(IMPLICIT64 as i64));
    emit(function, Instruction::I64Or);
    emit(function, Instruction::LocalSet(M2));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(1023));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(F32_EXPONENT));
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(-14));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, F32_M2, 42);
    emit_round_shift_even(function, M2, F32_M2, F32_MANTISSA);
    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::I32Const(2048));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, F32_MANTISSA, 1024);
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(F32_EXPONENT));
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(15));
    emit(function, Instruction::I32GtS);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, output, 0x7c00);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(F32_EXPONENT));
    emit(function, Instruction::I32Const(15));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32Shl);
    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::I32Const(1024));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32Or);
    emit(function, Instruction::LocalSet(output));
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    emit(function, Instruction::I32Const(1051));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(F32_M2));
    emit(function, Instruction::LocalGet(F32_M2));
    emit(function, Instruction::I32Const(64));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, F32_MANTISSA, 0);
    emit(function, Instruction::Else);
    emit_round_shift_even(function, M2, F32_M2, F32_MANTISSA);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::I32Const(1024));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, output, 0x0400);
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(F32_MANTISSA));
    emit(function, Instruction::LocalSet(output));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
}

fn emit_half_candidate(function: &mut Function) {
    emit(function, Instruction::LocalGet(HALF_CANDIDATE));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::LocalGet(HALF_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(CANDIDATE_EXP));

    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(HALF_CANDIDATE));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(HALF_CANDIDATE));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(HALF_CANDIDATE));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(CANDIDATE_EXP));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(HALF_CANDIDATE));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::F64ConvertI64U);
    emit(function, Instruction::LocalSet(CANDIDATE_F64));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, ROUND_UP, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(CANDIDATE_F64));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Mul);
    emit(function, Instruction::LocalSet(CANDIDATE_F64));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(ROUND_UP));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    set_i32(function, ROUND_UP, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(CANDIDATE_F64));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Div);
    emit(function, Instruction::LocalSet(CANDIDATE_F64));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(ROUND_UP));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit_f64_to_half_bits(function, CANDIDATE_F64, TMP32);
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::LocalGet(HALF_BITS));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(HALF_CANDIDATE));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalSet(DECIMAL_RESULT));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::LocalSet(E10));
    emit(function, Instruction::LocalGet(DECIMAL_RESULT));
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::Return);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
}

/// Emit the bounded shortest-decimal search used by the Rust binary16 contract.
///
/// Scaling a binary16 value to its requested decimal digit position is exact: multiplying
/// by powers of ten adds at most 12 powers of five to an 11-bit significand, which fits in
/// binary64. Division is needed only for values at least 10, where binary16 spacing is at
/// least 2^-7; the nearest half-integer boundary is at least 1/(2 * 128 * 10^4) away,
/// much farther than binary64 rounding error. Exact ties remain exactly representable.
///
/// Candidate checks divide by 10^k only when the decimal exponent is -k. Their value is at
/// most 10^5/10^k, so binary64 rounding error is bounded by 10^5/(2^53 * 10^k), below the
/// minimum distance 1/(2^25 * 10^k) to a distinct binary16 midpoint. Positive exponents
/// produce exact integers. Exact ties are dyadic and exactly representable in binary64.
fn emit_half_decimal(function: &mut Function) {
    emit_f64_to_half_bits(function, 0, HALF_BITS);
    emit(function, Instruction::LocalGet(0));
    emit(function, Instruction::LocalSet(NORM));
    set_i32(function, E10, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(NORM));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Ge);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(NORM));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Div);
    emit(function, Instruction::LocalSet(NORM));
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(E10));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(NORM));
    emit(function, Instruction::F64Const(1.0_f64.into()));
    emit(function, Instruction::F64Ge);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(NORM));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Mul);
    emit(function, Instruction::LocalSet(NORM));
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(E10));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    set_i32(function, HALF_DIGITS, 1);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(HALF_DIGITS));
    emit(function, Instruction::I32Const(5));
    emit(function, Instruction::I32GtU);
    emit(function, Instruction::BrIf(1));

    emit(function, Instruction::LocalGet(HALF_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(TMP32));
    emit(function, Instruction::LocalGet(0));
    emit(function, Instruction::LocalSet(CANDIDATE_F64));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    set_i32(function, ROUND_UP, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(CANDIDATE_F64));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Mul);
    emit(function, Instruction::LocalSet(CANDIDATE_F64));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(ROUND_UP));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    set_i32(function, ROUND_UP, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(TMP32));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(CANDIDATE_F64));
    emit(function, Instruction::F64Const(10.0_f64.into()));
    emit(function, Instruction::F64Div);
    emit(function, Instruction::LocalSet(CANDIDATE_F64));
    emit(function, Instruction::LocalGet(ROUND_UP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(ROUND_UP));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(CANDIDATE_F64));
    emit(function, Instruction::F64Nearest);
    emit(function, Instruction::I64TruncF64U);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::LocalSet(HALF_BASE));

    set_i32(function, MULT5_RESULT, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::I32Const(3));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::LocalSet(HALF_CANDIDATE));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Eq);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(HALF_CANDIDATE));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(HALF_CANDIDATE));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit_half_candidate(function);
    emit(function, Instruction::LocalGet(MULT5_RESULT));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(MULT5_RESULT));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(HALF_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(HALF_DIGITS));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalSet(DECIMAL_RESULT));
    emit(function, Instruction::LocalGet(E10));
    emit(function, Instruction::I32Const(4));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(CANDIDATE_EXP));
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32RemU);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(HALF_BASE));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(CANDIDATE_EXP));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(HALF_BASE));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalSet(DECIMAL_RESULT));
    emit(function, Instruction::LocalGet(DECIMAL_RESULT));
    emit(function, Instruction::LocalGet(CANDIDATE_EXP));
    emit(function, Instruction::Return);
}
// StringFromFloat locals: params 0..1; i32 locals 2..12, i64 locals 13..15,
// and one f64 local at index 16.
const STRING_SIGN: u32 = 2;
const STRING_DIGITS: u32 = 3;
const STRING_EXP: u32 = 4;
const STRING_SCI_EXP: u32 = 5;
const STRING_STYLE: u32 = 6;
const STRING_CURSOR: u32 = 7;
const STRING_REGION: u32 = 8;
const STRING_HANDLE: u32 = 9;
const STRING_LENGTH: u32 = 10;
const STRING_EXP_DIGITS: u32 = 11;
const STRING_AUX: u32 = 12;
const STRING_BITS: u32 = 13;
const STRING_MANTISSA: u32 = 14;
const STRING_TEMP: u32 = 15;
const STRING_VALUE: u32 = 16;

pub(crate) fn emit_string_from_float(
    plan: &WasmEmitPlan,
    alloc_index: u32,
) -> Result<Function, CompilerError> {
    let decimal_index = plan
        .helper_indices
        .get(&WasmRuntimeHelper::FloatToDecimal)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error("Wasm Float formatter is missing rt_float_to_decimal")
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })?;
    let mut function = Function::new(vec![
        (11, ValType::I32),
        (3, ValType::I64),
        (1, ValType::F64),
    ]);

    emit(&mut function, Instruction::LocalGet(1));
    emit(&mut function, Instruction::I32Const(16));
    emit(&mut function, Instruction::I32Ne);
    emit(&mut function, Instruction::LocalGet(1));
    emit(&mut function, Instruction::I32Const(32));
    emit(&mut function, Instruction::I32Ne);
    emit(&mut function, Instruction::I32And);
    emit(&mut function, Instruction::LocalGet(1));
    emit(&mut function, Instruction::I32Const(64));
    emit(&mut function, Instruction::I32Ne);
    emit(&mut function, Instruction::I32And);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::Unreachable);
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(0));
    emit(&mut function, Instruction::I64ReinterpretF64);
    emit(&mut function, Instruction::LocalSet(STRING_BITS));
    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::I64Const(63));
    emit(&mut function, Instruction::I64ShrU);
    emit(&mut function, Instruction::I32WrapI64);
    emit(&mut function, Instruction::LocalSet(STRING_SIGN));
    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::I64Const(ABS64 as i64));
    emit(&mut function, Instruction::I64And);
    emit(&mut function, Instruction::LocalSet(STRING_BITS));
    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::F64ReinterpretI64);
    emit(&mut function, Instruction::LocalSet(STRING_VALUE));

    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::I64Const(EXPONENT64 as i64));
    emit(&mut function, Instruction::I64And);
    emit(&mut function, Instruction::I64Const(EXPONENT64 as i64));
    emit(&mut function, Instruction::I64Eq);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::Unreachable);
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::I64Eqz);
    emit(&mut function, Instruction::If(BlockType::Empty));
    set_i32(&mut function, STRING_SIGN, 0);
    emit(&mut function, Instruction::I64Const(0));
    emit(&mut function, Instruction::LocalSet(STRING_MANTISSA));
    set_i32(&mut function, STRING_EXP, 0);
    emit(&mut function, Instruction::Else);
    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::F64ReinterpretI64);
    emit(&mut function, Instruction::LocalSet(STRING_VALUE));
    emit(&mut function, Instruction::LocalGet(STRING_VALUE));
    emit(&mut function, Instruction::LocalGet(1));
    emit(&mut function, Instruction::Call(decimal_index));
    emit(&mut function, Instruction::LocalSet(STRING_EXP));
    emit(&mut function, Instruction::LocalSet(STRING_MANTISSA));
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(STRING_MANTISSA));
    emit(&mut function, Instruction::LocalSet(STRING_TEMP));
    set_i32(&mut function, STRING_DIGITS, 0);
    emit(&mut function, Instruction::Block(BlockType::Empty));
    emit(&mut function, Instruction::Loop(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_TEMP));
    emit(&mut function, Instruction::I64Eqz);
    emit(&mut function, Instruction::BrIf(1));
    emit(&mut function, Instruction::LocalGet(STRING_TEMP));
    emit(&mut function, Instruction::I64Const(10));
    emit(&mut function, Instruction::I64DivU);
    emit(&mut function, Instruction::LocalSet(STRING_TEMP));
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::I32Const(1));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_DIGITS));
    emit(&mut function, Instruction::Br(0));
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::I32Eqz);
    emit(&mut function, Instruction::If(BlockType::Empty));
    set_i32(&mut function, STRING_DIGITS, 1);
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(STRING_EXP));
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::I32Const(1));
    emit(&mut function, Instruction::I32Sub);
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_SCI_EXP));
    emit(&mut function, Instruction::LocalGet(STRING_VALUE));
    emit(&mut function, Instruction::F64Const(1.0e21_f64.into()));
    emit(&mut function, Instruction::F64Ge);
    emit(&mut function, Instruction::LocalGet(STRING_VALUE));
    emit(&mut function, Instruction::F64Const(1.0e-6_f64.into()));
    emit(&mut function, Instruction::F64Lt);
    emit(&mut function, Instruction::I32Or);
    emit(&mut function, Instruction::LocalSet(STRING_STYLE));
    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::I64Eqz);
    emit(&mut function, Instruction::If(BlockType::Empty));
    set_i32(&mut function, STRING_STYLE, 0);
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(STRING_STYLE));
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(&mut function, Instruction::I32Const(0));
    emit(&mut function, Instruction::I32GeS);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(&mut function, Instruction::LocalSet(STRING_CURSOR));
    emit(&mut function, Instruction::Else);
    emit(&mut function, Instruction::I32Const(0));
    emit(&mut function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(&mut function, Instruction::I32Sub);
    emit(&mut function, Instruction::LocalSet(STRING_CURSOR));
    emit(&mut function, Instruction::End);
    set_i32(&mut function, STRING_EXP_DIGITS, 1);
    emit(&mut function, Instruction::LocalGet(STRING_CURSOR));
    emit(&mut function, Instruction::I32Const(10));
    emit(&mut function, Instruction::I32GeU);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_CURSOR));
    emit(&mut function, Instruction::I32Const(10));
    emit(&mut function, Instruction::I32DivU);
    emit(&mut function, Instruction::LocalSet(STRING_CURSOR));
    emit(&mut function, Instruction::I32Const(2));
    emit(&mut function, Instruction::LocalSet(STRING_EXP_DIGITS));
    emit(&mut function, Instruction::LocalGet(STRING_CURSOR));
    emit(&mut function, Instruction::I32Const(10));
    emit(&mut function, Instruction::I32GeU);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::I32Const(3));
    emit(&mut function, Instruction::LocalSet(STRING_EXP_DIGITS));
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::I32Const(1));
    emit(&mut function, Instruction::I32GtU);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::I32Const(1));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_LENGTH));
    emit(&mut function, Instruction::Else);
    set_i32(&mut function, STRING_LENGTH, 1);
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::LocalGet(STRING_LENGTH));
    emit(&mut function, Instruction::I32Const(2));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalGet(STRING_SIGN));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_LENGTH));
    emit(&mut function, Instruction::Else);
    emit(&mut function, Instruction::LocalGet(STRING_EXP));
    emit(&mut function, Instruction::I32Const(0));
    emit(&mut function, Instruction::I32GeS);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::LocalGet(STRING_EXP));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_LENGTH));
    emit(&mut function, Instruction::Else);
    emit(&mut function, Instruction::I32Const(0));
    emit(&mut function, Instruction::LocalGet(STRING_EXP));
    emit(&mut function, Instruction::I32Sub);
    emit(&mut function, Instruction::LocalSet(STRING_CURSOR));
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::LocalGet(STRING_CURSOR));
    emit(&mut function, Instruction::I32GtU);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_DIGITS));
    emit(&mut function, Instruction::I32Const(1));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_LENGTH));
    emit(&mut function, Instruction::Else);
    emit(&mut function, Instruction::LocalGet(STRING_CURSOR));
    emit(&mut function, Instruction::I32Const(2));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_LENGTH));
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::LocalGet(STRING_LENGTH));
    emit(&mut function, Instruction::LocalGet(STRING_SIGN));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::LocalSet(STRING_LENGTH));
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(STRING_LENGTH));
    emit(&mut function, Instruction::Call(alloc_index));
    emit(&mut function, Instruction::LocalSet(STRING_REGION));
    emit(&mut function, Instruction::I32Const(8));
    emit(&mut function, Instruction::Call(alloc_index));
    emit(&mut function, Instruction::LocalSet(STRING_HANDLE));
    emit(&mut function, Instruction::LocalGet(STRING_HANDLE));
    emit(&mut function, Instruction::LocalGet(STRING_REGION));
    emit(&mut function, Instruction::I32Store(memarg(0, 2)));
    emit(&mut function, Instruction::LocalGet(STRING_HANDLE));
    emit(&mut function, Instruction::LocalGet(STRING_LENGTH));
    emit(&mut function, Instruction::I32Store(memarg(4, 2)));

    emit(&mut function, Instruction::LocalGet(STRING_SIGN));
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_REGION));
    emit(&mut function, Instruction::I32Const(b'-' as i32));
    emit(&mut function, Instruction::I32Store8(memarg(0, 0)));
    emit(&mut function, Instruction::End);
    emit(&mut function, Instruction::LocalGet(STRING_BITS));
    emit(&mut function, Instruction::I64Eqz);
    emit(&mut function, Instruction::If(BlockType::Empty));
    emit(&mut function, Instruction::LocalGet(STRING_REGION));
    emit(&mut function, Instruction::LocalGet(STRING_SIGN));
    emit(&mut function, Instruction::I32Add);
    emit(&mut function, Instruction::I32Const(b'0' as i32));
    emit(&mut function, Instruction::I32Store8(memarg(0, 0)));
    emit(&mut function, Instruction::Else);
    emit_render_float(&mut function);
    emit(&mut function, Instruction::End);

    emit(&mut function, Instruction::LocalGet(STRING_HANDLE));
    emit(&mut function, Instruction::Return);
    emit(&mut function, Instruction::End);
    Ok(function)
}

fn emit_render_float(function: &mut Function) {
    emit(function, Instruction::LocalGet(STRING_STYLE));
    emit(function, Instruction::If(BlockType::Empty));
    emit_render_scientific(function);
    emit(function, Instruction::Else);
    emit_render_fixed(function);
    emit(function, Instruction::End);
}

fn emit_write_digits_reverse(
    function: &mut Function,
    start_offset: u32,
    digit_count: u32,
    value: u32,
    decimal_point_after: Option<u32>,
) {
    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(start_offset));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalGet(digit_count));
    emit(function, Instruction::I32Add);
    if decimal_point_after.is_some() {
        emit(function, Instruction::I32Const(1));
        emit(function, Instruction::I32Add);
    }
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(value));
    emit(function, Instruction::LocalSet(STRING_TEMP));
    set_i32(function, STRING_EXP_DIGITS, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::LocalGet(digit_count));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::I32Const(b'0' as i32));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(STRING_TEMP));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_EXP_DIGITS));
    if let Some(fraction_digits) = decimal_point_after {
        emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
        emit(function, Instruction::LocalGet(fraction_digits));
        emit(function, Instruction::I32Eq);
        emit(function, Instruction::If(BlockType::Empty));
        emit(function, Instruction::LocalGet(STRING_CURSOR));
        emit(function, Instruction::I32Const(1));
        emit(function, Instruction::I32Sub);
        emit(function, Instruction::LocalTee(STRING_CURSOR));
        emit(function, Instruction::I32Const(b'.' as i32));
        emit(function, Instruction::I32Store8(memarg(0, 0)));
        emit(function, Instruction::End);
    }
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
}

fn emit_render_scientific(function: &mut Function) {
    set_i64(function, STRING_TEMP, 1);
    set_i32(function, STRING_EXP_DIGITS, 1);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::LocalGet(STRING_DIGITS));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64Mul);
    emit(function, Instruction::LocalSet(STRING_TEMP));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_EXP_DIGITS));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalGet(STRING_MANTISSA));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::I32Const(b'0' as i32));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Store8(memarg(0, 0)));

    emit(function, Instruction::LocalGet(STRING_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32GtU);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(b'.' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_MANTISSA));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::LocalSet(STRING_TEMP));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::I32Const(2));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_AUX));
    emit_write_digits_reverse(function, STRING_CURSOR, STRING_AUX, STRING_TEMP, None);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(function, Instruction::LocalSet(STRING_LENGTH));
    emit(function, Instruction::Else);
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_LENGTH));
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(STRING_LENGTH));
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    set_i32(function, STRING_EXP_DIGITS, 1);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::I32Eqz);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(10));
    emit(function, Instruction::I32DivU);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_EXP_DIGITS));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_HANDLE));
    emit(function, Instruction::I32Load(memarg(4, 2)));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_LENGTH));
    emit(function, Instruction::I64ExtendI32U);
    emit(function, Instruction::LocalSet(STRING_TEMP));
    set_i32(function, STRING_STYLE, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_STYLE));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64RemU);
    emit(function, Instruction::I32WrapI64);
    emit(function, Instruction::I32Const(b'0' as i32));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_TEMP));
    emit(function, Instruction::I64Const(10));
    emit(function, Instruction::I64DivU);
    emit(function, Instruction::LocalSet(STRING_TEMP));
    emit(function, Instruction::LocalGet(STRING_STYLE));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_STYLE));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);

    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_SCI_EXP));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(b'+' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(b'-' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::End);
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::I32Const(b'e' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
}

fn emit_render_fixed(function: &mut Function) {
    emit(function, Instruction::LocalGet(STRING_EXP));
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::I32GeS);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit_write_digits_reverse(
        function,
        STRING_CURSOR,
        STRING_DIGITS,
        STRING_MANTISSA,
        None,
    );
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::LocalGet(STRING_DIGITS));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_LENGTH));
    set_i32(function, STRING_EXP_DIGITS, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::LocalGet(STRING_EXP));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_LENGTH));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(b'0' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_EXP_DIGITS));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit(function, Instruction::Else);
    emit(function, Instruction::I32Const(0));
    emit(function, Instruction::LocalGet(STRING_EXP));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_LENGTH));
    emit(function, Instruction::LocalGet(STRING_DIGITS));
    emit(function, Instruction::LocalGet(STRING_LENGTH));
    emit(function, Instruction::I32GtU);
    emit(function, Instruction::If(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit_write_digits_reverse(
        function,
        STRING_CURSOR,
        STRING_DIGITS,
        STRING_MANTISSA,
        Some(STRING_LENGTH),
    );
    emit(function, Instruction::Else);
    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(b'0' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(b'.' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_LENGTH));
    emit(function, Instruction::LocalGet(STRING_DIGITS));
    emit(function, Instruction::I32Sub);
    emit(function, Instruction::LocalSet(STRING_EXP_DIGITS));
    emit(function, Instruction::LocalGet(STRING_SIGN));
    emit(function, Instruction::I32Const(2));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    set_i32(function, STRING_STYLE, 0);
    emit(function, Instruction::Block(BlockType::Empty));
    emit(function, Instruction::Loop(BlockType::Empty));
    emit(function, Instruction::LocalGet(STRING_STYLE));
    emit(function, Instruction::LocalGet(STRING_EXP_DIGITS));
    emit(function, Instruction::I32GeU);
    emit(function, Instruction::BrIf(1));
    emit(function, Instruction::LocalGet(STRING_REGION));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::I32Const(b'0' as i32));
    emit(function, Instruction::I32Store8(memarg(0, 0)));
    emit(function, Instruction::LocalGet(STRING_CURSOR));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_CURSOR));
    emit(function, Instruction::LocalGet(STRING_STYLE));
    emit(function, Instruction::I32Const(1));
    emit(function, Instruction::I32Add);
    emit(function, Instruction::LocalSet(STRING_STYLE));
    emit(function, Instruction::Br(0));
    emit(function, Instruction::End);
    emit(function, Instruction::End);
    emit_write_digits_reverse(
        function,
        STRING_CURSOR,
        STRING_DIGITS,
        STRING_MANTISSA,
        None,
    );
    emit(function, Instruction::End);
    emit(function, Instruction::End);
}
