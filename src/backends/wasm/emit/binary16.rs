//! Portable core-Wasm helpers for binary16 representation boundaries.
//!
//! F16 values travel through Wasm as exact F32 carriers and use these helpers only when a semantic
//! F16 boundary or its compact memory representation requires conversion. Integer significand
//! rounding keeps ties-to-even, subnormals and signed zero independent of host numeric extensions.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

const F32_INPUT: u32 = 0;
const F32_BITS: u32 = 1;
const F32_EXPONENT: u32 = 2;
const F16_SIGN: u32 = 3;
const SIGNIFICAND: u32 = 4;
const SHIFT: u32 = 5;
const QUOTIENT: u32 = 6;
const REMAINDER: u32 = 7;
const HALFWAY: u32 = 8;
const F16_EXPONENT: u32 = 9;

const F16_SIGN_MASK: i32 = 0x8000;
const F32_FRACTION_MASK: i32 = 0x007f_ffff;
const F32_IMPLICIT_BIT: i32 = 0x0080_0000;
const F16_IMPLICIT_BIT: i32 = 0x0400;
const F16_FRACTION_MASK: i32 = 0x03ff;

/// Convert one finite F32 carrier directly to a finite binary16 bit pattern.
pub(crate) fn emit_f32_to_f16_bits() -> Function {
    let mut function = Function::new(vec![(9, ValType::I32)]);

    function.instruction(&Instruction::LocalGet(F32_INPUT));
    function.instruction(&Instruction::I32ReinterpretF32);
    function.instruction(&Instruction::LocalTee(F32_BITS));
    function.instruction(&Instruction::I32Const(23));
    function.instruction(&Instruction::I32ShrU);
    function.instruction(&Instruction::I32Const(0xff));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(F32_EXPONENT));

    function.instruction(&Instruction::LocalGet(F32_BITS));
    function.instruction(&Instruction::I32Const(16));
    function.instruction(&Instruction::I32ShrU);
    function.instruction(&Instruction::I32Const(F16_SIGN_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(F16_SIGN));

    // Non-finite inputs and finite values rounding above 65504 are invalid semantic F16 values.
    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Const(0xff));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Const(142));
    function.instruction(&Instruction::I32GtU);
    function.instruction(&Instruction::I32Or);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);

    // Values below the halfway point to the least subnormal round to signed zero.
    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Const(102));
    function.instruction(&Instruction::I32LtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(F16_SIGN));
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Const(113));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::If(BlockType::Empty));

    emit_f32_significand(&mut function);
    function.instruction(&Instruction::I32Const(13));
    function.instruction(&Instruction::LocalSet(SHIFT));
    emit_round_to_nearest_even(&mut function);

    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Const(112));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(F16_EXPONENT));

    // A rounded 2^11 significand carries into the exponent; only the carry beyond exponent 30 is
    // non-finite, which occurs at and above the 65520 ties-to-infinity boundary.
    function.instruction(&Instruction::LocalGet(QUOTIENT));
    function.instruction(&Instruction::I32Const(2048));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::I32Const(1024));
    function.instruction(&Instruction::LocalSet(QUOTIENT));
    function.instruction(&Instruction::LocalGet(F16_EXPONENT));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(F16_EXPONENT));
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(F16_EXPONENT));
    function.instruction(&Instruction::I32Const(31));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(F16_SIGN));
    function.instruction(&Instruction::LocalGet(F16_EXPONENT));
    function.instruction(&Instruction::I32Const(10));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Or);
    function.instruction(&Instruction::LocalGet(QUOTIENT));
    function.instruction(&Instruction::I32Const(1024));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::I32Or);
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    // Subnormal F16 values share a fixed 2^-24 unit. The shift ranges from 14 through 24 here.
    emit_f32_significand(&mut function);
    function.instruction(&Instruction::I32Const(126));
    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(SHIFT));
    emit_round_to_nearest_even(&mut function);

    function.instruction(&Instruction::LocalGet(F16_SIGN));
    function.instruction(&Instruction::LocalGet(QUOTIENT));
    function.instruction(&Instruction::I32Or);
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);
    function
}

