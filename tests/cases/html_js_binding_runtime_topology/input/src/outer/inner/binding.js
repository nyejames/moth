const records = [];
const choices = [];
let generatedCalls = 0;
let limitCalls = 0;

/** @moth.const limit U32 */
export const LIMIT = 4294967295;

/** @moth.sig record_event |label String| -> */
export function recordEvent(label) {
    records.push(label);
}

/** @moth.sig choose |mode String| -> String, Error! */
export function chooseOutcome(mode) {
    choices.push(mode);
    if (mode === "ok") {
        return { ok: true, value: `accepted:${choices.length}` };
    }
    return { ok: false, error: { message: "fixture failure", code: 73 } };
}

/** @moth.sig generated_label || -> String */
export function generatedLabel() {
    generatedCalls += 1;
    return `generated:${generatedCalls}`;
}

/** @moth.sig read_limit |value U32| -> U32 */
export function readLimit(value) {
    limitCalls += 1;
    return value;
}

/** @moth.sig observed || -> String */
export function observeState() {
    return `records=${records.join(",")};choices=${choices.join(",")};generated=${generatedCalls};limits=${limitCalls}`;
}

/** @moth.sig unused_live || -> String */
export function unusedLiveExport() {
    return "unused export in a live asset";
}
