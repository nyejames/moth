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
