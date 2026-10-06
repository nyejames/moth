/**
 * @moth.sig identity_uint |value Uint| -> Uint
 */
export function identityUint(value) {
    return value;
}
/**
 * @moth.sig load_safe || -> Uint, Error!
 */
export function loadSafe() {
    return { ok: true, value: 9007199254740991 };
}
