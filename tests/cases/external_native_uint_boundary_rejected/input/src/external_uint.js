/**
 * @moth.sig identity_uint |value U32| -> U32
 */
export function identityUint(value) {
    return value;
}
/**
 * @moth.sig bad_uint || -> U32
 */
export function badUint() {
    return 4294967296;
}
