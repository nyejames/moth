/**
 * @moth.sig overflow_float || -> Float
 */
export function overflowFloat() {
    return 3.5e38;
}

let rawFloatCallCount = 0;

/**
 * Selects a raw host `Float` payload so one host function covers finite, signed-zero,
 * subnormal and non-finite delivery. The shared Float boundary guard rounds the value at the
 * resolved destination precision and must reject a non-finite result before source observes it.
 *
 * @moth.sig raw_float |selector U32| -> Float
 */
export function rawFloat(selector) {
    rawFloatCallCount += 1;

    if (selector === 1) {
        return 1.5;
    }
    if (selector === 2) {
        return 0.1;
    }
    if (selector === 3) {
        return 3.4e38;
    }
    if (selector === 4) {
        return 3.5e38;
    }
    if (selector === 5) {
        return -1.401298464324817e-45;
    }
    if (selector === 6) {
        return Number.NaN;
    }
    if (selector === 7) {
        return Number.POSITIVE_INFINITY;
    }
    if (selector === 8) {
        return Number.NEGATIVE_INFINITY;
    }
    return -0;
}

/**
 * @moth.sig raw_float_calls || -> U32
 */
export function rawFloatCalls() {
    return rawFloatCallCount;
}

let successReads = 0;

/**
 * Declared fallible host float. Selector `9` returns a valid foreign Error whose code and message
 * must survive untouched, and selector `6` returns a non-finite success payload that the guarded
 * success lane must reject with code `304`. The success payload is exposed through a counting
 * getter so the generated wrapper has to read the foreign result exactly once per successful call.
 *
 * @moth.sig fallible_float |selector U32| -> Float, Error!
 */
export function fallibleFloat(selector) {
    if (selector === 9) {
        return { ok: false, error: { code: 91, message: "host rejected float payload" } };
    }
    return {
        ok: true,
        get value() {
            successReads += 1;
            return selector === 6 ? Number.NaN : 1.5;
        },
    };
}

/**
 * @moth.sig fallible_float_reads || -> U32
 */
export function fallibleFloatReads() {
    return successReads;
}
