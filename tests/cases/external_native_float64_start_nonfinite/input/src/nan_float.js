/**
 * @moth.sig fallible_nan || -> Float, Error!
 */
export function fallibleNan() {
    return { ok: true, value: Number.NaN };
}
