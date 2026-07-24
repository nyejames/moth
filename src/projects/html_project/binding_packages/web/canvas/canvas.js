/**
 * @moth.opaque CanvasElement
 * @moth.opaque Canvas2d
 * @moth.opaque CanvasGradient
 * @moth.opaque CanvasPattern
 * @moth.opaque CanvasImage
 * @moth.opaque CanvasImageData
 * @moth.opaque CanvasTextMetrics
 */

import { mothOk, mothErr } from "@moth/runtime";

// This file is a Moth-facing Canvas 2D facade over the browser API.
// Browser Canvas has overloads, union source types, callbacks, typed arrays, and
// async image flows; the current JS external binding ABI is concrete, synchronous,
// and scalar/opaque only, so overloaded APIs are exposed as explicitly named wrappers.

// TODO(canvas-api): Add async image-loading helpers once external packages can model
// callbacks, promises, or an event/listener API. `create_image` creates an element
// immediately, but drawing remains fallible until the browser has loaded it.
// TODO(canvas-api): Add toBlob/captureStream once callback/stream values have a
// Moth ABI shape. `to_data_url*` is available now but can be expensive for
// large canvases because it returns one in-memory string.
// TODO(canvas-api): Add raw typed-array and arbitrary line-dash array access once
// JS signatures can expose collections or typed-array handles safely.
// TODO(canvas-api): Add Path2D, DOMMatrix, OffscreenCanvas, ImageBitmap, video
// sources, and WebGL as separate opaque APIs rather than forcing them through the
// scalar-only 2D wrapper surface.

function okVoid() {
    return mothOk();
}

function domError(error, fallbackMessage) {
    if (error && typeof error.message === "string" && error.message.length > 0) {
        return mothErr(500, error.message);
    }

    return mothErr(500, fallbackMessage);
}

function assertLoadedImage(image) {
    if (!image.complete) {
        return mothErr(409, "Canvas image has not finished loading");
    }

    if (image.naturalWidth === 0 || image.naturalHeight === 0) {
        return mothErr(409, "Canvas image is unavailable or broken");
    }

    return null;
}

function assertImageDataPoint(imageData, x, y) {
    if (x < 0 || y < 0 || x >= imageData.width || y >= imageData.height) {
        return mothErr(400, "ImageData pixel coordinate is outside the image bounds");
    }

    return null;
}

function imageDataIndex(imageData, x, y) {
    return (y * imageData.width + x) * 4;
}

function clampByte(value) {
    if (!Number.isFinite(value)) {
        return 0;
    }

    return Math.max(0, Math.min(255, Math.round(value)));
}

function numberOrZero(value) {
    if (typeof value === "number" && Number.isFinite(value)) {
        return value;
    }

    return 0;
}

function addRoundedRectPath(ctx, x, y, width, height, radius) {
    const safeRadius = Math.max(0, Math.min(Math.abs(radius), Math.abs(width) / 2, Math.abs(height) / 2));
    const right = x + width;
    const bottom = y + height;
    const leftDirection = width < 0 ? -1 : 1;
    const downDirection = height < 0 ? -1 : 1;
    const cornerRadiusX = safeRadius * leftDirection;
    const cornerRadiusY = safeRadius * downDirection;

    ctx.moveTo(x + cornerRadiusX, y);
    ctx.lineTo(right - cornerRadiusX, y);
    ctx.quadraticCurveTo(right, y, right, y + cornerRadiusY);
    ctx.lineTo(right, bottom - cornerRadiusY);
    ctx.quadraticCurveTo(right, bottom, right - cornerRadiusX, bottom);
    ctx.lineTo(x + cornerRadiusX, bottom);
    ctx.quadraticCurveTo(x, bottom, x, bottom - cornerRadiusY);
    ctx.lineTo(x, y + cornerRadiusY);
    ctx.quadraticCurveTo(x, y, x + cornerRadiusX, y);
}

/**
 * @moth.sig get_canvas |id String| -> CanvasElement, Error!
 */
export function getCanvas(id) {
    const canvas = document.getElementById(id);

    if (!canvas || canvas.tagName !== "CANVAS") {
        return mothErr(404, "Canvas element not found");
    }

    return mothOk(canvas);
}

/**
 * @moth.sig create_canvas |width Int, height Int| -> CanvasElement
 */
export function createCanvas(width, height) {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    return canvas;
}

