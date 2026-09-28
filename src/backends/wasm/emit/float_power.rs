//! Pure binary64 power helper emitted as core Wasm instructions.
//!
//! This is a direct fdlibm `e_pow.c` port: it splits logarithm and exponential
//! work into high/low parts, preserves the original polynomial ordering, and
//! handles powers, signed zeros, exponent parity, subnormals and range edges.
//! fdlibm power is nearly rounded, not universally correctly rounded.

/*
 * @(#)e_pow.c 1.5 04/04/22 SMI
 * ====================================================
 * Copyright (C) 2004 by Sun Microsystems, Inc. All rights reserved.
 *
 * Permission to use, copy, modify, and distribute this
 * software is freely granted, provided that this notice
 * is preserved.
 * ====================================================
 */

use wasm_encoder::{BlockType, Function, Instruction, ValType};

const SIGN_MASK: i64 = i64::MIN;
const ABS_MASK: i64 = 0x7fff_ffff_ffff_ffff;
const FRACTION_MASK: i64 = 0x000f_ffff_ffff_ffff;
const HIGH_WORD_MASK: i64 = 0xffff_ffff_0000_0000_u64 as i64;
const LOW_WORD_MASK: i64 = 0x0000_0000_ffff_ffff;
const INFINITY_BITS: i64 = 0x7ff0_0000_0000_0000;
const ONE_BITS: i64 = 0x3ff0_0000_0000_0000;
const TWO_BITS: i64 = 0x4000_0000_0000_0000;
const HALF_BITS: i64 = 0x3fe0_0000_0000_0000;
const IMPLICIT_BIT: i64 = 0x0010_0000_0000_0000;
const HIGH_WORD_SHIFT: i64 = 32;
const EXPONENT_SHIFT: i64 = 52;

// Parameters 0..1 are the inputs. Integer locals occupy 2..13 and binary64
// scratch locals occupy 14..37.
const X_BITS: u32 = 2;
const Y_BITS: u32 = 3;
const ABS_X_BITS: u32 = 4;
const ABS_Y_BITS: u32 = 5;
const X_HIGH: u32 = 6;
const Y_HIGH: u32 = 7;
const Y_IS_INTEGER: u32 = 8;
const N: u32 = 9;
const K: u32 = 10;
const J: u32 = 11;
const SHIFT: u32 = 12;
const SIGNIFICAND: u32 = 13;

const AX: u32 = 14;
const SIGN: u32 = 15;
const Z: u32 = 16;
const T: u32 = 17;
const T1: u32 = 18;
const T2: u32 = 19;
const U: u32 = 20;
const V: u32 = 21;
const W: u32 = 22;
const R: u32 = 23;
const SS: u32 = 24;
const S_H: u32 = 25;
const S_L: u32 = 26;
const T_H: u32 = 27;
const T_L: u32 = 28;
const Y1: u32 = 29;
const P_H: u32 = 30;
const P_L: u32 = 31;
const Z_H: u32 = 32;
const Z_L: u32 = 33;
const BP: u32 = 34;
const DP_H: u32 = 35;
const DP_L: u32 = 36;
const S2: u32 = 37;

const BP_ONE: u64 = 0x3ff0_0000_0000_0000;
const BP_ONE_AND_HALF: u64 = 0x3ff8_0000_0000_0000;
const DP_H_ONE: u64 = 0;
const DP_H_ONE_AND_HALF: u64 = 0x3fe2_b803_4000_0000;
const DP_L_ONE: u64 = 0;
const DP_L_ONE_AND_HALF: u64 = 0x3e4c_fdeb_43cf_d006;
const L1: u64 = 0x3fe3_3333_3333_3303;
const L2: u64 = 0x3fdb_6db6_db6f_abff;
const L3: u64 = 0x3fd5_5555_518f_264d;
const L4: u64 = 0x3fd1_7460_a91d_4101;
const L5: u64 = 0x3fcd_864a_93c9_db65;
const L6: u64 = 0x3fca_7e28_4a45_4eef;
const P1: u64 = 0x3fc5_5555_5555_553e;
const P2: u64 = 0xbf66_c16c_16be_bd93;
const P3: u64 = 0x3f11_566a_af25_de2c;
const P4: u64 = 0xbebb_bd41_c5d2_6bf1;
const P5: u64 = 0x3e66_3769_72be_a4d0;
const LOG2: u64 = 0x3fe6_2e42_fefa_39ef;
const LOG2_HI: u64 = 0x3fe6_2e43_0000_0000;
const LOG2_LO: u64 = 0xbe20_5c61_0ca8_6c39;
const OVERFLOW_TAIL: u64 = 0x3c97_1547_652b_82fe;
const CP: u64 = 0x3fee_c709_dc3a_03fd;
const CP_HI: u64 = 0x3fee_c709_e000_0000;
const CP_LO: u64 = 0xbe3e_2fe0_145b_01f5;
const INV_LOG2: u64 = 0x3ff7_1547_652b_82fe;
const INV_LOG2_HI: u64 = 0x3ff7_1547_6000_0000;
const INV_LOG2_LO: u64 = 0x3e54_ae0b_f85d_df44;

