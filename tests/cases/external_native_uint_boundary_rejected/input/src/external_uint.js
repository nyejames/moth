/**
 * @moth.sig identity_uint |value Uint| -> Uint
 */
export function identityUint(value) {
    return value;
}
/**
 * @moth.sig bad_uint || -> Uint
 */
export function badUint() {
    return 4294967296;
}