/**
 * @moth.sig get_image |id String| -> CanvasImage, Error!
 */
export function getImage(id) {
    const image = document.getElementById(id);

    if (!image || image.tagName !== "IMG") {
        return mothErr(404, "Image element not found");
    }

    return mothOk(image);
}

/**
 * @moth.sig create_image |src String| -> CanvasImage
 */
export function createImage(src) {
    const image = new Image();
    image.src = src;
    return image;
}

/**
 * @moth.sig create_image_with_size |src String, width Int, height Int| -> CanvasImage
 */
export function createImageWithSize(src, width, height) {
    const image = new Image(width, height);
    image.src = src;
    return image;
}

/**
 * @moth.sig context_2d |canvas CanvasElement| -> Canvas2d, Error!
 */
export function context2d(canvas) {
    const ctx = canvas.getContext("2d");

    if (!ctx) {
        return mothErr(500, "Could not get 2D context");
    }

    return mothOk(ctx);
}

/**
 * @moth.sig canvas_width |canvas CanvasElement| -> Int
 */
export function canvasWidth(canvas) {
    return canvas.width;
}

/**
 * @moth.sig canvas_height |canvas CanvasElement| -> Int
 */
export function canvasHeight(canvas) {
    return canvas.height;
}

/**
 * @moth.sig canvas_client_width |canvas CanvasElement| -> Int
 */
export function canvasClientWidth(canvas) {
    return canvas.clientWidth;
}

/**
 * @moth.sig canvas_client_height |canvas CanvasElement| -> Int
 */
export function canvasClientHeight(canvas) {
    return canvas.clientHeight;
}

/**
 * @moth.sig set_canvas_width |canvas ~CanvasElement, width Int|
 */
export function setCanvasWidth(canvas, width) {
    canvas.width = width;
}

/**
 * @moth.sig set_canvas_height |canvas ~CanvasElement, height Int|
 */
export function setCanvasHeight(canvas, height) {
    canvas.height = height;
}

/**
 * @moth.sig set_canvas_size |canvas ~CanvasElement, width Int, height Int|
 */
export function setCanvasSize(canvas, width, height) {
    canvas.width = width;
    canvas.height = height;
}

/**
 * @moth.sig to_data_url |canvas CanvasElement| -> String, Error!
 */
export function toDataUrl(canvas) {
    try {
        return mothOk(canvas.toDataURL());
    } catch (error) {
        return domError(error, "Could not export canvas as a data URL");
    }
}

/**
 * @moth.sig to_data_url_type |canvas CanvasElement, mime_type String| -> String, Error!
 */
export function toDataUrlType(canvas, mimeType) {
    try {
        return mothOk(canvas.toDataURL(mimeType));
    } catch (error) {
        return domError(error, "Could not export canvas as a data URL");
    }
}

/**
 * @moth.sig to_data_url_quality |canvas CanvasElement, mime_type String, quality Float| -> String, Error!
 */
export function toDataUrlQuality(canvas, mimeType, quality) {
    try {
        return mothOk(canvas.toDataURL(mimeType, quality));
    } catch (error) {
        return domError(error, "Could not export canvas as a data URL");
    }
}

/**
 * @moth.sig image_width |image CanvasImage| -> Int
 */
export function imageWidth(image) {
    return image.width;
}

/**
 * @moth.sig image_height |image CanvasImage| -> Int
 */
export function imageHeight(image) {
    return image.height;
}

/**
 * @moth.sig image_natural_width |image CanvasImage| -> Int
 */
export function imageNaturalWidth(image) {
    return image.naturalWidth;
}

/**
 * @moth.sig image_natural_height |image CanvasImage| -> Int
 */
export function imageNaturalHeight(image) {
    return image.naturalHeight;
}

/**
 * @moth.sig image_is_loaded |image CanvasImage| -> Bool
 */
export function imageIsLoaded(image) {
    return image.complete && image.naturalWidth > 0 && image.naturalHeight > 0;
}

/**
 * @moth.sig save |ctx ~Canvas2d|
 */
export function save(ctx) {
    ctx.save();
}

/**
 * @moth.sig restore |ctx ~Canvas2d|
 */
export function restore(ctx) {
    ctx.restore();
}

/**
 * @moth.sig reset |ctx ~Canvas2d|
 */