macro_rules! emit {
    ($function:expr; $($instruction:expr),+ $(,)?) => {
        $( $function.instruction(&$instruction); )+
    };
}

pub(super) fn emit_float_power() -> Function {
    let mut function = Function::new([(12, ValType::I64), (24, ValType::F64)]);

    emit_input_bits(&mut function);
    emit!(function;
        Instruction::F64Const(1.0_f64.into()),
        Instruction::LocalSet(SIGN),
    );
    emit_zero_exponent_case(&mut function);
    emit_non_finite_input_cases(&mut function);
    emit_integer_exponent_classification(&mut function);
    emit_infinite_exponent_case(&mut function);
    emit_simple_exponent_cases(&mut function);
    emit_special_base_cases(&mut function);
    emit_negative_base_domain_and_sign(&mut function);

    emit_logarithm(&mut function);
    emit_split_product(&mut function);
    emit_overflow_and_underflow_checks(&mut function);
    emit_exp2_reduction(&mut function);
    emit_exp2_approximation(&mut function);
    emit_scaled_result(&mut function);

    function.instruction(&Instruction::End);
    function
}

fn emit_input_bits(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(0),
        Instruction::I64ReinterpretF64,
        Instruction::LocalTee(X_BITS),
        Instruction::I64Const(ABS_MASK),
        Instruction::I64And,
        Instruction::LocalSet(ABS_X_BITS),
        Instruction::LocalGet(1),
        Instruction::I64ReinterpretF64,
        Instruction::LocalTee(Y_BITS),
        Instruction::I64Const(ABS_MASK),
        Instruction::I64And,
        Instruction::LocalSet(ABS_Y_BITS),
    );
}

fn emit_zero_exponent_case(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::F64Const(1.0_f64.into()),
        Instruction::Return,
        Instruction::End,
    );
}

fn emit_non_finite_input_cases(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(INFINITY_BITS),
        Instruction::I64GtU,
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Const(INFINITY_BITS),
        Instruction::I64GtU,
        Instruction::I32Or,
        Instruction::If(BlockType::Empty),
    );
    emit_nan_return(function);
    emit!(function; Instruction::End);

    emit!(function;
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64ShrU,
        Instruction::LocalSet(X_HIGH),
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64ShrU,
        Instruction::LocalSet(Y_HIGH),
    );
}

fn emit_integer_exponent_classification(function: &mut Function) {
    // Negative bases need the exact integer/parity classification of the exponent.
    emit!(function;
        Instruction::I64Const(0),
        Instruction::LocalSet(Y_IS_INTEGER),
        Instruction::LocalGet(X_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::Else,
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Const(EXPONENT_SHIFT),
        Instruction::I64ShrU,
        Instruction::I64Const(0x7ff),
        Instruction::I64And,
        Instruction::LocalSet(J),
        Instruction::LocalGet(J),
        Instruction::I64Const(1076),
        Instruction::I64GeU,
        Instruction::If(BlockType::Empty),
        Instruction::I64Const(2),
        Instruction::LocalSet(Y_IS_INTEGER),
        Instruction::Else,
        Instruction::LocalGet(J),
        Instruction::I64Const(1023),
        Instruction::I64GeU,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(J),
        Instruction::I64Const(1023),
        Instruction::I64Sub,
        Instruction::LocalSet(K),
        Instruction::I64Const(52),
        Instruction::LocalGet(K),
        Instruction::I64Sub,
        Instruction::LocalSet(SHIFT),
        Instruction::I64Const(1),
        Instruction::LocalGet(SHIFT),
        Instruction::I64Shl,
        Instruction::I64Const(1),
        Instruction::I64Sub,
        Instruction::LocalSet(J),
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Const(FRACTION_MASK),
        Instruction::I64And,
        Instruction::I64Const(IMPLICIT_BIT),
        Instruction::I64Or,
        Instruction::LocalTee(SIGNIFICAND),
        Instruction::LocalGet(J),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(SIGNIFICAND),
        Instruction::LocalGet(SHIFT),
        Instruction::I64ShrU,
        Instruction::I64Const(1),
        Instruction::I64And,
        Instruction::I64Const(0),
        Instruction::I64Ne,
        Instruction::If(BlockType::Empty),
        Instruction::I64Const(1),
        Instruction::LocalSet(Y_IS_INTEGER),
        Instruction::Else,
        Instruction::I64Const(2),
        Instruction::LocalSet(Y_IS_INTEGER),
        Instruction::End,
        Instruction::End,
        Instruction::End,
        Instruction::End,
        Instruction::End,
    );
}

fn emit_infinite_exponent_case(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Const(INFINITY_BITS),
        Instruction::I64Eq,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(ONE_BITS),
        Instruction::I64Eq,
        Instruction::If(BlockType::Empty),
    );
    emit_nan_return(function);
    emit!(function;
        Instruction::End,
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(ONE_BITS),
        Instruction::I64GtU,
        Instruction::If(BlockType::Empty),
    );
    emit_y_sign_extreme_return(function, false);
    emit!(function;
        Instruction::Else,
    );
    emit_y_sign_extreme_return(function, true);
    emit!(function;
        Instruction::End,
        Instruction::End,
    );
}

