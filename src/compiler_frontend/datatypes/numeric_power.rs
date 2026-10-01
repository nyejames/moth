//! Portable binary64 power arithmetic for constant folding.
//!
//! WHAT: evaluate binary64 power with the fdlibm `e_pow.c` reduction and scaling algorithm.
//! WHY: compile-time Float powers must use the same operation order and edge handling as the
//!      Wasm implementation instead of inheriting the host's libm result.
//!
//! fdlibm power is nearly rounded, not universally correctly rounded. This implementation keeps
//! the 2004 fdlibm constants, polynomial order, bit-level exponent handling, and subnormal scaling.

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

const SIGN_MASK: u64 = 0x8000_0000_0000_0000;
const ABS_MASK: u64 = 0x7fff_ffff_ffff_ffff;
const FRACTION_MASK: u64 = 0x000f_ffff_ffff_ffff;
const HIGH_WORD_MASK: u64 = 0xffff_ffff_0000_0000;
const LOW_WORD_MASK: u64 = 0x0000_0000_ffff_ffff;
const INFINITY_BITS: u64 = 0x7ff0_0000_0000_0000;
const ONE_BITS: u64 = 0x3ff0_0000_0000_0000;
const TWO_BITS: u64 = 0x4000_0000_0000_0000;
const HALF_BITS: u64 = 0x3fe0_0000_0000_0000;
const IMPLICIT_BIT: u64 = 0x0010_0000_0000_0000;

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

/// Evaluate binary64 power with the same fdlibm algorithm used by emitted backends.
///
/// The explicit special-case and rounding sequence matches the portable Wasm instruction port;
/// in particular, it does not call the host `pow`/`powf` implementation.
pub(crate) fn pow(x: f64, y: f64) -> f64 {
    let x_bits = x.to_bits();
    let y_bits = y.to_bits();
    let abs_x_bits = x_bits & ABS_MASK;
    let abs_y_bits = y_bits & ABS_MASK;

    // Preserve fdlibm's precedence: every signed zero exponent returns one, including NaN^0.
    if abs_y_bits == 0 {
        return 1.0;
    }
    if abs_x_bits > INFINITY_BITS || abs_y_bits > INFINITY_BITS {
        return f64::NAN;
    }

    let x_high = abs_x_bits >> 32;
    let y_high = abs_y_bits >> 32;
    let y_is_integer = if x_bits & SIGN_MASK != 0 {
        integer_exponent_parity(abs_y_bits)
    } else {
        0
    };

    if abs_y_bits == INFINITY_BITS {
        if abs_x_bits == ONE_BITS {
            return f64::NAN;
        }
        let negative_y_overflows = abs_x_bits < ONE_BITS;
        return extreme_for_exponent(y_bits, negative_y_overflows, 1.0);
    }

    if abs_y_bits == ONE_BITS {
        return if y_bits & SIGN_MASK == 0 { x } else { 1.0 / x };
    }
    if y_bits == TWO_BITS {
        return x * x;
    }
    if y_bits == HALF_BITS && x_bits & SIGN_MASK == 0 {
        return x.sqrt();
    }

    let ax = f64::from_bits(abs_x_bits);
    if abs_x_bits == 0 || abs_x_bits == ONE_BITS || abs_x_bits == INFINITY_BITS {
        let mut z = ax;
        if y_bits & SIGN_MASK != 0 {
            z = 1.0 / z;
        }
        if x_bits & SIGN_MASK != 0 {
            if abs_x_bits == ONE_BITS && y_is_integer == 0 {
                return f64::NAN;
            }
            if y_is_integer == 1 {
                z = -z;
            }
        }
        return z;
    }

    if x_bits & SIGN_MASK != 0 && y_is_integer == 0 {
        return f64::NAN;
    }

    let sign = if x_bits & SIGN_MASK != 0 && y_is_integer == 1 {
        -1.0
    } else {
        1.0
    };

    if y_high > 0x41e0_0000 {
        if y_high > 0x43f0_0000 {
            if x_high <= 0x3fef_ffff {
                return extreme_for_exponent(y_bits, true, sign);
            }
            if x_high >= 0x3ff0_0000 {
                return extreme_for_exponent(y_bits, false, sign);
            }
        }
        if x_high < 0x3fef_ffff {
            return extreme_for_exponent(y_bits, true, sign);
        }
        if x_high > 0x3ff0_0000 {
            return extreme_for_exponent(y_bits, false, sign);
        }
    }

    let (t1, t2) = if y_high > 0x41e0_0000 {
        near_one_logarithm(ax)
    } else {
        general_logarithm(ax, x_high)
    };
    let (p_h, p_l, z) = split_product(y, t1, t2);

    if z >= 1024.0 && (z > 1024.0 || p_l + f64::from_bits(OVERFLOW_TAIL) > z - p_h) {
        return sign * f64::INFINITY;
    }
    if z <= -1075.0 && (z < -1075.0 || p_l <= z - p_h) {
        return sign * 0.0;
    }

    let (p_h, n) = reduce_exp2(p_h, z);
    let (z, j) = approximate_exp2(p_h, p_l);
    scale_result(z, j + (n << 20), n, sign)
}