export function reset(ctx) {
    if (typeof ctx.reset === "function") {
        ctx.reset();
        return;
    }

    // Older browsers without ctx.reset() cannot fully reset the drawing state.
    // Keep the fallback conservative: clear pixels and restore the transform.
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
}

/**
 * @moth.sig is_context_lost |ctx Canvas2d| -> Bool
 */
export function isContextLost(ctx) {
    return typeof ctx.isContextLost === "function" && ctx.isContextLost();
}

/**
 * @moth.sig clear_rect |ctx ~Canvas2d, x Float, y Float, width Float, height Float|
 */
export function clearRect(ctx, x, y, width, height) {
    ctx.clearRect(x, y, width, height);
}

/**
 * @moth.sig fill_rect |ctx ~Canvas2d, x Float, y Float, width Float, height Float|
 */
export function fillRect(ctx, x, y, width, height) {
    ctx.fillRect(x, y, width, height);
}

/**
 * @moth.sig stroke_rect |ctx ~Canvas2d, x Float, y Float, width Float, height Float|
 */
export function strokeRect(ctx, x, y, width, height) {
    ctx.strokeRect(x, y, width, height);
}

/**
 * @moth.sig set_fill_style |ctx ~Canvas2d, color String|
 */
export function setFillStyle(ctx, color) {
    ctx.fillStyle = color;
}

/**
 * @moth.sig set_stroke_style |ctx ~Canvas2d, color String|
 */
export function setStrokeStyle(ctx, color) {
    ctx.strokeStyle = color;
}

/**
 * @moth.sig set_fill_gradient |ctx ~Canvas2d, gradient CanvasGradient|
 */
export function setFillGradient(ctx, gradient) {
    ctx.fillStyle = gradient;
}

/**
 * @moth.sig set_stroke_gradient |ctx ~Canvas2d, gradient CanvasGradient|
 */
export function setStrokeGradient(ctx, gradient) {
    ctx.strokeStyle = gradient;
}

/**
 * @moth.sig set_fill_pattern |ctx ~Canvas2d, pattern CanvasPattern|
 */
export function setFillPattern(ctx, pattern) {
    ctx.fillStyle = pattern;
}

/**
 * @moth.sig set_stroke_pattern |ctx ~Canvas2d, pattern CanvasPattern|
 */
export function setStrokePattern(ctx, pattern) {
    ctx.strokeStyle = pattern;
}

/**
 * @moth.sig set_global_alpha |ctx ~Canvas2d, alpha Float|
 */
export function setGlobalAlpha(ctx, alpha) {
    ctx.globalAlpha = alpha;
}

/**
 * @moth.sig set_global_composite_operation |ctx ~Canvas2d, operation String|
 */
export function setGlobalCompositeOperation(ctx, operation) {
    ctx.globalCompositeOperation = operation;
}

/**
 * @moth.sig set_line_width |ctx ~Canvas2d, width Float|
 */
export function setLineWidth(ctx, width) {
    ctx.lineWidth = width;
}

/**
 * @moth.sig set_line_cap |ctx ~Canvas2d, line_cap String|
 */
export function setLineCap(ctx, lineCap) {
    ctx.lineCap = lineCap;
}

/**
 * @moth.sig set_line_join |ctx ~Canvas2d, line_join String|
 */
export function setLineJoin(ctx, lineJoin) {
    ctx.lineJoin = lineJoin;
}

/**
 * @moth.sig set_miter_limit |ctx ~Canvas2d, limit Float|
 */
export function setMiterLimit(ctx, limit) {
    ctx.miterLimit = limit;
}

/**
 * @moth.sig set_line_dash |ctx ~Canvas2d, dash Float, gap Float|
 */
export function setLineDash(ctx, dash, gap) {
    ctx.setLineDash([dash, gap]);
}

/**
 * @moth.sig set_line_dash_solid |ctx ~Canvas2d|
 */
export function setLineDashSolid(ctx) {
    ctx.setLineDash([]);
}

/**
 * @moth.sig set_line_dash_offset |ctx ~Canvas2d, offset Float|
 */
export function setLineDashOffset(ctx, offset) {
    ctx.lineDashOffset = offset;
}

/**
 * @moth.sig set_font |ctx ~Canvas2d, font String|
 */
export function setFont(ctx, font) {
    ctx.font = font;
}

/**
 * @moth.sig set_text_align |ctx ~Canvas2d, align String|
 */
export function setTextAlign(ctx, align) {
    ctx.textAlign = align;
}