fn emit_simple_exponent_cases(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(ABS_Y_BITS),
        Instruction::I64Const(ONE_BITS),
        Instruction::I64Eq,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(Y_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(0),
        Instruction::Return,
        Instruction::Else,
        Instruction::F64Const(1.0_f64.into()),
        Instruction::LocalGet(0),
        Instruction::F64Div,
        Instruction::Return,
        Instruction::End,
        Instruction::End,
        Instruction::LocalGet(Y_BITS),
        Instruction::I64Const(TWO_BITS),
        Instruction::I64Eq,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(0),
        Instruction::LocalGet(0),
        Instruction::F64Mul,
        Instruction::Return,
        Instruction::End,
        Instruction::LocalGet(Y_BITS),
        Instruction::I64Const(HALF_BITS),
        Instruction::I64Eq,
        Instruction::LocalGet(X_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::I32And,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(0),
        Instruction::F64Sqrt,
        Instruction::Return,
        Instruction::End,
    );
}

fn emit_special_base_cases(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(0),
        Instruction::F64Abs,
        Instruction::LocalSet(AX),
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Eqz,
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(ONE_BITS),
        Instruction::I64Eq,
        Instruction::I32Or,
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(INFINITY_BITS),
        Instruction::I64Eq,
        Instruction::I32Or,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(AX),
        Instruction::LocalSet(Z),
        Instruction::LocalGet(Y_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::Else,
        Instruction::F64Const(1.0_f64.into()),
        Instruction::LocalGet(Z),
        Instruction::F64Div,
        Instruction::LocalSet(Z),
        Instruction::End,
        Instruction::LocalGet(X_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::Else,
        Instruction::LocalGet(ABS_X_BITS),
        Instruction::I64Const(ONE_BITS),
        Instruction::I64Eq,
        Instruction::LocalGet(Y_IS_INTEGER),
        Instruction::I64Eqz,
        Instruction::I32And,
        Instruction::If(BlockType::Empty),
    );
    emit_nan_return(function);
    emit!(function;
        Instruction::End,
        Instruction::LocalGet(Y_IS_INTEGER),
        Instruction::I64Const(1),
        Instruction::I64Eq,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(Z),
        Instruction::F64Neg,
        Instruction::LocalSet(Z),
        Instruction::End,
        Instruction::End,
        Instruction::LocalGet(Z),
        Instruction::Return,
        Instruction::End,
    );
}

fn emit_negative_base_domain_and_sign(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(X_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::Else,
        Instruction::LocalGet(Y_IS_INTEGER),
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
    );
    emit_nan_return(function);
    emit!(function;
        Instruction::End,
        Instruction::End,
        Instruction::F64Const(1.0_f64.into()),
        Instruction::LocalSet(SIGN),
        Instruction::LocalGet(X_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
        Instruction::Else,
        Instruction::LocalGet(Y_IS_INTEGER),
        Instruction::I64Const(1),
        Instruction::I64Eq,
        Instruction::If(BlockType::Empty),
        Instruction::F64Const((-1.0_f64).into()),
        Instruction::LocalSet(SIGN),
        Instruction::End,
        Instruction::End,
    );
}

fn emit_logarithm(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(Y_HIGH),
        Instruction::I64Const(0x41e0_0000),
        Instruction::I64GtU,
        Instruction::If(BlockType::Empty),
    );

    emit_large_exponent_limits(function);
    emit_near_one_logarithm(function);

    emit!(function;
        Instruction::Else,
    );
    emit_general_logarithm(function);
    emit!(function;
        Instruction::End,
    );
}

fn emit_large_exponent_limits(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(Y_HIGH),
        Instruction::I64Const(0x43f0_0000),
        Instruction::I64GtU,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x3fef_ffff),
        Instruction::I64LeU,
        Instruction::If(BlockType::Empty),
    );
    emit_y_sign_extreme_return(function, true);
    emit!(function;
        Instruction::End,
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x3ff0_0000),
        Instruction::I64GeU,
        Instruction::If(BlockType::Empty),
    );
    emit_y_sign_extreme_return(function, false);
    emit!(function;
        Instruction::End,
        Instruction::End,
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x3fef_ffff),
        Instruction::I64LtU,
        Instruction::If(BlockType::Empty),
    );
    emit_y_sign_extreme_return(function, true);
    emit!(function;
        Instruction::End,
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x3ff0_0000),
        Instruction::I64GtU,
        Instruction::If(BlockType::Empty),
    );
    emit_y_sign_extreme_return(function, false);
    emit!(function; Instruction::End);
}

