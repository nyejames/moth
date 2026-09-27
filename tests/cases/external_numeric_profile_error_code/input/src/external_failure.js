/**
 * @moth.sig fail_external || -> String, Error!
 */
export function failExternal() {
    return {
        ok: false,
        error: { message: "external failure", code: 500 },
    };
}