/**
 * @moth.sig set_text_baseline |ctx ~Canvas2d, baseline String|
 */
export function setTextBaseline(ctx, baseline) {
    ctx.textBaseline = baseline;
}

/**
 * @moth.sig set_direction |ctx ~Canvas2d, direction String|
 */
export function setDirection(ctx, direction) {
    ctx.direction = direction;
}

/**
 * @moth.sig set_letter_spacing |ctx ~Canvas2d, spacing String|
 */
export function setLetterSpacing(ctx, spacing) {
    ctx.letterSpacing = spacing;
}

/**
 * @moth.sig set_word_spacing |ctx ~Canvas2d, spacing String|
 */
export function setWordSpacing(ctx, spacing) {
    ctx.wordSpacing = spacing;
}

/**
 * @moth.sig set_font_kerning |ctx ~Canvas2d, kerning String|
 */
export function setFontKerning(ctx, kerning) {
    ctx.fontKerning = kerning;
}

/**
 * @moth.sig set_font_stretch |ctx ~Canvas2d, stretch String|
 */
export function setFontStretch(ctx, stretch) {
    ctx.fontStretch = stretch;
}

/**
 * @moth.sig set_font_variant_caps |ctx ~Canvas2d, caps String|
 */
export function setFontVariantCaps(ctx, caps) {
    ctx.fontVariantCaps = caps;
}

/**
 * @moth.sig set_text_rendering |ctx ~Canvas2d, rendering String|
 */
export function setTextRendering(ctx, rendering) {
    ctx.textRendering = rendering;
}

/**
 * @moth.sig set_lang |ctx ~Canvas2d, lang String|
 */
export function setLang(ctx, lang) {
    ctx.lang = lang;
}

/**
 * @moth.sig set_image_smoothing_enabled |ctx ~Canvas2d, enabled Bool|
 */
export function setImageSmoothingEnabled(ctx, enabled) {
    ctx.imageSmoothingEnabled = enabled;
}

/**
 * @moth.sig set_image_smoothing_quality |ctx ~Canvas2d, quality String|
 */
export function setImageSmoothingQuality(ctx, quality) {
    ctx.imageSmoothingQuality = quality;
}

/**
 * @moth.sig set_filter |ctx ~Canvas2d, filter String|
 */
export function setFilter(ctx, filter) {
    ctx.filter = filter;
}

/**
 * @moth.sig set_shadow_color |ctx ~Canvas2d, color String|
 */
export function setShadowColor(ctx, color) {
    ctx.shadowColor = color;
}

/**
 * @moth.sig set_shadow_blur |ctx ~Canvas2d, blur Float|
 */
export function setShadowBlur(ctx, blur) {
    ctx.shadowBlur = blur;
}

/**
 * @moth.sig set_shadow_offset |ctx ~Canvas2d, x Float, y Float|
 */
export function setShadowOffset(ctx, x, y) {
    ctx.shadowOffsetX = x;
    ctx.shadowOffsetY = y;
}

/**
 * @moth.sig begin_path |ctx ~Canvas2d|
 */
export function beginPath(ctx) {
    ctx.beginPath();
}

/**
 * @moth.sig move_to |ctx ~Canvas2d, x Float, y Float|
 */
export function moveTo(ctx, x, y) {
    ctx.moveTo(x, y);
}

/**
 * @moth.sig line_to |ctx ~Canvas2d, x Float, y Float|
 */
export function lineTo(ctx, x, y) {
    ctx.lineTo(x, y);
}

/**
 * @moth.sig close_path |ctx ~Canvas2d|
 */
export function closePath(ctx) {
    ctx.closePath();
}

/**
 * @moth.sig rect |ctx ~Canvas2d, x Float, y Float, width Float, height Float|
 */
export function rect(ctx, x, y, width, height) {
    ctx.rect(x, y, width, height);
}

/**
 * @moth.sig round_rect |ctx ~Canvas2d, x Float, y Float, width Float, height Float, radius Float| -> Error!
 */
export function roundRect(ctx, x, y, width, height, radius) {
    try {
        if (typeof ctx.roundRect === "function") {
            ctx.roundRect(x, y, width, height, radius);
        } else {
            addRoundedRectPath(ctx, x, y, width, height, radius);
        }

        return okVoid();
    } catch (error) {
        return domError(error, "Could not add rounded rectangle to canvas path");
    }
}

