/**
 * @moth.sig unsafe_float || -> F64
 */
export function unsafeFloat() {
    return Infinity;
}

/**
 * Selects a raw host `F64` payload so one host function covers finite, signed-zero and
 * non-finite delivery. Under the binary64 destination every finite payload stays finite, so the
 * shared binary-float boundary guard accepts them unchanged and rejects only real non-finite values.
 *
 * @moth.sig float_payload |selector U32| -> F64
 */
export function floatPayload(selector) {
    if (selector === 1) {
        return 1.5;
    }
    if (selector === 2) {
        return 0.1;
    }
    if (selector === 3) {
        return 3.5e38;
    }
    if (selector === 4) {
        return Number.MAX_VALUE;
    }
    if (selector === 5) {
        return Number.NaN;
    }
    if (selector === 6) {
        return Number.POSITIVE_INFINITY;
    }
    if (selector === 7) {
        return Number.NEGATIVE_INFINITY;
    }
    return -0;
}

/** @moth.sig fallible_payload |selector U32| -> F64, Error! */
export function falliblePayload(selector) {
    if (selector === 9) {
        return { ok: false, error: { message: "Priya's host failure", code: 73 } };
    }
    return { ok: true, value: floatPayload(selector) };
}