/// Return 0 for non-integers, 1 for odd integers, and 2 for even integers.
fn integer_exponent_parity(abs_y_bits: u64) -> u64 {
    let exponent = (abs_y_bits >> 52) & 0x7ff;
    if exponent >= 1076 {
        return 2;
    }
    if exponent < 1023 {
        return 0;
    }

    let shift = 52 - (exponent - 1023);
    let significand = (abs_y_bits & FRACTION_MASK) | IMPLICIT_BIT;
    let integer_mask = (1_u64 << shift) - 1;
    if significand & integer_mask != 0 {
        return 0;
    }
    if (significand >> shift) & 1 != 0 {
        1
    } else {
        2
    }
}

/// Return signed infinity or zero according to the exponent sign and base magnitude.
fn extreme_for_exponent(y_bits: u64, negative_y_overflows: bool, sign: f64) -> f64 {
    let y_is_negative = y_bits & SIGN_MASK != 0;
    if y_is_negative == negative_y_overflows {
        sign * f64::INFINITY
    } else {
        sign * 0.0
    }
}

/// The accurate `log2(ax)` reduction used for bases sufficiently close to one.
fn near_one_logarithm(ax: f64) -> (f64, f64) {
    let t = ax - 1.0;
    let w = (t * t) * (0.5 - (t * (f64::from_bits(0x3fd5_5555_5555_5555) - (t * 0.25))));
    let u = f64::from_bits(INV_LOG2_HI) * t;
    let v = (t * f64::from_bits(INV_LOG2_LO)) - (w * f64::from_bits(INV_LOG2));
    let t1 = clear_low_word(u + v);
    let t2 = v - (t1 - u);
    (t1, t2)
}

/// The split `log2(ax)` reduction from fdlibm, preserving each rounded operation's order.
fn general_logarithm(mut ax: f64, mut x_high: u64) -> (f64, f64) {
    let mut n = 0_i64;
    if x_high < 0x0010_0000 {
        ax *= f64::from_bits(0x4340_0000_0000_0000);
        n = -53;
        x_high = ax.to_bits() >> 32;
    }
    n += ((x_high >> 20) as i64) - 0x3ff;

    let j = x_high & 0x000f_ffff;
    let k = if j <= 0x0003_988e {
        0_i64
    } else if j < 0x000b_b67a {
        1_i64
    } else {
        n += 1;
        0_i64
    };

    x_high = j | 0x3ff0_0000;
    if j >= 0x000b_b67a {
        x_high -= 0x0010_0000;
    }
    ax = f64::from_bits((ax.to_bits() & LOW_WORD_MASK) | (x_high << 32));

    let bp = if k == 0 {
        f64::from_bits(BP_ONE)
    } else {
        f64::from_bits(BP_ONE_AND_HALF)
    };
    let dp_h = f64::from_bits(if k == 0 { DP_H_ONE } else { DP_H_ONE_AND_HALF });
    let dp_l = f64::from_bits(if k == 0 { DP_L_ONE } else { DP_L_ONE_AND_HALF });
    let u = ax - bp;
    let v = 1.0 / (ax + bp);
    let ss = u * v;
    let s_h = clear_low_word(ss);

    let t_h_bits = (((x_high >> 1) | 0x2000_0000) + 0x0008_0000 + ((k as u64) << 18)) << 32;
    let t_h = f64::from_bits(t_h_bits);
    let t_l = ax - (t_h - bp);
    let s_l = ((u - (s_h * t_h)) - (s_h * t_l)) * v;

    let s2 = ss * ss;
    let mut r = f64::from_bits(L6);
    r = f64::from_bits(L5) + (s2 * r);
    r = f64::from_bits(L4) + (s2 * r);
    r = f64::from_bits(L3) + (s2 * r);
    r = f64::from_bits(L2) + (s2 * r);
    r = f64::from_bits(L1) + (s2 * r);
    r = ((s2 * s2) * r) + (s_l * (s_h + ss));

    let s2 = s_h * s_h;
    let t_h = clear_low_word((3.0 + s2) + r);
    let t_l = r - ((t_h - 3.0) - s2);
    let u = s_h * t_h;
    let v = (s_l * t_h) + (t_l * ss);
    let p_h = clear_low_word(u + v);
    let p_l = v - (p_h - u);

    let z_h = f64::from_bits(CP_HI) * p_h;
    let z_l = ((f64::from_bits(CP_LO) * p_h) + (p_l * f64::from_bits(CP))) + dp_l;
    let t = n as f64;
    let t1 = clear_low_word(((z_h + z_l) + dp_h) + t);
    let t2 = z_l - (((t1 - t) - dp_h) - z_h);
    (t1, t2)
}