/**
 * @moth.sig arc |ctx ~Canvas2d, x Float, y Float, radius Float, start_angle Float, end_angle Float| -> Error!
 */
export function arc(ctx, x, y, radius, startAngle, endAngle) {
    try {
        ctx.arc(x, y, radius, startAngle, endAngle);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not add arc to canvas path");
    }
}

/**
 * @moth.sig arc_counterclockwise |ctx ~Canvas2d, x Float, y Float, radius Float, start_angle Float, end_angle Float| -> Error!
 */
export function arcCounterclockwise(ctx, x, y, radius, startAngle, endAngle) {
    try {
        ctx.arc(x, y, radius, startAngle, endAngle, true);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not add counterclockwise arc to canvas path");
    }
}

/**
 * @moth.sig arc_to |ctx ~Canvas2d, x1 Float, y1 Float, x2 Float, y2 Float, radius Float| -> Error!
 */
export function arcTo(ctx, x1, y1, x2, y2, radius) {
    try {
        ctx.arcTo(x1, y1, x2, y2, radius);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not add tangent arc to canvas path");
    }
}

/**
 * @moth.sig quadratic_curve_to |ctx ~Canvas2d, cpx Float, cpy Float, x Float, y Float|
 */
export function quadraticCurveTo(ctx, cpx, cpy, x, y) {
    ctx.quadraticCurveTo(cpx, cpy, x, y);
}

/**
 * @moth.sig bezier_curve_to |ctx ~Canvas2d, cp1x Float, cp1y Float, cp2x Float, cp2y Float, x Float, y Float|
 */
export function bezierCurveTo(ctx, cp1x, cp1y, cp2x, cp2y, x, y) {
    ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, x, y);
}

/**
 * @moth.sig ellipse |ctx ~Canvas2d, x Float, y Float, radius_x Float, radius_y Float, rotation Float, start_angle Float, end_angle Float| -> Error!
 */
export function ellipse(ctx, x, y, radiusX, radiusY, rotation, startAngle, endAngle) {
    try {
        ctx.ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not add ellipse to canvas path");
    }
}

/**
 * @moth.sig ellipse_counterclockwise |ctx ~Canvas2d, x Float, y Float, radius_x Float, radius_y Float, rotation Float, start_angle Float, end_angle Float| -> Error!
 */
export function ellipseCounterclockwise(ctx, x, y, radiusX, radiusY, rotation, startAngle, endAngle) {
    try {
        ctx.ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle, true);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not add counterclockwise ellipse to canvas path");
    }
}

/**
 * @moth.sig fill |ctx ~Canvas2d|
 */
export function fill(ctx) {
    ctx.fill();
}

/**
 * @moth.sig fill_even_odd |ctx ~Canvas2d|
 */
export function fillEvenOdd(ctx) {
    ctx.fill("evenodd");
}

/**
 * @moth.sig stroke |ctx ~Canvas2d|
 */
export function stroke(ctx) {
    ctx.stroke();
}

/**
 * @moth.sig clip |ctx ~Canvas2d|
 */
export function clip(ctx) {
    ctx.clip();
}

/**
 * @moth.sig clip_even_odd |ctx ~Canvas2d|
 */
export function clipEvenOdd(ctx) {
    ctx.clip("evenodd");
}

/**
 * @moth.sig is_point_in_path |ctx Canvas2d, x Float, y Float| -> Bool
 */
export function isPointInPath(ctx, x, y) {
    return ctx.isPointInPath(x, y);
}

/**
 * @moth.sig is_point_in_stroke |ctx Canvas2d, x Float, y Float| -> Bool
 */
export function isPointInStroke(ctx, x, y) {
    return ctx.isPointInStroke(x, y);
}

/**
 * @moth.sig translate |ctx ~Canvas2d, x Float, y Float|
 */
export function translate(ctx, x, y) {
    ctx.translate(x, y);
}

/**
 * @moth.sig rotate |ctx ~Canvas2d, angle Float|
 */
export function rotate(ctx, angle) {
    ctx.rotate(angle);
}

/**
 * @moth.sig scale |ctx ~Canvas2d, x Float, y Float|
 */
export function scale(ctx, x, y) {
    ctx.scale(x, y);
}

/**
 * @moth.sig transform |ctx ~Canvas2d, a Float, b Float, c Float, d Float, e Float, f Float|
 */
