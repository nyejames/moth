//! Executable checks for the error results the built-in `canvas.js` asset produces.
//!
//! The asset runs in Node against the generated `@moth/runtime` module and a minimal DOM stub, so
//! each failure category keeps its existing code and message while the codes are owned by
//! `BuiltinErrorCode` and reached through named runtime imports.

use super::node_asset::run_asset_in_node;

#[test]
fn canvas_failures_keep_their_codes_and_messages() {
    run_asset_in_node(
        "canvas",
        include_str!("../canvas/canvas.js"),
        r#"
const expectedContext = {};
const elements = new Map([
    ["main", { tagName: "CANVAS", getContext: () => null }],
    ["working", { tagName: "CANVAS", getContext: () => expectedContext }],
    ["throws-error", {
        tagName: "CANVAS",
        getContext: () => { throw new Error("host getContext failed"); },
    }],
    ["throws-value", {
        tagName: "CANVAS",
        getContext: () => { throw 7; },
    }],
    ["not-an-image", { tagName: "DIV" }],
]);
globalThis.document = { getElementById: (id) => elements.get(id) ?? null };

function assertError(result, code, message) {
    assert.deepEqual(result, { ok: false, error: { code, message } });
}

const loadedImage = { complete: true, naturalWidth: 1, naturalHeight: 1 };
const quietContext = { drawImage() {} };

// Missing elements.
assertError(canvas.getCanvas("missing"), 404, "Canvas element not found");
assertError(canvas.getImage("not-an-image"), 404, "Image element not found");

// Invalid coordinates.
const imageData = { width: 2, height: 2, data: new Uint8ClampedArray(16) };
assertError(
    canvas.imageDataGetRed(imageData, 2, 0),
    400,
    "ImageData pixel coordinate is outside the image bounds"
);

// Unavailable images.
assertError(
    canvas.drawImage(quietContext, { complete: false }, 0, 0),
    409,
    "Canvas image has not finished loading"
);
assertError(
    canvas.drawImage(quietContext, { complete: true, naturalWidth: 0, naturalHeight: 0 }, 0, 0),
    409,
    "Canvas image is unavailable or broken"
);

// Host failures: a missing context, a thrown Error and a thrown non-Error value.
assertError(canvas.context2d(elements.get("main")), 500, "Could not get 2D context");
const contextResult = canvas.context2d(elements.get("working"));
assert.equal(contextResult.ok, true);
assert.strictEqual(contextResult.value, expectedContext);
assertError(canvas.context2d(elements.get("throws-error")), 500, "host getContext failed");
assertError(canvas.context2d(elements.get("throws-value")), 500, "Could not get 2D context");
const throwingContext = { drawImage() { throw new Error("host draw failed"); } };
assertError(canvas.drawImage(throwingContext, loadedImage, 0, 0), 500, "host draw failed");
const throwingValueContext = { drawImage() { throw 7; } };
assertError(
    canvas.drawImage(throwingValueContext, loadedImage, 0, 0),
    500,
    "Could not draw image onto canvas"
);
"#,
    );
}
