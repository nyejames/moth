//! Pure binary64 remainder helper emitted as core Wasm instructions.
//!
//! The integer significand shift/subtract and result reconstruction follow
//! Sun fdlibm's `e_fmod.c`. Binary64 significands fit in one Wasm i64, avoiding
//! fdlibm's paired-word representation while preserving exact remainders.

/*
 * ====================================================
 * Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
 *
 * Developed at SunSoft, a Sun Microsystems, Inc. business.
 * Permission to use, copy, modify, and distribute this
 * software is freely granted, provided that this notice
 * is preserved.
 * ====================================================
 */

use wasm_encoder::{BlockType, Function, Instruction, ValType};

const SIGN_MASK: i64 = i64::MIN;
const ABS_MASK: i64 = 0x7fff_ffff_ffff_ffff;
const EXPONENT_MASK: i64 = 0x7ff0_0000_0000_0000;
const FRACTION_MASK: i64 = 0x000f_ffff_ffff_ffff;
const IMPLICIT_BIT: i64 = 0x0010_0000_0000_0000;
const INFINITY_BITS: i64 = 0x7ff0_0000_0000_0000;
const EXPONENT_SHIFT: i64 = 52;

// Parameters 0..1 are the inputs. The remaining locals are i64 bit fields,
// exponents, significands and loop state.
const LEFT_BITS: u32 = 2;
const RIGHT_BITS: u32 = 3;
const ABS_LEFT: u32 = 4;
const ABS_RIGHT: u32 = 5;
const RESULT_SIGN: u32 = 6;
const LEFT_EXPONENT: u32 = 7;
const RIGHT_EXPONENT: u32 = 8;
const LEFT_SIGNIFICAND: u32 = 9;
const RIGHT_SIGNIFICAND: u32 = 10;
const SCRATCH: u32 = 11;
const SHIFT: u32 = 12;
const REMAINDER: u32 = 13;
const RESULT_BITS: u32 = 14;