export function transform(ctx, a, b, c, d, e, f) {
    ctx.transform(a, b, c, d, e, f);
}

/**
 * @moth.sig set_transform |ctx ~Canvas2d, a Float, b Float, c Float, d Float, e Float, f Float|
 */
export function setTransform(ctx, a, b, c, d, e, f) {
    ctx.setTransform(a, b, c, d, e, f);
}

/**
 * @moth.sig reset_transform |ctx ~Canvas2d|
 */
export function resetTransform(ctx) {
    ctx.resetTransform();
}

/**
 * @moth.sig fill_text |ctx ~Canvas2d, text String, x Float, y Float|
 */
export function fillText(ctx, text, x, y) {
    ctx.fillText(text, x, y);
}

/**
 * @moth.sig fill_text_max_width |ctx ~Canvas2d, text String, x Float, y Float, max_width Float|
 */
export function fillTextMaxWidth(ctx, text, x, y, maxWidth) {
    ctx.fillText(text, x, y, maxWidth);
}

/**
 * @moth.sig stroke_text |ctx ~Canvas2d, text String, x Float, y Float|
 */
export function strokeText(ctx, text, x, y) {
    ctx.strokeText(text, x, y);
}

/**
 * @moth.sig stroke_text_max_width |ctx ~Canvas2d, text String, x Float, y Float, max_width Float|
 */
export function strokeTextMaxWidth(ctx, text, x, y, maxWidth) {
    ctx.strokeText(text, x, y, maxWidth);
}

/**
 * @moth.sig measure_text |ctx Canvas2d, text String| -> CanvasTextMetrics
 */
export function measureText(ctx, text) {
    return ctx.measureText(text);
}

/**
 * @moth.sig text_width |metrics CanvasTextMetrics| -> Float
 */
export function textWidth(metrics) {
    return numberOrZero(metrics.width);
}

/**
 * @moth.sig text_actual_bounding_box_left |metrics CanvasTextMetrics| -> Float
 */
export function textActualBoundingBoxLeft(metrics) {
    return numberOrZero(metrics.actualBoundingBoxLeft);
}

/**
 * @moth.sig text_actual_bounding_box_right |metrics CanvasTextMetrics| -> Float
 */
export function textActualBoundingBoxRight(metrics) {
    return numberOrZero(metrics.actualBoundingBoxRight);
}

/**
 * @moth.sig text_actual_bounding_box_ascent |metrics CanvasTextMetrics| -> Float
 */
export function textActualBoundingBoxAscent(metrics) {
    return numberOrZero(metrics.actualBoundingBoxAscent);
}

/**
 * @moth.sig text_actual_bounding_box_descent |metrics CanvasTextMetrics| -> Float
 */
export function textActualBoundingBoxDescent(metrics) {
    return numberOrZero(metrics.actualBoundingBoxDescent);
}

/**
 * @moth.sig text_font_bounding_box_ascent |metrics CanvasTextMetrics| -> Float
 */
export function textFontBoundingBoxAscent(metrics) {
    return numberOrZero(metrics.fontBoundingBoxAscent);
}

/**
 * @moth.sig text_font_bounding_box_descent |metrics CanvasTextMetrics| -> Float
 */
export function textFontBoundingBoxDescent(metrics) {
    return numberOrZero(metrics.fontBoundingBoxDescent);
}

/**
 * @moth.sig create_linear_gradient |ctx Canvas2d, x0 Float, y0 Float, x1 Float, y1 Float| -> CanvasGradient, Error!
 */
export function createLinearGradient(ctx, x0, y0, x1, y1) {
    try {
        return mothOk(ctx.createLinearGradient(x0, y0, x1, y1));
    } catch (error) {
        return domError(error, "Could not create linear canvas gradient");
    }
}

/**
 * @moth.sig create_radial_gradient |ctx Canvas2d, x0 Float, y0 Float, r0 Float, x1 Float, y1 Float, r1 Float| -> CanvasGradient, Error!
 */
export function createRadialGradient(ctx, x0, y0, r0, x1, y1, r1) {
    try {
        return mothOk(ctx.createRadialGradient(x0, y0, r0, x1, y1, r1));
    } catch (error) {
        return domError(error, "Could not create radial canvas gradient");
    }
}

/**
 * @moth.sig create_conic_gradient |ctx Canvas2d, start_angle Float, x Float, y Float| -> CanvasGradient, Error!
 */