fn emit_near_one_logarithm(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(AX),
        Instruction::F64Const(1.0_f64.into()),
        Instruction::F64Sub,
        Instruction::LocalSet(T),
        Instruction::LocalGet(T),
        Instruction::LocalGet(T),
        Instruction::F64Mul,
        Instruction::F64Const(f64::from_bits(0x3fe0_0000_0000_0000).into()),
        Instruction::LocalGet(T),
        Instruction::F64Const(f64::from_bits(0x3fd5_5555_5555_5555).into()),
        Instruction::LocalGet(T),
        Instruction::F64Const(f64::from_bits(0x3fd0_0000_0000_0000).into()),
        Instruction::F64Mul,
        Instruction::F64Sub,
        Instruction::F64Mul,
        Instruction::F64Sub,
        Instruction::F64Mul,
        Instruction::LocalSet(W),
        Instruction::F64Const(f64::from_bits(INV_LOG2_HI).into()),
        Instruction::LocalGet(T),
        Instruction::F64Mul,
        Instruction::LocalSet(U),
        Instruction::LocalGet(T),
        Instruction::F64Const(f64::from_bits(INV_LOG2_LO).into()),
        Instruction::F64Mul,
        Instruction::LocalGet(W),
        Instruction::F64Const(f64::from_bits(INV_LOG2).into()),
        Instruction::F64Mul,
        Instruction::F64Sub,
        Instruction::LocalSet(V),
        Instruction::LocalGet(U),
        Instruction::LocalGet(V),
        Instruction::F64Add,
        Instruction::LocalSet(T1),
    );
    emit_clear_low_word(function, T1);
    emit!(function;
        Instruction::LocalGet(V),
        Instruction::LocalGet(T1),
        Instruction::LocalGet(U),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(T2),
    );
}

