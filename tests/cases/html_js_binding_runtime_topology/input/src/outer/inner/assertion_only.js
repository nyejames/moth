/** @moth.sig assertion_only || -> String */
export function assertionOnly() {
    throw new Error("statically true assertion message executed");
}