export function createConicGradient(ctx, startAngle, x, y) {
    try {
        return mothOk(ctx.createConicGradient(startAngle, x, y));
    } catch (error) {
        return domError(error, "Could not create conic canvas gradient");
    }
}

/**
 * @moth.sig add_color_stop |gradient ~CanvasGradient, offset Float, color String| -> Error!
 */
export function addColorStop(gradient, offset, color) {
    try {
        gradient.addColorStop(offset, color);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not add color stop to canvas gradient");
    }
}

/**
 * @moth.sig create_pattern |ctx Canvas2d, image CanvasImage, repetition String| -> CanvasPattern, Error!
 */
export function createPattern(ctx, image, repetition) {
    const loadedError = assertLoadedImage(image);
    if (loadedError) {
        return loadedError;
    }

    try {
        const pattern = ctx.createPattern(image, repetition);
        if (!pattern) {
            return mothErr(409, "Canvas pattern could not be created from the image");
        }

        return mothOk(pattern);
    } catch (error) {
        return domError(error, "Could not create canvas pattern");
    }
}

/**
 * @moth.sig create_canvas_pattern |ctx Canvas2d, canvas CanvasElement, repetition String| -> CanvasPattern, Error!
 */
export function createCanvasPattern(ctx, canvas, repetition) {
    try {
        const pattern = ctx.createPattern(canvas, repetition);
        if (!pattern) {
            return mothErr(409, "Canvas pattern could not be created from the source canvas");
        }

        return mothOk(pattern);
    } catch (error) {
        return domError(error, "Could not create canvas pattern");
    }
}

/**
 * @moth.sig draw_image |ctx ~Canvas2d, image CanvasImage, x Float, y Float| -> Error!
 */
export function drawImage(ctx, image, x, y) {
    const loadedError = assertLoadedImage(image);
    if (loadedError) {
        return loadedError;
    }

    try {
        ctx.drawImage(image, x, y);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not draw image onto canvas");
    }
}

/**
 * @moth.sig draw_image_scaled |ctx ~Canvas2d, image CanvasImage, x Float, y Float, width Float, height Float| -> Error!
 */
export function drawImageScaled(ctx, image, x, y, width, height) {
    const loadedError = assertLoadedImage(image);
    if (loadedError) {
        return loadedError;
    }

    try {
        ctx.drawImage(image, x, y, width, height);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not draw scaled image onto canvas");
    }
}

/**
 * @moth.sig draw_image_cropped |ctx ~Canvas2d, image CanvasImage, source_x Float, source_y Float, source_width Float, source_height Float, x Float, y Float, width Float, height Float| -> Error!
 */
export function drawImageCropped(ctx, image, sourceX, sourceY, sourceWidth, sourceHeight, x, y, width, height) {
    const loadedError = assertLoadedImage(image);
    if (loadedError) {
        return loadedError;
    }

    try {
        ctx.drawImage(image, sourceX, sourceY, sourceWidth, sourceHeight, x, y, width, height);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not draw cropped image onto canvas");
    }
}

/**
 * @moth.sig draw_canvas |ctx ~Canvas2d, canvas CanvasElement, x Float, y Float| -> Error!
 */
export function drawCanvas(ctx, canvas, x, y) {
    try {
        ctx.drawImage(canvas, x, y);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not draw source canvas onto canvas");
    }
}

/**
 * @moth.sig draw_canvas_scaled |ctx ~Canvas2d, canvas CanvasElement, x Float, y Float, width Float, height Float| -> Error!
 */
export function drawCanvasScaled(ctx, canvas, x, y, width, height) {
    try {
        ctx.drawImage(canvas, x, y, width, height);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not draw scaled source canvas onto canvas");
    }
}

/**
 * @moth.sig get_image_data |ctx Canvas2d, x Int, y Int, width Int, height Int| -> CanvasImageData, Error!
 */
export function getImageData(ctx, x, y, width, height) {
    try {
        return mothOk(ctx.getImageData(x, y, width, height));
    } catch (error) {
        return domError(error, "Could not read canvas image data");
    }
}

/**
 * @moth.sig create_image_data |ctx Canvas2d, width Int, height Int| -> CanvasImageData, Error!
 */
export function createImageData(ctx, width, height) {
    try {
        return mothOk(ctx.createImageData(width, height));
    } catch (error) {
        return domError(error, "Could not create canvas image data");
    }
}