fn emit_general_logarithm(function: &mut Function) {
    emit!(function;
        Instruction::I64Const(0),
        Instruction::LocalSet(N),
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x0010_0000),
        Instruction::I64LtU,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(AX),
        Instruction::F64Const(f64::from_bits(0x4340_0000_0000_0000).into()),
        Instruction::F64Mul,
        Instruction::LocalSet(AX),
        Instruction::I64Const(-53),
        Instruction::LocalSet(N),
        Instruction::LocalGet(AX),
        Instruction::I64ReinterpretF64,
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64ShrU,
        Instruction::LocalSet(X_HIGH),
        Instruction::End,
        Instruction::LocalGet(N),
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(20),
        Instruction::I64ShrU,
        Instruction::I64Const(0x3ff),
        Instruction::I64Sub,
        Instruction::I64Add,
        Instruction::LocalSet(N),
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x000f_ffff),
        Instruction::I64And,
        Instruction::LocalSet(J),
        Instruction::LocalGet(J),
        Instruction::I64Const(0x3988e),
        Instruction::I64LeU,
        Instruction::If(BlockType::Empty),
        Instruction::I64Const(0),
        Instruction::LocalSet(K),
        Instruction::Else,
        Instruction::LocalGet(J),
        Instruction::I64Const(0xbb67a),
        Instruction::I64LtU,
        Instruction::If(BlockType::Empty),
        Instruction::I64Const(1),
        Instruction::LocalSet(K),
        Instruction::Else,
        Instruction::I64Const(0),
        Instruction::LocalSet(K),
        Instruction::LocalGet(N),
        Instruction::I64Const(1),
        Instruction::I64Add,
        Instruction::LocalSet(N),
        Instruction::End,
        Instruction::End,
        Instruction::LocalGet(J),
        Instruction::I64Const(0x3ff0_0000),
        Instruction::I64Or,
        Instruction::LocalSet(X_HIGH),
    );

    // The upper reduction interval uses 0.75 and one lower exponent bin.
    emit!(function;
        Instruction::LocalGet(J),
        Instruction::I64Const(0xbb67a),
        Instruction::I64GeU,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(0x0010_0000),
        Instruction::I64Sub,
        Instruction::LocalSet(X_HIGH),
        Instruction::End,
        Instruction::LocalGet(AX),
        Instruction::I64ReinterpretF64,
        Instruction::I64Const(LOW_WORD_MASK),
        Instruction::I64And,
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64Shl,
        Instruction::I64Or,
        Instruction::F64ReinterpretI64,
        Instruction::LocalSet(AX),
    );

    emit_set_f64_by_k(function, K, BP, BP_ONE, BP_ONE_AND_HALF);
    emit!(function;
        Instruction::LocalGet(AX),
        Instruction::LocalGet(BP),
        Instruction::F64Sub,
        Instruction::LocalSet(U),
        Instruction::F64Const(1.0_f64.into()),
        Instruction::LocalGet(AX),
        Instruction::LocalGet(BP),
        Instruction::F64Add,
        Instruction::F64Div,
        Instruction::LocalSet(V),
        Instruction::LocalGet(U),
        Instruction::LocalGet(V),
        Instruction::F64Mul,
        Instruction::LocalSet(SS),
        Instruction::LocalGet(SS),
        Instruction::LocalSet(S_H),
    );
    emit_clear_low_word(function, S_H);

    emit!(function;
        Instruction::LocalGet(X_HIGH),
        Instruction::I64Const(1),
        Instruction::I64ShrU,
        Instruction::I64Const(0x2000_0000),
        Instruction::I64Or,
        Instruction::I64Const(0x0008_0000),
        Instruction::I64Add,
        Instruction::LocalGet(K),
        Instruction::I64Const(18),
        Instruction::I64Shl,
        Instruction::I64Add,
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64Shl,
        Instruction::F64ReinterpretI64,
        Instruction::LocalSet(T_H),
        Instruction::LocalGet(AX),
        Instruction::LocalGet(T_H),
        Instruction::LocalGet(BP),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(T_L),
        Instruction::LocalGet(U),
        Instruction::LocalGet(S_H),
        Instruction::LocalGet(T_H),
        Instruction::F64Mul,
        Instruction::F64Sub,
        Instruction::LocalGet(S_H),
        Instruction::LocalGet(T_L),
        Instruction::F64Mul,
        Instruction::F64Sub,
        Instruction::LocalGet(V),
        Instruction::F64Mul,
        Instruction::LocalSet(S_L),
    );

    emit!(function;
        Instruction::LocalGet(SS),
        Instruction::LocalGet(SS),
        Instruction::F64Mul,
        Instruction::LocalSet(S2),
        Instruction::F64Const(f64::from_bits(L6).into()),
        Instruction::LocalSet(R),
    );
    emit_poly_step(function, L5);
    emit_poly_step(function, L4);
    emit_poly_step(function, L3);
    emit_poly_step(function, L2);
    emit_poly_step(function, L1);
    emit!(function;
        Instruction::LocalGet(S2),
        Instruction::LocalGet(S2),
        Instruction::F64Mul,
        Instruction::LocalGet(R),
        Instruction::F64Mul,
        Instruction::LocalSet(R),
        Instruction::LocalGet(R),
        Instruction::LocalGet(S_L),
        Instruction::LocalGet(S_H),
        Instruction::LocalGet(SS),
        Instruction::F64Add,
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalSet(R),
        Instruction::LocalGet(S_H),
        Instruction::LocalGet(S_H),
        Instruction::F64Mul,
        Instruction::LocalSet(S2),
        Instruction::F64Const(3.0_f64.into()),
        Instruction::LocalGet(S2),
        Instruction::F64Add,
        Instruction::LocalGet(R),
        Instruction::F64Add,
        Instruction::LocalSet(T_H),
    );
    emit_clear_low_word(function, T_H);
    emit!(function;
        Instruction::LocalGet(R),
        Instruction::LocalGet(T_H),
        Instruction::F64Const(3.0_f64.into()),
        Instruction::F64Sub,
        Instruction::LocalGet(S2),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(T_L),
        Instruction::LocalGet(S_H),
        Instruction::LocalGet(T_H),
        Instruction::F64Mul,
        Instruction::LocalSet(U),
        Instruction::LocalGet(S_L),
        Instruction::LocalGet(T_H),
        Instruction::F64Mul,
        Instruction::LocalGet(T_L),
        Instruction::LocalGet(SS),
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalSet(V),
        Instruction::LocalGet(U),
        Instruction::LocalGet(V),
        Instruction::F64Add,
        Instruction::LocalSet(P_H),
    );
    emit_clear_low_word(function, P_H);
    emit!(function;
        Instruction::LocalGet(V),
        Instruction::LocalGet(P_H),
        Instruction::LocalGet(U),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(P_L),
    );

    emit_set_f64_by_k(function, K, DP_H, DP_H_ONE, DP_H_ONE_AND_HALF);
    emit_set_f64_by_k(function, K, DP_L, DP_L_ONE, DP_L_ONE_AND_HALF);
    emit!(function;
        Instruction::F64Const(f64::from_bits(CP_HI).into()),
        Instruction::LocalGet(P_H),
        Instruction::F64Mul,
        Instruction::LocalSet(Z_H),
        Instruction::F64Const(f64::from_bits(CP_LO).into()),
        Instruction::LocalGet(P_H),
        Instruction::F64Mul,
        Instruction::LocalGet(P_L),
        Instruction::F64Const(f64::from_bits(CP).into()),
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalGet(DP_L),
        Instruction::F64Add,
        Instruction::LocalSet(Z_L),
        Instruction::LocalGet(N),
        Instruction::F64ConvertI64S,
        Instruction::LocalSet(T),
        Instruction::LocalGet(Z_H),
        Instruction::LocalGet(Z_L),
        Instruction::F64Add,
        Instruction::LocalGet(DP_H),
        Instruction::F64Add,
        Instruction::LocalGet(T),
        Instruction::F64Add,
        Instruction::LocalSet(T1),
    );
    emit_clear_low_word(function, T1);
    emit!(function;
        Instruction::LocalGet(Z_L),
        Instruction::LocalGet(T1),
        Instruction::LocalGet(T),
        Instruction::F64Sub,
        Instruction::LocalGet(DP_H),
        Instruction::F64Sub,
        Instruction::LocalGet(Z_H),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(T2),
    );
}

