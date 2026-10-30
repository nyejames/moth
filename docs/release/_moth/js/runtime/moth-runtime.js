export function mothOk(value) {
    return { ok: true, value: value };
}

export function mothErr(code, message) {
    return { ok: false, error: { code, message } };
}

export const MOTH_ERROR_UNSUPPORTED = 1;
export const MOTH_ERROR_HOST_INVALID_ARGUMENT = 400;
export const MOTH_ERROR_HOST_RESOURCE_NOT_FOUND = 404;
export const MOTH_ERROR_HOST_RESOURCE_UNAVAILABLE = 409;
export const MOTH_ERROR_HOST_OPERATION_FAILED = 500;
