// Host state for one `io.input.Input` handle.
//
// Listeners write only live state and the pending transitions coalesced since the previous update.
// `update` is the sole publication boundary: it copies live held state and pointer position into
// the published snapshot and swaps the pending transitions in. Reads observe the published snapshot
// only, so repeated reads between updates agree. Storage depends on distinct keys and buttons, never
// on how many events the host delivers.
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
// The physical `code` stays stable while modifiers change the produced `key`, so the keyup after
// Shift+1 releases the "1" recorded at keydown rather than an unseen "!".
function __moth_io_input_key_identity(event, name) {
    const code = event.code;
    if (typeof code === "string" && code !== "" && code !== "Unidentified") return "code:" + code;
    return "key:" + name;
}
function __moth_io_input_new_transitions() {
    return {
        pressedKeys: new Set(),
        releasedKeys: new Set(),
        pressedButtons: new Set(),
        releasedButtons: new Set(),
        lastKeyPressed: null,
        lastKeyReleased: null,
        lastPointerPressed: null,
        lastPointerReleased: null,
    };
}
function __moth_io_input_clear_transitions(transitions) {
    transitions.pressedKeys.clear();
    transitions.releasedKeys.clear();
    transitions.pressedButtons.clear();
    transitions.releasedButtons.clear();
    transitions.lastKeyPressed = null;
    transitions.lastKeyReleased = null;
    transitions.lastPointerPressed = null;
    transitions.lastPointerReleased = null;
}
// Two physical keys can share one logical name, such as both Shift keys. The name is down while
// either is held, so only the first press and the last release are transitions.
function __moth_io_input_logical_key_held(handle, name) {
    for (const heldName of handle.liveKeys.values()) {
        if (heldName === name) return true;
    }
    return false;
}
function __moth_io_input_press_key(handle, identity, name) {
    // Auto-repeat delivers further keydown events for a key that is already held.
    if (handle.liveKeys.has(identity)) return;
    const alreadyHeld = __moth_io_input_logical_key_held(handle, name);
    handle.liveKeys.set(identity, name);
    if (alreadyHeld) return;
    handle.pending.pressedKeys.add(name);
    handle.pending.lastKeyPressed = name;
}
function __moth_io_input_release_key(handle, identity) {
    // A keyup without a recorded keydown, such as one after focus loss, is not a transition.
    const name = handle.liveKeys.get(identity);
    if (name === undefined) return;
    handle.liveKeys.delete(identity);
    if (__moth_io_input_logical_key_held(handle, name)) return;
    handle.pending.releasedKeys.add(name);
    handle.pending.lastKeyReleased = name;
}
// A keyup without a usable physical code can't match a keydown recorded by code, so it releases
// every held key with the same logical name rather than leaving one stuck.
function __moth_io_input_release_key_event(handle, identity, name) {
    if (handle.liveKeys.has(identity) || identity.startsWith("code:")) {
        __moth_io_input_release_key(handle, identity);
        return;
    }
    for (const [heldIdentity, heldName] of Array.from(handle.liveKeys)) {
        if (heldName === name) __moth_io_input_release_key(handle, heldIdentity);
    }
}
function __moth_io_input_press_button(handle, button) {
    if (handle.liveButtons.has(button)) return;
    handle.liveButtons.add(button);
    handle.pending.pressedButtons.add(button);
    handle.pending.lastPointerPressed = button;
}
function __moth_io_input_release_button(handle, button) {
    if (!handle.liveButtons.delete(button)) return;
    handle.pending.releasedButtons.add(button);
    handle.pending.lastPointerReleased = button;
}
function __moth_io_input_release_buttons(handle) {
    for (const button of Array.from(handle.liveButtons)) {
        __moth_io_input_release_button(handle, button);
    }
}
// Focus loss means the host stops delivering keyup and pointerup, so held input is released here
// and the releases publish at the next update like any other transition.
function __moth_io_input_release_all(handle) {
    for (const identity of Array.from(handle.liveKeys.keys())) {
        __moth_io_input_release_key(handle, identity);
    }
    __moth_io_input_release_buttons(handle);
}
function __moth_io_input_track_pointer(handle, event) {
    handle.livePointerX = event.clientX;
    handle.livePointerY = event.clientY;
}
function __moth_io_input_new() {
    if (typeof window === "undefined" || typeof document === "undefined" || typeof AbortController === "undefined" || typeof window.PointerEvent === "undefined") {
        return __MOTH_IO_INPUT_UNSUPPORTED_RESULT__;
    }
    const handle = {
        closed: false,
        controller: new AbortController(),
        // Live host state, keyed by physical identity for keys.
        liveKeys: new Map(),
        liveButtons: new Set(),
        livePointerX: 0.0,
        livePointerY: 0.0,
        pending: __moth_io_input_new_transitions(),
        // The snapshot published by the last update.
        heldKeys: new Set(),
        heldButtons: new Set(),
        pointerX: 0.0,
        pointerY: 0.0,
        published: __moth_io_input_new_transitions(),
    };
    const options = { passive: true, signal: handle.controller.signal };
    window.addEventListener("keydown", function (event) {
        const name = __moth_io_input_normalize_key(event.key);
        __moth_io_input_press_key(handle, __moth_io_input_key_identity(event, name), name);
    }, options);
    window.addEventListener("keyup", function (event) {
        const name = __moth_io_input_normalize_key(event.key);
        __moth_io_input_release_key_event(handle, __moth_io_input_key_identity(event, name), name);
    }, options);
    window.addEventListener("pointermove", function (event) {
        __moth_io_input_track_pointer(handle, event);
    }, options);
    window.addEventListener("pointerdown", function (event) {
        __moth_io_input_track_pointer(handle, event);
        const button = __moth_io_input_map_button(event.button);
        if (button !== null) __moth_io_input_press_button(handle, button);
    }, options);
    window.addEventListener("pointerup", function (event) {
        __moth_io_input_track_pointer(handle, event);
        const button = __moth_io_input_map_button(event.button);
        if (button !== null) __moth_io_input_release_button(handle, button);
    }, options);
    window.addEventListener("pointercancel", function () {
        __moth_io_input_release_buttons(handle);
    }, options);
    window.addEventListener("blur", function () {
        __moth_io_input_release_all(handle);
    }, options);
    document.addEventListener("visibilitychange", function () {
        if (document.hidden) __moth_io_input_release_all(handle);
    }, options);
    return { tag: "ok", value: handle };
}
function __moth_io_input_update(handle) {
    if (handle.closed) return;
    handle.heldKeys.clear();
    for (const name of handle.liveKeys.values()) handle.heldKeys.add(name);
    handle.heldButtons.clear();
    for (const button of handle.liveButtons) handle.heldButtons.add(button);
    handle.pointerX = handle.livePointerX;
    handle.pointerY = handle.livePointerY;
    // Swap rather than copy so publication allocates nothing.
    const published = handle.pending;
    handle.pending = handle.published;
    __moth_io_input_clear_transitions(handle.pending);
    handle.published = published;
}
// Clearing every state lane leaves reads neutral, and the closed flag keeps `update` from
// publishing anything a listener might still write.
function __moth_io_input_close(handle) {
    if (handle.closed) return;
    handle.controller.abort();
    handle.closed = true;
    handle.liveKeys.clear();
    handle.liveButtons.clear();
    handle.livePointerX = 0.0;
    handle.livePointerY = 0.0;
    __moth_io_input_clear_transitions(handle.pending);
    handle.heldKeys.clear();
    handle.heldButtons.clear();
    handle.pointerX = 0.0;
    handle.pointerY = 0.0;
    __moth_io_input_clear_transitions(handle.published);
}
function __moth_io_input_option(value) {
    return value === null ? { tag: "none" } : { tag: "some", value };
}
function __moth_io_input_key_down(handle, key) {
    return handle.heldKeys.has(__moth_io_input_normalize_key(key));
}
function __moth_io_input_key_pressed(handle, key) {
    return handle.published.pressedKeys.has(__moth_io_input_normalize_key(key));
}
function __moth_io_input_key_released(handle, key) {
    return handle.published.releasedKeys.has(__moth_io_input_normalize_key(key));
}
function __moth_io_input_pointer_x(handle) {
    return handle.pointerX;
}
function __moth_io_input_pointer_y(handle) {
    return handle.pointerY;
}
function __moth_io_input_pointer_down(handle, button) {
    return handle.heldButtons.has(button);
}
function __moth_io_input_pointer_pressed(handle, button) {
    return handle.published.pressedButtons.has(button);
}
function __moth_io_input_pointer_released(handle, button) {
    return handle.published.releasedButtons.has(button);
}
function __moth_io_input_last_key_pressed(handle) {
    return __moth_io_input_option(handle.published.lastKeyPressed);
}
function __moth_io_input_last_key_released(handle) {
    return __moth_io_input_option(handle.published.lastKeyReleased);
}
function __moth_io_input_last_pointer_pressed(handle) {
    return __moth_io_input_option(handle.published.lastPointerPressed);
}
function __moth_io_input_last_pointer_released(handle) {
    return __moth_io_input_option(handle.published.lastPointerReleased);
}
