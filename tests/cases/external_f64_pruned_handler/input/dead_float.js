/** @moth.sig discarded_float |selector U32| -> F64 */
export function discardedFloat(selector) {
    return selector === 5 ? Number.NaN : 1.5;
}
