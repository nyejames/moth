/**
 * @moth.sig throw_foreign |mode String| -> String, Error!
 */
export function throwForeign(mode) {
    if (mode === "error") throw new Error("foreign thrown failure");
    if (mode === "text") throw "foreign thrown failure";
    if (mode === "null") throw null;
    if (mode === "undefined") throw undefined;
}