/// Convert one finite binary16 bit pattern to its exact F32 carrier.
pub(crate) fn emit_f16_bits_to_f32() -> Function {
    const INPUT: u32 = 0;
    const SIGN: u32 = 1;
    const EXPONENT: u32 = 2;
    const SIGNIFICAND: u32 = 3;
    const F32_EXPONENT: u32 = 4;

    let mut function = Function::new(vec![(4, ValType::I32)]);

    function.instruction(&Instruction::LocalGet(INPUT));
    function.instruction(&Instruction::I32Const(F16_SIGN_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Const(16));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::LocalSet(SIGN));

    function.instruction(&Instruction::LocalGet(INPUT));
    function.instruction(&Instruction::I32Const(10));
    function.instruction(&Instruction::I32ShrU);
    function.instruction(&Instruction::I32Const(0x1f));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(EXPONENT));

    function.instruction(&Instruction::LocalGet(INPUT));
    function.instruction(&Instruction::I32Const(F16_FRACTION_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(SIGNIFICAND));

    // Corrupt storage must not manufacture a semantic infinity or NaN.
    function.instruction(&Instruction::LocalGet(EXPONENT));
    function.instruction(&Instruction::I32Const(31));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(EXPONENT));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(SIGNIFICAND));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(SIGN));
    function.instruction(&Instruction::F32ReinterpretI32);
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::I32Const(113));
    function.instruction(&Instruction::LocalSet(F32_EXPONENT));
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(SIGNIFICAND));
    function.instruction(&Instruction::I32Const(F16_IMPLICIT_BIT));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(SIGNIFICAND));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::LocalSet(SIGNIFICAND));
    function.instruction(&Instruction::LocalGet(F32_EXPONENT));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalSet(F32_EXPONENT));
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    emit_f32_bits_from_components(&mut function, SIGN, F32_EXPONENT, SIGNIFICAND);
    function.instruction(&Instruction::F32ReinterpretI32);
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    // A normal half exponent differs from the F32 exponent bias by 112.
    function.instruction(&Instruction::LocalGet(EXPONENT));
    function.instruction(&Instruction::I32Const(112));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(F32_EXPONENT));
    emit_f32_bits_from_components(&mut function, SIGN, F32_EXPONENT, SIGNIFICAND);
    function.instruction(&Instruction::F32ReinterpretI32);
    function.instruction(&Instruction::Return);

    function.instruction(&Instruction::End);
    function
}

fn emit_f32_significand(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(F32_BITS));
    function.instruction(&Instruction::I32Const(F32_FRACTION_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Const(F32_IMPLICIT_BIT));
    function.instruction(&Instruction::I32Or);
    function.instruction(&Instruction::LocalSet(SIGNIFICAND));
}

fn emit_round_to_nearest_even(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(SIGNIFICAND));
    function.instruction(&Instruction::LocalGet(SHIFT));
    function.instruction(&Instruction::I32ShrU);
    function.instruction(&Instruction::LocalSet(QUOTIENT));

    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::LocalGet(SHIFT));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::LocalGet(SIGNIFICAND));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::LocalSet(REMAINDER));

    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::LocalGet(SHIFT));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Sub);
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::LocalSet(HALFWAY));

    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::LocalGet(HALFWAY));
    function.instruction(&Instruction::I32GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    increment_quotient(function);
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::LocalGet(HALFWAY));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::LocalGet(QUOTIENT));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::If(BlockType::Empty));
    increment_quotient(function);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
}

fn increment_quotient(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(QUOTIENT));
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalSet(QUOTIENT));
}

fn emit_f32_bits_from_components(
    function: &mut Function,
    sign: u32,
    exponent: u32,
    significand: u32,
) {
    function.instruction(&Instruction::LocalGet(sign));
    function.instruction(&Instruction::LocalGet(exponent));
    function.instruction(&Instruction::I32Const(23));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Or);
    function.instruction(&Instruction::LocalGet(significand));
    function.instruction(&Instruction::I32Const(F16_FRACTION_MASK));
    function.instruction(&Instruction::I32And);
    function.instruction(&Instruction::I32Const(13));
    function.instruction(&Instruction::I32Shl);
    function.instruction(&Instruction::I32Or);
}