/// Multiply the split logarithm by `y` as two separately rounded products.
fn split_product(y: f64, t1: f64, t2: f64) -> (f64, f64, f64) {
    let y1 = clear_low_word(y);
    let p_l = ((y - y1) * t1) + (y * t2);
    let p_h = y1 * t1;
    let z = p_l + p_h;
    (p_h, p_l, z)
}

/// Reduce `p_h` by the nearest integral exponent and return its signed scale adjustment.
fn reduce_exp2(mut p_h: f64, z: f64) -> (f64, i64) {
    let mut n = 0_i64;
    let mut j = (z.to_bits() as i64) >> 32;
    let shift = (j & 0x7fff_ffff) as u64;
    if shift > 0x3fe0_0000 {
        let mut k = ((shift >> 20) as i64) - 0x3ff;
        n = j + (0x0010_0000_i64 >> (k + 1));
        k = ((n & 0x7fff_ffff) >> 20) - 0x3ff;
        let exponent_shift = (k as u32) & 63;
        let shift = 0x000f_ffff_u64 >> exponent_shift;
        j = n & !((shift) as i64);
        let t = f64::from_bits((j as u64) << 32);
        let n_shift = ((20 - k) as u32) & 63;
        n = (((n as u64 & 0x000f_ffff) | 0x0010_0000) >> n_shift) as i64;
        if j < 0 {
            n = -n;
        }
        p_h -= t;
    }
    (p_h, n)
}

/// Evaluate the fdlibm exponential polynomial after the `log2` reduction.
fn approximate_exp2(p_h: f64, p_l: f64) -> (f64, i64) {
    let t = clear_low_word(p_l + p_h);
    let u = t * f64::from_bits(LOG2_HI);
    let v = ((p_l - (t - p_h)) * f64::from_bits(LOG2)) + (t * f64::from_bits(LOG2_LO));
    let z = u + v;
    let w = v - (z - u);
    let t = z * z;
    let mut t1 = f64::from_bits(P5);
    t1 = (t * t1) + f64::from_bits(P4);
    t1 = (t * t1) + f64::from_bits(P3);
    t1 = (t * t1) + f64::from_bits(P2);
    t1 = (t * t1) + f64::from_bits(P1);
    let t1 = z - (t * t1);
    let r = ((z * t1) / (t1 - 2.0)) - ((w * z) + w);
    let z = 1.0 - (r - z);
    let j = (z.to_bits() as i64) >> 32;
    (z, j)
}

/// Restore the integral exponent, including fdlibm's single-rounding subnormal path.
fn scale_result(z: f64, j: i64, n: i64, sign: f64) -> f64 {
    let mut z = z;
    if j >> 20 <= 0 {
        if n >= -1022 {
            let scale = f64::from_bits(((n + 1023) as u64) << 52);
            z *= scale;
        } else {
            let scale = f64::from_bits(((n + 54 + 1023) as u64) << 52);
            z *= scale;
            z *= f64::from_bits(0x3c90_0000_0000_0000);
        }
    } else {
        let bits = (z.to_bits() & LOW_WORD_MASK) | ((j as u64) << 32);
        z = f64::from_bits(bits);
    }
    z * sign
}

/// Clear the low 32 bits of a binary64 significand exactly as the fdlibm split requires.
fn clear_low_word(value: f64) -> f64 {
    f64::from_bits(value.to_bits() & HIGH_WORD_MASK)
}
