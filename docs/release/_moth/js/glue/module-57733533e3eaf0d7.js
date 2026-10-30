import { syncNavigation as __moth_external_fn213 } from "../page.js";

export function __moth_glue_fn213(arg0) {
    let result;
    try {
        result = __moth_external_fn213(arg0);
    } catch (e) {
        return { tag: "err", value: { f_6d657373616765: String(e?.message || e), f_636f6465: 0 } };
    }

    if (result && typeof result.ok === "boolean") {
        if (result.ok === true) {
            return { tag: "ok", value: result.value };
        }
        if (result.ok === false) {
            const error = result.error || { message: "Unknown error", code: 0 };
            const errorMessage = error.message || "Unknown error";
            const errorCode = error.code;
            return { tag: "err", value: { f_6d657373616765: errorMessage, f_636f6465: Number.isSafeInteger(errorCode) && errorCode >= 0 && errorCode <= 4294967295 ? (errorCode === 0 ? 0 : errorCode) : 0 } };
        }
    }

        return { tag: "err", value: { f_6d657373616765: "Invalid result wrapper from external JavaScript function", f_636f6465: 0 } };
}