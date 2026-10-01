/*
 * @(#)e_pow.c 1.5 04/04/22 SMI
 * ====================================================
 * Copyright (C) 2004 by Sun Microsystems, Inc. All rights reserved.
 *
 * Permission to use, copy, modify, and distribute this
 * software is freely granted, provided that this notice
 * is preserved.
 * ====================================================
 *
 * This is the binary64 fdlibm power algorithm shared with the Wasm backend.
 * Its polynomial ordering and split-word arithmetic are intentional: fdlibm
 * is nearly rounded, not universally correctly rounded.
 */

const __moth_float_power_bits = new DataView(new ArrayBuffer(8));

function __moth_float_power_high(value) {
    __moth_float_power_bits.setFloat64(0, value, false);
    return __moth_float_power_bits.getUint32(0, false);
}

function __moth_float_power_low(value) {
    __moth_float_power_bits.setFloat64(0, value, false);
    return __moth_float_power_bits.getUint32(4, false);
}

function __moth_float_power_from_words(high, low) {
    __moth_float_power_bits.setUint32(0, high >>> 0, false);
    __moth_float_power_bits.setUint32(4, low >>> 0, false);
    return __moth_float_power_bits.getFloat64(0, false);
}

function __moth_float_power_clear_low(value) {
    return __moth_float_power_from_words(__moth_float_power_high(value), 0);
}

function __moth_float_power_extreme(sign, overflow) {
    return sign * (overflow ? Infinity : 0);
}

function __moth_float_power_y_extreme(sign, yIsNegative, negativeYOverflows) {
    return __moth_float_power_extreme(sign, yIsNegative ? negativeYOverflows : !negativeYOverflows);
}

const __moth_float_power_bp_one = __moth_float_power_from_words(0x3ff00000, 0x00000000);
const __moth_float_power_bp_one_and_half = __moth_float_power_from_words(0x3ff80000, 0x00000000);
const __moth_float_power_dp_h_one_and_half = __moth_float_power_from_words(0x3fe2b803, 0x40000000);
const __moth_float_power_dp_l_one_and_half = __moth_float_power_from_words(0x3e4cfdeb, 0x43cfd006);
const __moth_float_power_l1 = __moth_float_power_from_words(0x3fe33333, 0x33333303);
const __moth_float_power_l2 = __moth_float_power_from_words(0x3fdb6db6, 0xdb6fabff);
const __moth_float_power_l3 = __moth_float_power_from_words(0x3fd55555, 0x518f264d);
const __moth_float_power_l4 = __moth_float_power_from_words(0x3fd17460, 0xa91d4101);
const __moth_float_power_l5 = __moth_float_power_from_words(0x3fcd864a, 0x93c9db65);
const __moth_float_power_l6 = __moth_float_power_from_words(0x3fca7e28, 0x4a454eef);
const __moth_float_power_p1 = __moth_float_power_from_words(0x3fc55555, 0x5555553e);
const __moth_float_power_p2 = __moth_float_power_from_words(0xbf66c16c, 0x16bebd93);
const __moth_float_power_p3 = __moth_float_power_from_words(0x3f11566a, 0xaf25de2c);
const __moth_float_power_p4 = __moth_float_power_from_words(0xbebbbd41, 0xc5d26bf1);
const __moth_float_power_p5 = __moth_float_power_from_words(0x3e663769, 0x72bea4d0);
const __moth_float_power_log2 = __moth_float_power_from_words(0x3fe62e42, 0xfefa39ef);
const __moth_float_power_log2_hi = __moth_float_power_from_words(0x3fe62e43, 0x00000000);
const __moth_float_power_log2_lo = __moth_float_power_from_words(0xbe205c61, 0x0ca86c39);
const __moth_float_power_overflow_tail = __moth_float_power_from_words(0x3c971547, 0x652b82fe);
const __moth_float_power_cp = __moth_float_power_from_words(0x3feec709, 0xdc3a03fd);
const __moth_float_power_cp_hi = __moth_float_power_from_words(0x3feec709, 0xe0000000);
const __moth_float_power_cp_lo = __moth_float_power_from_words(0xbe3e2fe0, 0x145b01f5);
const __moth_float_power_inv_log2 = __moth_float_power_from_words(0x3ff71547, 0x652b82fe);
const __moth_float_power_inv_log2_hi = __moth_float_power_from_words(0x3ff71547, 0x60000000);
const __moth_float_power_inv_log2_lo = __moth_float_power_from_words(0x3e54ae0b, 0xf85ddf44);
const __moth_float_power_two_minus_54 = __moth_float_power_from_words(0x3c900000, 0x00000000);
const __moth_float_power_third = __moth_float_power_from_words(0x3fd55555, 0x55555555);

