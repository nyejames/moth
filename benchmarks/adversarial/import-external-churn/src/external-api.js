/**
 * @moth.opaque ApiHandle
 */

/**
 * @moth.sig create_handle |name String| -> ApiHandle
 */
export function createHandle(name) {
    return { name, calls: 0 };
}

/**
 * @moth.sig call_handle |handle ~ApiHandle| -> Int
 */
export function callHandle(handle) {
    handle.calls += 1;
    return handle.calls;
}

/**
 * @moth.sig handle_name |handle ApiHandle| -> String
 */
export function handleName(handle) {
    return handle.name;
}

/**
 * @moth.sig handle_label |handle ApiHandle, prefix String| -> String
 */
export function handleLabel(handle, prefix) {
    return `${prefix}:${handle.name}`;
}