fn emit_poly_step(function: &mut Function, coefficient: u64) {
    emit!(function;
        Instruction::F64Const(f64::from_bits(coefficient).into()),
        Instruction::LocalGet(S2),
        Instruction::LocalGet(R),
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalSet(R),
    );
}

fn emit_set_f64_by_k(
    function: &mut Function,
    k_local: u32,
    destination: u32,
    zero_bits: u64,
    one_bits: u64,
) {
    emit!(function;
        Instruction::LocalGet(k_local),
        Instruction::I64Eqz,
        Instruction::If(BlockType::Result(ValType::F64)),
    );
    emit_f64_constant(function, zero_bits);
    emit!(function; Instruction::Else);
    emit_f64_constant(function, one_bits);
    emit!(function;
        Instruction::End,
        Instruction::LocalSet(destination),
    );
}

fn emit_split_product(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(1),
        Instruction::LocalSet(Y1),
    );
    emit_clear_low_word(function, Y1);
    emit!(function;
        Instruction::LocalGet(1),
        Instruction::LocalGet(Y1),
        Instruction::F64Sub,
        Instruction::LocalGet(T1),
        Instruction::F64Mul,
        Instruction::LocalGet(1),
        Instruction::LocalGet(T2),
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalSet(P_L),
        Instruction::LocalGet(Y1),
        Instruction::LocalGet(T1),
        Instruction::F64Mul,
        Instruction::LocalSet(P_H),
        Instruction::LocalGet(P_L),
        Instruction::LocalGet(P_H),
        Instruction::F64Add,
        Instruction::LocalSet(Z),
    );
}

fn emit_overflow_and_underflow_checks(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(Z),
        Instruction::F64Const(1024.0_f64.into()),
        Instruction::F64Ge,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(Z),
        Instruction::F64Const(1024.0_f64.into()),
        Instruction::F64Gt,
        Instruction::If(BlockType::Empty),
    );
    emit_signed_extreme_return(function, true);
    emit!(function;
        Instruction::End,
        Instruction::LocalGet(P_L),
        Instruction::F64Const(f64::from_bits(OVERFLOW_TAIL).into()),
        Instruction::F64Add,
        Instruction::LocalGet(Z),
        Instruction::LocalGet(P_H),
        Instruction::F64Sub,
        Instruction::F64Gt,
        Instruction::If(BlockType::Empty),
    );
    emit_signed_extreme_return(function, true);
    emit!(function;
        Instruction::End,
        Instruction::End,
        Instruction::LocalGet(Z),
        Instruction::F64Const((-1075.0_f64).into()),
        Instruction::F64Le,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(Z),
        Instruction::F64Const((-1075.0_f64).into()),
        Instruction::F64Lt,
        Instruction::If(BlockType::Empty),
    );
    emit_signed_extreme_return(function, false);
    emit!(function;
        Instruction::End,
        Instruction::LocalGet(P_L),
        Instruction::LocalGet(Z),
        Instruction::LocalGet(P_H),
        Instruction::F64Sub,
        Instruction::F64Le,
        Instruction::If(BlockType::Empty),
    );
    emit_signed_extreme_return(function, false);
    emit!(function;
        Instruction::End,
        Instruction::End,
    );
}