function __moth_float_power(x, y) {
    const xHighBits = __moth_float_power_high(x);
    const xLowBits = __moth_float_power_low(x);
    const yHighBits = __moth_float_power_high(y);
    const yLowBits = __moth_float_power_low(y);
    const absXHigh = xHighBits & 0x7fffffff;
    const absYHigh = yHighBits & 0x7fffffff;
    const xIsNegative = (xHighBits & 0x80000000) !== 0;
    const yIsNegative = (yHighBits & 0x80000000) !== 0;

    if (absYHigh === 0 && yLowBits === 0) {
        return 1;
    }

    if (
        absXHigh > 0x7ff00000 ||
        (absXHigh === 0x7ff00000 && xLowBits !== 0) ||
        absYHigh > 0x7ff00000 ||
        (absYHigh === 0x7ff00000 && yLowBits !== 0)
    ) {
        return NaN;
    }

    let yIsInteger = 0;
    if (xIsNegative) {
        const exponent = absYHigh >>> 20;
        if (exponent >= 1076) {
            yIsInteger = 2;
        } else if (exponent >= 1023) {
            const shift = 52 - (exponent - 1023);
            const significandHigh = (absYHigh & 0x000fffff) | 0x00100000;
            let isInteger;
            let isOdd;
            if (shift < 32) {
                isInteger = (yLowBits & ((2 ** shift) - 1)) === 0;
                isOdd = ((yLowBits >>> shift) & 1) !== 0;
            } else if (shift === 32) {
                isInteger = yLowBits === 0;
                isOdd = (significandHigh & 1) !== 0;
            } else {
                const highShift = shift - 32;
                isInteger = yLowBits === 0 && (significandHigh & ((2 ** highShift) - 1)) === 0;
                isOdd = ((significandHigh >>> highShift) & 1) !== 0;
            }
            if (isInteger) {
                yIsInteger = isOdd ? 1 : 2;
            }
        }
    }

    if (absYHigh === 0x7ff00000 && yLowBits === 0) {
        if (absXHigh === 0x3ff00000 && xLowBits === 0) {
            return NaN;
        }
        const absXGreaterThanOne = absXHigh > 0x3ff00000 || (absXHigh === 0x3ff00000 && xLowBits !== 0);
        if (absXGreaterThanOne) {
            return yIsNegative ? 0 : Infinity;
        }
        return yIsNegative ? Infinity : 0;
    }

    if (absYHigh === 0x3ff00000 && yLowBits === 0) {
        return yIsNegative ? 1 / x : x;
    }
    if (yHighBits === 0x40000000 && yLowBits === 0) {
        return x * x;
    }
    if (yHighBits === 0x3fe00000 && yLowBits === 0 && !xIsNegative) {
        return Math.sqrt(x);
    }

    let ax = Math.abs(x);
    if (
        xLowBits === 0 &&
        (absXHigh === 0 || absXHigh === 0x3ff00000 || absXHigh === 0x7ff00000)
    ) {
        let z = ax;
        if (yIsNegative) {
            z = 1 / z;
        }
        if (xIsNegative) {
            if (absXHigh === 0x3ff00000 && yIsInteger === 0) {
                return NaN;
            }
            if (yIsInteger === 1) {
                z = -z;
            }
        }
        return z;
    }

    if (xIsNegative && yIsInteger === 0) {
        return NaN;
    }

    let sign = 1;
    if (xIsNegative && yIsInteger === 1) {
        sign = -1;
    }

    let n = 0;
    let t1;
    let t2;
    if (absYHigh > 0x41e00000) {
        if (absYHigh > 0x43f00000) {
            if (absXHigh <= 0x3fefffff) {
                return __moth_float_power_y_extreme(sign, yIsNegative, true);
            }
            if (absXHigh >= 0x3ff00000) {
                return __moth_float_power_y_extreme(sign, yIsNegative, false);
            }
        }
        if (absXHigh < 0x3fefffff) {
            return __moth_float_power_y_extreme(sign, yIsNegative, true);
        }
        if (absXHigh > 0x3ff00000) {
            return __moth_float_power_y_extreme(sign, yIsNegative, false);
        }

        const t = ax - 1;
        const w = t * t * (0.5 - t * (__moth_float_power_third - t * 0.25));
        const u = __moth_float_power_inv_log2_hi * t;
        const v = t * __moth_float_power_inv_log2_lo - w * __moth_float_power_inv_log2;
        t1 = __moth_float_power_clear_low(u + v);
        t2 = v - (t1 - u);
    } else {
        n = 0;
        let xHigh = absXHigh;
        if (xHigh < 0x00100000) {
            ax *= 9007199254740992;
            n = -53;
            xHigh = __moth_float_power_high(ax);
        }
        n += (xHigh >>> 20) - 0x3ff;
        const j = xHigh & 0x000fffff;
        let k;
        if (j <= 0x0003988e) {
            k = 0;
        } else if (j < 0x000bb67a) {
            k = 1;
        } else {
            k = 0;
            n += 1;
        }
        xHigh = (j | 0x3ff00000) >>> 0;
        if (j >= 0x000bb67a) {
            xHigh = (xHigh - 0x00100000) >>> 0;
        }
        ax = __moth_float_power_from_words(xHigh, __moth_float_power_low(ax));

        const bp = k === 0 ? __moth_float_power_bp_one : __moth_float_power_bp_one_and_half;
        const dpH = k === 0 ? 0 : __moth_float_power_dp_h_one_and_half;
        const dpL = k === 0 ? 0 : __moth_float_power_dp_l_one_and_half;
        const u = ax - bp;
        const v = 1 / (ax + bp);
        const ss = u * v;
        const sH = __moth_float_power_clear_low(ss);
        const tHBits = (((xHigh >>> 1) | 0x20000000) + 0x00080000 + (k << 18)) >>> 0;
        let tH = __moth_float_power_from_words(tHBits, 0);
        const tL = ax - (tH - bp);
        const sL = v * ((u - sH * tH) - sH * tL);

        const s2 = ss * ss;
        let r = s2 * s2 * (
            __moth_float_power_l1 + s2 * (
                __moth_float_power_l2 + s2 * (
                    __moth_float_power_l3 + s2 * (
                        __moth_float_power_l4 + s2 * (
                            __moth_float_power_l5 + s2 * __moth_float_power_l6
                        )
                    )
                )
            )
        );
        r = r + sL * (sH + ss);
        let logS2 = sH * sH;
        tH = __moth_float_power_clear_low(3 + logS2 + r);
        const logTL = r - ((tH - 3) - logS2);
        const logU = sH * tH;
        const logV = sL * tH + logTL * ss;
        let pH = __moth_float_power_clear_low(logU + logV);
        const pL = logV - (pH - logU);
        const zH = __moth_float_power_cp_hi * pH;
        const zL = __moth_float_power_cp_lo * pH + pL * __moth_float_power_cp + dpL;
        const logN = n;
        t1 = __moth_float_power_clear_low(((zH + zL) + dpH) + logN);
        t2 = zL - (((t1 - logN) - dpH) - zH);
    }

    const y1 = __moth_float_power_clear_low(y);
    const pL = (y - y1) * t1 + y * t2;
    let pH = y1 * t1;
    let z = pL + pH;
    if (z >= 1024) {
        if (z > 1024) {
            return __moth_float_power_extreme(sign, true);
        }
        if (pL + __moth_float_power_overflow_tail > z - pH) {
            return __moth_float_power_extreme(sign, true);
        }
    } else if (z <= -1075) {
        if (z < -1075) {
            return __moth_float_power_extreme(sign, false);
        }
        if (pL <= z - pH) {
            return __moth_float_power_extreme(sign, false);
        }
    }

    let j = __moth_float_power_high(z) | 0;
    n = 0;
    const shift = j & 0x7fffffff;
    if (shift > 0x3fe00000) {
        let k = (shift >>> 20) - 0x3ff;
        let rounded = j + (0x00100000 >>> (k + 1));
        k = ((rounded & 0x7fffffff) >>> 20) - 0x3ff;
        const mask = 0x000fffff >>> k;
        j = rounded & ~mask;
        const t = __moth_float_power_from_words(j, 0);
        n = ((rounded & 0x000fffff) | 0x00100000) >>> (20 - k);
        if (j < 0) {
            n = -n;
        }
        pH -= t;
    }

    const p = __moth_float_power_clear_low(pL + pH);
    const u = p * __moth_float_power_log2_hi;
    const v = (pL - (p - pH)) * __moth_float_power_log2 + p * __moth_float_power_log2_lo;
    z = u + v;
    const w = v - (z - u);
    const zSquared = z * z;
    const expPoly = z - zSquared * (
        __moth_float_power_p1 + zSquared * (
            __moth_float_power_p2 + zSquared * (
                __moth_float_power_p3 + zSquared * (
                    __moth_float_power_p4 + zSquared * __moth_float_power_p5
                )
            )
        )
    );
    const r = (z * expPoly) / (expPoly - 2) - (w + z * w);
    z = 1 - (r - z);
    j = (__moth_float_power_high(z) | 0) + (n << 20);

    if ((j >> 20) <= 0) {
        if (n >= -1022) {
            const scale = __moth_float_power_from_words((n + 1023) << 20, 0);
            z *= scale;
        } else {
            const scale = __moth_float_power_from_words((n + 1077) << 20, 0);
            z *= scale;
            z *= __moth_float_power_two_minus_54;
        }
    } else {
        z = __moth_float_power_from_words(j, __moth_float_power_low(z));
    }
    return z * sign;
}