pub(super) fn emit_float_remainder() -> Function {
    let mut function = Function::new([(13, ValType::I64)]);

    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I64ReinterpretF64);
    function.instruction(&Instruction::LocalTee(LEFT_BITS));
    function.instruction(&Instruction::I64Const(ABS_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalSet(ABS_LEFT));

    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I64ReinterpretF64);
    function.instruction(&Instruction::LocalTee(RIGHT_BITS));
    function.instruction(&Instruction::I64Const(ABS_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalSet(ABS_RIGHT));

    function.instruction(&Instruction::LocalGet(LEFT_BITS));
    function.instruction(&Instruction::I64Const(SIGN_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalSet(RESULT_SIGN));

    // fmod is undefined for a zero divisor, NaN operands, or an infinite dividend.
    function.instruction(&Instruction::LocalGet(ABS_RIGHT));
    function.instruction(&Instruction::I64Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_nan_return(&mut function);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(ABS_LEFT));
    function.instruction(&Instruction::I64Const(INFINITY_BITS));
    function.instruction(&Instruction::I64GeU);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_nan_return(&mut function);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(ABS_RIGHT));
    function.instruction(&Instruction::I64Const(INFINITY_BITS));
    function.instruction(&Instruction::I64GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_nan_return(&mut function);
    function.instruction(&Instruction::End);

    // A finite dividend modulo infinity is the dividend itself.
    function.instruction(&Instruction::LocalGet(ABS_LEFT));
    function.instruction(&Instruction::LocalGet(ABS_RIGHT));
    function.instruction(&Instruction::I64LtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(ABS_LEFT));
    function.instruction(&Instruction::LocalGet(ABS_RIGHT));
    function.instruction(&Instruction::I64Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_zero_return(&mut function);
    function.instruction(&Instruction::End);

    emit_unpack_significand(
        &mut function,
        ABS_LEFT,
        LEFT_SIGNIFICAND,
        LEFT_EXPONENT,
        SCRATCH,
        SHIFT,
    );
    emit_unpack_significand(
        &mut function,
        ABS_RIGHT,
        RIGHT_SIGNIFICAND,
        RIGHT_EXPONENT,
        SCRATCH,
        SHIFT,
    );

    // Align the divisor significand to the dividend and subtract only when it fits.
    function.instruction(&Instruction::LocalGet(LEFT_EXPONENT));
    function.instruction(&Instruction::LocalGet(RIGHT_EXPONENT));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(SHIFT));
    function.instruction(&Instruction::LocalGet(LEFT_SIGNIFICAND));
    function.instruction(&Instruction::LocalSet(REMAINDER));

    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::LocalGet(RIGHT_SIGNIFICAND));
    function.instruction(&Instruction::I64GeU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::LocalGet(RIGHT_SIGNIFICAND));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(REMAINDER));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::I64Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    emit_signed_zero_return(&mut function);
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(SHIFT));
    function.instruction(&Instruction::I64Eqz);
    function.instruction(&Instruction::BrIf(1));
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::LocalSet(REMAINDER));
    function.instruction(&Instruction::LocalGet(SHIFT));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(SHIFT));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    // Normalize the exact integer remainder before restoring the divisor exponent.
    function.instruction(&Instruction::Block(BlockType::Empty));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::I64Const(IMPLICIT_BIT));
    function.instruction(&Instruction::I64GeU);
    function.instruction(&Instruction::BrIf(1));
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::LocalSet(REMAINDER));
    function.instruction(&Instruction::LocalGet(RIGHT_EXPONENT));
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(RIGHT_EXPONENT));
    function.instruction(&Instruction::Br(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);

    // Pack a normal result directly; subnormal packing discards only zero low bits.
    function.instruction(&Instruction::LocalGet(RIGHT_EXPONENT));
    function.instruction(&Instruction::I64Const(-1022));
    function.instruction(&Instruction::I64GeS);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(RIGHT_EXPONENT));
    function.instruction(&Instruction::I64Const(1023));
    function.instruction(&Instruction::I64Add);
    function.instruction(&Instruction::I64Const(EXPONENT_SHIFT));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::I64Const(FRACTION_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::LocalGet(RESULT_SIGN));
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::LocalSet(RESULT_BITS));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::I64Const(-1022));
    function.instruction(&Instruction::LocalGet(RIGHT_EXPONENT));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(SHIFT));
    function.instruction(&Instruction::LocalGet(REMAINDER));
    function.instruction(&Instruction::LocalGet(SHIFT));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::LocalGet(RESULT_SIGN));
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::LocalSet(RESULT_BITS));
    function.instruction(&Instruction::End);

    function.instruction(&Instruction::LocalGet(RESULT_BITS));
    function.instruction(&Instruction::F64ReinterpretI64);
    function.instruction(&Instruction::End);
    function
}

fn emit_unpack_significand(
    function: &mut Function,
    bits_local: u32,
    significand_local: u32,
    exponent_local: u32,
    scratch_local: u32,
    shift_local: u32,
) {
    function.instruction(&Instruction::LocalGet(bits_local));
    function.instruction(&Instruction::I64Const(EXPONENT_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(EXPONENT_SHIFT));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::LocalTee(scratch_local));
    function.instruction(&Instruction::I64Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(bits_local));
    function.instruction(&Instruction::I64Const(FRACTION_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::LocalTee(significand_local));
    function.instruction(&Instruction::I64Clz);
    function.instruction(&Instruction::I64Const(11));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(shift_local));
    function.instruction(&Instruction::LocalGet(significand_local));
    function.instruction(&Instruction::LocalGet(shift_local));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::LocalSet(significand_local));
    function.instruction(&Instruction::I64Const(-1022));
    function.instruction(&Instruction::LocalGet(shift_local));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(exponent_local));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::LocalGet(bits_local));
    function.instruction(&Instruction::I64Const(FRACTION_MASK));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(IMPLICIT_BIT));
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::LocalSet(significand_local));
    function.instruction(&Instruction::LocalGet(scratch_local));
    function.instruction(&Instruction::I64Const(1023));
    function.instruction(&Instruction::I64Sub);
    function.instruction(&Instruction::LocalSet(exponent_local));
    function.instruction(&Instruction::End);
}

fn emit_signed_zero_return(function: &mut Function) {
    function.instruction(&Instruction::LocalGet(RESULT_SIGN));
    function.instruction(&Instruction::F64ReinterpretI64);
    function.instruction(&Instruction::Return);
}

fn emit_nan_return(function: &mut Function) {
    function.instruction(&Instruction::F64Const(f64::NAN.into()));
    function.instruction(&Instruction::Return);
}