/**
 * @moth.sig create_image_data_from |ctx Canvas2d, image_data CanvasImageData| -> CanvasImageData, Error!
 */
export function createImageDataFrom(ctx, imageData) {
    try {
        return mothOk(ctx.createImageData(imageData));
    } catch (error) {
        return domError(error, "Could not clone canvas image data shape");
    }
}

/**
 * @moth.sig put_image_data |ctx ~Canvas2d, image_data CanvasImageData, x Float, y Float| -> Error!
 */
export function putImageData(ctx, imageData, x, y) {
    try {
        ctx.putImageData(imageData, x, y);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not write canvas image data");
    }
}

/**
 * @moth.sig put_image_data_dirty |ctx ~Canvas2d, image_data CanvasImageData, x Float, y Float, dirty_x Float, dirty_y Float, dirty_width Float, dirty_height Float| -> Error!
 */
export function putImageDataDirty(ctx, imageData, x, y, dirtyX, dirtyY, dirtyWidth, dirtyHeight) {
    try {
        ctx.putImageData(imageData, x, y, dirtyX, dirtyY, dirtyWidth, dirtyHeight);
        return okVoid();
    } catch (error) {
        return domError(error, "Could not write dirty canvas image data rectangle");
    }
}

/**
 * @moth.sig image_data_width |image_data CanvasImageData| -> Int
 */
export function imageDataWidth(imageData) {
    return imageData.width;
}

/**
 * @moth.sig image_data_height |image_data CanvasImageData| -> Int
 */
export function imageDataHeight(imageData) {
    return imageData.height;
}

/**
 * @moth.sig image_data_get_red |image_data CanvasImageData, x Int, y Int| -> Int, Error!
 */
export function imageDataGetRed(imageData, x, y) {
    const pointError = assertImageDataPoint(imageData, x, y);
    if (pointError) {
        return pointError;
    }

    return mothOk(imageData.data[imageDataIndex(imageData, x, y)]);
}

/**
 * @moth.sig image_data_get_green |image_data CanvasImageData, x Int, y Int| -> Int, Error!
 */
export function imageDataGetGreen(imageData, x, y) {
    const pointError = assertImageDataPoint(imageData, x, y);
    if (pointError) {
        return pointError;
    }

    return mothOk(imageData.data[imageDataIndex(imageData, x, y) + 1]);
}

/**
 * @moth.sig image_data_get_blue |image_data CanvasImageData, x Int, y Int| -> Int, Error!
 */
export function imageDataGetBlue(imageData, x, y) {
    const pointError = assertImageDataPoint(imageData, x, y);
    if (pointError) {
        return pointError;
    }

    return mothOk(imageData.data[imageDataIndex(imageData, x, y) + 2]);
}

/**
 * @moth.sig image_data_get_alpha |image_data CanvasImageData, x Int, y Int| -> Int, Error!
 */
export function imageDataGetAlpha(imageData, x, y) {
    const pointError = assertImageDataPoint(imageData, x, y);
    if (pointError) {
        return pointError;
    }

    return mothOk(imageData.data[imageDataIndex(imageData, x, y) + 3]);
}

/**
 * @moth.sig image_data_set_pixel |image_data ~CanvasImageData, x Int, y Int, red Int, green Int, blue Int, alpha Int| -> Error!
 */
export function imageDataSetPixel(imageData, x, y, red, green, blue, alpha) {
    const pointError = assertImageDataPoint(imageData, x, y);
    if (pointError) {
        return pointError;
    }

    const index = imageDataIndex(imageData, x, y);
    imageData.data[index] = clampByte(red);
    imageData.data[index + 1] = clampByte(green);
    imageData.data[index + 2] = clampByte(blue);
    imageData.data[index + 3] = clampByte(alpha);

    return okVoid();
}

/**
 * @moth.sig image_data_clear_pixel |image_data ~CanvasImageData, x Int, y Int| -> Error!
 */
export function imageDataClearPixel(imageData, x, y) {
    const pointError = assertImageDataPoint(imageData, x, y);
    if (pointError) {
        return pointError;
    }

    const index = imageDataIndex(imageData, x, y);
    imageData.data[index] = 0;
    imageData.data[index + 1] = 0;
    imageData.data[index + 2] = 0;
    imageData.data[index + 3] = 0;

    return okVoid();
}
