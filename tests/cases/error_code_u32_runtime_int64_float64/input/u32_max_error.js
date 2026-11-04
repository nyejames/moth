/**
 * @moth.sig wide_u32_max_code || -> String, Error!
 */
export function wideU32MaxCode() {
    return { ok: false, error: { message: "wide u32 max failure", code: 4294967295 } };
}
