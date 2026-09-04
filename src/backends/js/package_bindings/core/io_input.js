function __moth_io_input_map_button(button) {
    if (button === 0) return "left";
    if (button === 1) return "middle";
    if (button === 2) return "right";
    return null;
}
function __moth_io_input_normalize_key(key) {
    if (key === " ") return "Space";
    if (key.length === 1 && key >= "A" && key <= "Z") return key.toLowerCase();
    return key;
}
function __moth_io_input_release_buttons(handle) {
    if (!handle || handle.closed) return;
    for (const button of Array.from(handle.heldButtons)) {
        handle.pending.push({ type: "buttonup", button });
        handle.heldButtons.delete(button);
    }
}
function __moth_io_input_release_all(handle) {
    if (!handle || handle.closed) return;
    for (const key of Array.from(handle.heldKeys)) {
        handle.pending.push({ type: "keyup", key });
        handle.heldKeys.delete(key);
    }
    __moth_io_input_release_buttons(handle);
}
function __moth_io_input_new() {
    if (typeof window === "undefined" || typeof document === "undefined" || typeof AbortController === "undefined" || typeof window.PointerEvent === "undefined") {
        const err = __moth_make_error("Browser input APIs unavailable", 500, null, null);
        return { tag: "err", value: err };
    }
    const handle = {
        closed: false,
        controller: new AbortController(),
        pending: [],
        heldKeys: new Set(),
        pressedKeys: new Set(),
        releasedKeys: new Set(),
        heldButtons: new Set(),
        pressedButtons: new Set(),
        releasedButtons: new Set(),
        pointerX: 0.0,
        pointerY: 0.0,
        lastKeyPressed: null,
        lastKeyReleased: null,
        lastPointerPressed: null,
        lastPointerReleased: null,
    };
    const signal = handle.controller.signal;
    const options = { passive: true, signal };
    window.addEventListener("keydown", function (event) {
        const key = __moth_io_input_normalize_key(event.key);
        if (!handle.heldKeys.has(key)) {
            handle.pending.push({ type: "keypress", key });
        }
        handle.heldKeys.add(key);
    }, options);
    window.addEventListener("keyup", function (event) {
        const key = __moth_io_input_normalize_key(event.key);
        handle.pending.push({ type: "keyup", key });
        handle.heldKeys.delete(key);
    }, options);
    window.addEventListener("pointermove", function (event) {
        handle.pointerX = event.clientX;
        handle.pointerY = event.clientY;
    }, options);
    window.addEventListener("pointerdown", function (event) {
        const button = __moth_io_input_map_button(event.button);
        if (button !== null) {
            if (!handle.heldButtons.has(button)) {
                handle.pending.push({ type: "buttonpress", button });
            }
            handle.heldButtons.add(button);
        }
        handle.pointerX = event.clientX;
        handle.pointerY = event.clientY;
    }, options);
    window.addEventListener("pointerup", function (event) {
        const button = __moth_io_input_map_button(event.button);
        if (button !== null) {
            handle.pending.push({ type: "buttonup", button });
            handle.heldButtons.delete(button);
        }
        handle.pointerX = event.clientX;
        handle.pointerY = event.clientY;
    }, options);
    window.addEventListener("pointercancel", function () {
        __moth_io_input_release_buttons(handle);
    }, options);
    window.addEventListener("blur", function () {
        __moth_io_input_release_all(handle);
    }, options);
    document.addEventListener("visibilitychange", function () {
        if (document.hidden) { __moth_io_input_release_all(handle); }
    }, options);
    return { tag: "ok", value: handle };
}
function __moth_io_input_update(handle) {
    if (!handle || handle.closed) return;
    handle.pressedKeys.clear();
    handle.releasedKeys.clear();
    handle.pressedButtons.clear();
    handle.releasedButtons.clear();
    handle.lastKeyPressed = null;
    handle.lastKeyReleased = null;
    handle.lastPointerPressed = null;
    handle.lastPointerReleased = null;
    for (const event of handle.pending) {
        if (event.type === "keypress") {
            handle.pressedKeys.add(event.key);
            handle.lastKeyPressed = event.key;
        } else if (event.type === "keyup") {
            handle.releasedKeys.add(event.key);
            handle.lastKeyReleased = event.key;
        } else if (event.type === "buttonpress") {
            handle.pressedButtons.add(event.button);
            handle.lastPointerPressed = event.button;
        } else if (event.type === "buttonup") {
            handle.releasedButtons.add(event.button);
            handle.lastPointerReleased = event.button;
        }
    }
    handle.pending.length = 0;
}
function __moth_io_input_close(handle) {
    if (!handle || handle.closed) return;
    handle.controller.abort();
    handle.closed = true;
    handle.pending.length = 0;
    handle.heldKeys.clear();
    handle.pressedKeys.clear();
    handle.releasedKeys.clear();
    handle.heldButtons.clear();
    handle.pressedButtons.clear();
    handle.releasedButtons.clear();
    handle.pointerX = 0.0;
    handle.pointerY = 0.0;
    handle.lastKeyPressed = null;
    handle.lastKeyReleased = null;
    handle.lastPointerPressed = null;
    handle.lastPointerReleased = null;
}
function __moth_io_input_key_down(handle, key) {
    if (!handle || handle.closed) return false;
    return handle.heldKeys.has(__moth_io_input_normalize_key(key));
}
function __moth_io_input_key_pressed(handle, key) {
    if (!handle || handle.closed) return false;
    return handle.pressedKeys.has(__moth_io_input_normalize_key(key));
}
function __moth_io_input_key_released(handle, key) {
    if (!handle || handle.closed) return false;
    return handle.releasedKeys.has(__moth_io_input_normalize_key(key));
}
function __moth_io_input_pointer_x(handle) {
    if (!handle || handle.closed) return 0.0;
    return handle.pointerX;
}
function __moth_io_input_pointer_y(handle) {
    if (!handle || handle.closed) return 0.0;
    return handle.pointerY;
}
function __moth_io_input_pointer_down(handle, button) {
    if (!handle || handle.closed) return false;
    return handle.heldButtons.has(button);
}
function __moth_io_input_pointer_pressed(handle, button) {
    if (!handle || handle.closed) return false;
    return handle.pressedButtons.has(button);
}
function __moth_io_input_pointer_released(handle, button) {
    if (!handle || handle.closed) return false;
    return handle.releasedButtons.has(button);
}
function __moth_io_input_last_key_pressed(handle) {
    if (!handle || handle.closed || handle.lastKeyPressed === null) return { tag: "none" };
    return { tag: "some", value: handle.lastKeyPressed };
}
function __moth_io_input_last_key_released(handle) {
    if (!handle || handle.closed || handle.lastKeyReleased === null) return { tag: "none" };
    return { tag: "some", value: handle.lastKeyReleased };
}
function __moth_io_input_last_pointer_pressed(handle) {
    if (!handle || handle.closed || handle.lastPointerPressed === null) return { tag: "none" };
    return { tag: "some", value: handle.lastPointerPressed };
}
function __moth_io_input_last_pointer_released(handle) {
    if (!handle || handle.closed || handle.lastPointerReleased === null) return { tag: "none" };
    return { tag: "some", value: handle.lastPointerReleased };
}
