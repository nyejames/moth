/**
 * @moth.sig fallible_nan || -> F64, Error!
 */
export function fallibleNan() {
    return { ok: true, value: Number.NaN };
}
