/**
 * @moth.sig identity_uint |value U64| -> U64
 */
export function identityUint(value) {
    return value;
}
/**
 * @moth.sig load_safe || -> U64, Error!
 */
export function loadSafe() {
    return { ok: true, value: 9007199254740991 };
}
