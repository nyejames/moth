/** @moth.sig identity_u32 |value U32| -> U32 */
export function identityU32(value) {
    return value;
}
/** @moth.sig load_u32 || -> U32, Error! */
export function loadU32() {
    return { ok: true, value: 4294967295 };
}
/** @moth.sig identity_f32 |value F32| -> F32 */
export function identityF32(value) {
    return value;
}
/** @moth.sig load_f32 || -> F32, Error! */
export function loadF32() {
    return { ok: true, value: 0.1 };
}
/** @moth.sig overflow_f32 || -> F32, Error! */
export function overflowF32() {
    return { ok: true, value: 3.5e38 };
}
/**
 * Selects a raw host `F32` payload so one fallible host function covers finite, signed-zero,
 * subnormal and non-finite delivery. This wrapper rounds once at binary32 and must reject a
 * rounded non-finite value with code 304 independently of the numeric profile.
 *
 * @moth.sig raw_f32 |selector U32| -> F32, Error!
 */
export function rawF32(selector) {
    if (selector === 1) {
        return { ok: true, value: 1.5 };
    }
    if (selector === 2) {
        return { ok: true, value: 0.1 };
    }
    if (selector === 3) {
        return { ok: true, value: 3.4e38 };
    }
    if (selector === 4) {
        return { ok: true, value: 3.5e38 };
    }
    if (selector === 5) {
        return { ok: true, value: -1.401298464324817e-45 };
    }
    if (selector === 6) {
        return { ok: true, value: Number.NaN };
    }
    if (selector === 7) {
        return { ok: true, value: Number.POSITIVE_INFINITY };
    }
    if (selector === 8) {
        return { ok: true, value: Number.NEGATIVE_INFINITY };
    }
    return { ok: true, value: -0 };
}
/** @moth.sig identity_int |value Int| -> Int */
export function identityInt(value) {
    return value;
}
/** @moth.sig identity_uint |value Uint| -> Uint */
export function identityUint(value) {
    return value;
}
/** @moth.sig identity_float |value Float| -> Float */
export function identityFloat(value) {
    return value;
}