fn emit_exp2_reduction(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(Z),
        Instruction::I64ReinterpretF64,
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64ShrS,
        Instruction::LocalSet(J),
        Instruction::I64Const(0),
        Instruction::LocalSet(N),
        Instruction::LocalGet(J),
        Instruction::I64Const(0x7fff_ffff),
        Instruction::I64And,
        Instruction::LocalSet(SHIFT),
        Instruction::LocalGet(SHIFT),
        Instruction::I64Const(0x3fe0_0000),
        Instruction::I64GtU,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(SHIFT),
        Instruction::I64Const(20),
        Instruction::I64ShrU,
        Instruction::I64Const(0x3ff),
        Instruction::I64Sub,
        Instruction::LocalSet(K),
        Instruction::LocalGet(J),
        Instruction::I64Const(0x0010_0000),
        Instruction::LocalGet(K),
        Instruction::I64Const(1),
        Instruction::I64Add,
        Instruction::I64ShrU,
        Instruction::I64Add,
        Instruction::LocalSet(N),
        Instruction::LocalGet(N),
        Instruction::I64Const(0x7fff_ffff),
        Instruction::I64And,
        Instruction::I64Const(20),
        Instruction::I64ShrU,
        Instruction::I64Const(0x3ff),
        Instruction::I64Sub,
        Instruction::LocalSet(K),
        Instruction::I64Const(0x000f_ffff),
        Instruction::LocalGet(K),
        Instruction::I64ShrU,
        Instruction::LocalSet(SHIFT),
        Instruction::LocalGet(N),
        Instruction::LocalGet(SHIFT),
        Instruction::I64Const(-1),
        Instruction::I64Xor,
        Instruction::I64And,
        Instruction::LocalSet(J),
        Instruction::LocalGet(J),
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64Shl,
        Instruction::F64ReinterpretI64,
        Instruction::LocalSet(T),
        Instruction::LocalGet(N),
        Instruction::I64Const(0x000f_ffff),
        Instruction::I64And,
        Instruction::I64Const(IMPLICIT_BIT >> 32),
        Instruction::I64Or,
        Instruction::I64Const(20),
        Instruction::LocalGet(K),
        Instruction::I64Sub,
        Instruction::I64ShrU,
        Instruction::LocalSet(N),
        Instruction::LocalGet(J),
        Instruction::I64Const(0),
        Instruction::I64LtS,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(N),
        Instruction::I64Const(-1),
        Instruction::I64Mul,
        Instruction::LocalSet(N),
        Instruction::End,
        Instruction::LocalGet(P_H),
        Instruction::LocalGet(T),
        Instruction::F64Sub,
        Instruction::LocalSet(P_H),
        Instruction::End,
    );
}

fn emit_exp2_approximation(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(P_L),
        Instruction::LocalGet(P_H),
        Instruction::F64Add,
        Instruction::LocalSet(T),
    );
    emit_clear_low_word(function, T);
    emit!(function;
        Instruction::LocalGet(T),
        Instruction::F64Const(f64::from_bits(LOG2_HI).into()),
        Instruction::F64Mul,
        Instruction::LocalSet(U),
        Instruction::LocalGet(P_L),
        Instruction::LocalGet(T),
        Instruction::LocalGet(P_H),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::F64Const(f64::from_bits(LOG2).into()),
        Instruction::F64Mul,
        Instruction::LocalGet(T),
        Instruction::F64Const(f64::from_bits(LOG2_LO).into()),
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalSet(V),
        Instruction::LocalGet(U),
        Instruction::LocalGet(V),
        Instruction::F64Add,
        Instruction::LocalSet(Z),
        Instruction::LocalGet(V),
        Instruction::LocalGet(Z),
        Instruction::LocalGet(U),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(W),
        Instruction::LocalGet(Z),
        Instruction::LocalGet(Z),
        Instruction::F64Mul,
        Instruction::LocalSet(T),
        Instruction::F64Const(f64::from_bits(P5).into()),
        Instruction::LocalSet(T1),
    );
    emit_exp_poly_step(function, P4);
    emit_exp_poly_step(function, P3);
    emit_exp_poly_step(function, P2);
    emit_exp_poly_step(function, P1);
    emit!(function;
        Instruction::LocalGet(Z),
        Instruction::LocalGet(T),
        Instruction::LocalGet(T1),
        Instruction::F64Mul,
        Instruction::F64Sub,
        Instruction::LocalSet(T1),
        Instruction::LocalGet(Z),
        Instruction::LocalGet(T1),
        Instruction::F64Mul,
        Instruction::LocalGet(T1),
        Instruction::F64Const(2.0_f64.into()),
        Instruction::F64Sub,
        Instruction::F64Div,
        Instruction::LocalGet(W),
        Instruction::LocalGet(Z),
        Instruction::F64Mul,
        Instruction::LocalGet(W),
        Instruction::F64Add,
        Instruction::F64Sub,
        Instruction::LocalSet(R),
        Instruction::F64Const(1.0_f64.into()),
        Instruction::LocalGet(R),
        Instruction::LocalGet(Z),
        Instruction::F64Sub,
        Instruction::F64Sub,
        Instruction::LocalSet(Z),
        Instruction::LocalGet(Z),
        Instruction::I64ReinterpretF64,
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64ShrS,
        Instruction::LocalSet(J),
        Instruction::LocalGet(J),
        Instruction::LocalGet(N),
        Instruction::I64Const(20),
        Instruction::I64Shl,
        Instruction::I64Add,
        Instruction::LocalSet(J),
    );
}

