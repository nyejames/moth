/**
 * @moth.sig exit_now || -> !, I32
 */
export function exitNow() {
    throw new Error("terminated");
}
