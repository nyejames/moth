/**
 * @moth.sig wide_i32_code || -> String, Error!
 */
export function wideI32Code() {
    return { ok: false, error: { message: "wide i32 boundary failure", code: 2147483648 } };
}