fn emit_exp_poly_step(function: &mut Function, coefficient: u64) {
    emit!(function;
        Instruction::F64Const(f64::from_bits(coefficient).into()),
        Instruction::LocalGet(T),
        Instruction::LocalGet(T1),
        Instruction::F64Mul,
        Instruction::F64Add,
        Instruction::LocalSet(T1),
    );
}

fn emit_scaled_result(function: &mut Function) {
    emit!(function;
        Instruction::LocalGet(J),
        Instruction::I64Const(20),
        Instruction::I64ShrS,
        Instruction::I64Const(0),
        Instruction::I64LeS,
        Instruction::If(BlockType::Empty),
    );
    emit_scalbn_subnormal(function);
    emit!(function;
        Instruction::Else,
        Instruction::LocalGet(Z),
        Instruction::I64ReinterpretF64,
        Instruction::I64Const(LOW_WORD_MASK),
        Instruction::I64And,
        Instruction::LocalGet(J),
        Instruction::I64Const(HIGH_WORD_SHIFT),
        Instruction::I64Shl,
        Instruction::I64Or,
        Instruction::F64ReinterpretI64,
        Instruction::LocalSet(Z),
        Instruction::End,
        Instruction::LocalGet(Z),
        Instruction::LocalGet(SIGN),
        Instruction::F64Mul,
    );
}

fn emit_scalbn_subnormal(function: &mut Function) {
    // `n` is bounded to [-1075, -1022] here. For subnormal scaling, first
    // multiply by a normal power and then by 2^-54 to incur only one final rounding.
    emit!(function;
        Instruction::LocalGet(N),
        Instruction::I64Const(-1022),
        Instruction::I64GeS,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(N),
        Instruction::I64Const(1023),
        Instruction::I64Add,
        Instruction::I64Const(EXPONENT_SHIFT),
        Instruction::I64Shl,
        Instruction::F64ReinterpretI64,
        Instruction::LocalGet(Z),
        Instruction::F64Mul,
        Instruction::LocalSet(Z),
        Instruction::Else,
        Instruction::LocalGet(N),
        Instruction::I64Const(54),
        Instruction::I64Add,
        Instruction::I64Const(1023),
        Instruction::I64Add,
        Instruction::I64Const(EXPONENT_SHIFT),
        Instruction::I64Shl,
        Instruction::F64ReinterpretI64,
        Instruction::LocalGet(Z),
        Instruction::F64Mul,
        Instruction::F64Const(f64::from_bits(0x3c90_0000_0000_0000).into()),
        Instruction::F64Mul,
        Instruction::LocalSet(Z),
        Instruction::End,
    );
}

fn emit_clear_low_word(function: &mut Function, float_local: u32) {
    emit!(function;
        Instruction::LocalGet(float_local),
        Instruction::I64ReinterpretF64,
        Instruction::I64Const(HIGH_WORD_MASK),
        Instruction::I64And,
        Instruction::F64ReinterpretI64,
        Instruction::LocalSet(float_local),
    );
}

fn emit_y_sign_extreme_return(function: &mut Function, negative_y_overflows: bool) {
    emit!(function;
        Instruction::LocalGet(Y_BITS),
        Instruction::I64Const(SIGN_MASK),
        Instruction::I64And,
        Instruction::I64Eqz,
        Instruction::If(BlockType::Empty),
    );
    emit_signed_extreme_return(function, !negative_y_overflows);
    emit!(function; Instruction::Else);
    emit_signed_extreme_return(function, negative_y_overflows);
    emit!(function; Instruction::End);
}

fn emit_signed_extreme_return(function: &mut Function, overflow: bool) {
    emit!(function;
        Instruction::LocalGet(SIGN),
    );
    if overflow {
        emit_f64_constant(function, INFINITY_BITS as u64);
    } else {
        emit!(function; Instruction::F64Const(0.0_f64.into()));
    }
    emit!(function;
        Instruction::F64Mul,
        Instruction::Return,
    );
}

fn emit_nan_return(function: &mut Function) {
    emit!(function;
        Instruction::F64Const(f64::NAN.into()),
        Instruction::Return,
    );
}

fn emit_f64_constant(function: &mut Function, bits: u64) {
    function.instruction(&Instruction::F64Const(f64::from_bits(bits).into()));
}
