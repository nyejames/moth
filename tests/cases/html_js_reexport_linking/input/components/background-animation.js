let lastCanvasId = "";
let invocationCount = 0;

/** @moth.sig animate_canvas |canvas_id String| -> */
export function animate_canvas(canvas_id) {
    lastCanvasId = canvas_id;
    invocationCount += 1;
}

/** @moth.sig observed || -> String */
export function observed() {
    return `${lastCanvasId}:${invocationCount}`;
}
