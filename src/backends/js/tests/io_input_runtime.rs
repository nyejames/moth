//! Executable behaviour tests for the `@core/io` input polling helpers.
//!
//! Each scenario lowers a real module that reaches `io.input.new`, installs a minimal fake browser
//! host in Node.js and dispatches synthetic events. The Node runtime harness used by integration
//! cases has no `window`, so snapshot, transition, focus-loss and teardown semantics are owned here.
//! [io-input-helper]

use super::support::*;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use std::process::Command;

/// Fake host and snapshot printer shared by every scenario.
///
/// `snapshot(input)` prints one line: held keys, `+` pressed keys and `-` released keys, the same
/// three sets for buttons, the published pointer position and the four `last_*` reads, with `-`
/// for `none`. Key and button sets list the probed names that read true, in probe order.
const FAKE_HOST_HARNESS: &str = r#"
const preventedEvents = [];
class FakeAbortSignal {
    constructor() { this.aborted = false; this.abortHandlers = []; }
    addEventListener(type, handler) { if (type === "abort") this.abortHandlers.push(handler); }
}
class FakeAbortController {
    constructor() { this.signal = new FakeAbortSignal(); }
    abort() {
        if (this.signal.aborted) return;
        this.signal.aborted = true;
        for (const handler of this.signal.abortHandlers) handler();
    }
}
class FakeEventTarget {
    constructor() { this.registrations = []; }
    addEventListener(type, listener, options) {
        const signal = options && options.signal;
        if (signal && signal.aborted) return;
        const registration = { type, listener, options };
        this.registrations.push(registration);
        if (signal) {
            signal.addEventListener("abort", () => {
                this.registrations = this.registrations.filter((entry) => entry !== registration);
            });
        }
    }
    dispatch(type, fields) {
        const event = Object.assign({ type, preventDefault() { preventedEvents.push(type); } }, fields);
        for (const registration of this.registrations.slice()) {
            if (registration.type === type) registration.listener.call(this, event);
        }
    }
}
function installHost() {
    globalThis.window = new FakeEventTarget();
    window.PointerEvent = function PointerEvent() {};
    globalThis.document = new FakeEventTarget();
    document.hidden = false;
    globalThis.AbortController = FakeAbortController;
}
function openInput() {
    installHost();
    const result = __moth_io_input_new();
    if (result.tag !== "ok") throw new Error("io.input.new rejected a complete fake host");
    return result.value;
}
const keyDown = (code, key) => window.dispatch("keydown", { code, key });
const keyUp = (code, key) => window.dispatch("keyup", { code, key });
const pointer = (type, button, x, y) => window.dispatch(type, { button, clientX: x, clientY: y });
const update = (input) => __moth_io_input_update(input);
const PROBED_KEYS = ["a", "b", "1", "!", "Space", "Shift", "É", "é"];
const PROBED_BUTTONS = ["left", "middle", "right"];
function probe(names, read) {
    return "[" + names.filter(read).join(",") + "]";
}
function option(carrier) {
    return carrier.tag === "some" ? carrier.value : "-";
}
function snapshot(input) {
    console.log([
        "keys=" + probe(PROBED_KEYS, (key) => __moth_io_input_key_down(input, key)),
        "+" + probe(PROBED_KEYS, (key) => __moth_io_input_key_pressed(input, key)),
        "-" + probe(PROBED_KEYS, (key) => __moth_io_input_key_released(input, key)),
        "buttons=" + probe(PROBED_BUTTONS, (button) => __moth_io_input_pointer_down(input, button)),
        "+" + probe(PROBED_BUTTONS, (button) => __moth_io_input_pointer_pressed(input, button)),
        "-" + probe(PROBED_BUTTONS, (button) => __moth_io_input_pointer_released(input, button)),
        "at=" + __moth_io_input_pointer_x(input) + "," + __moth_io_input_pointer_y(input),
        "last=" + option(__moth_io_input_last_key_pressed(input)) + "/" + option(__moth_io_input_last_key_released(input)),
        "last_button=" + option(__moth_io_input_last_pointer_pressed(input)) + "/" + option(__moth_io_input_last_pointer_released(input)),
    ].join(" "));
}
"#;

const NEUTRAL: &str = "keys=[] +[] -[] buttons=[] +[] -[] at=0,0 last=-/- last_button=-/-";

/// Runs one scenario against the emitted input helpers and returns its printed lines.
fn run_input_scenario(scenario: &str) -> Vec<String> {
    let module_source =
        lower_minimal_module_with_io_input_call("main", ExternalFunctionId::IoInputNew);
    let script = format!("{module_source}\n{FAKE_HOST_HARNESS}\n{scenario}");
    let output = Command::new("node")
        .args(["--eval", &script])
        .output()
        .expect("Node.js is required for input runtime behaviour tests");
    assert!(
        output.status.success(),
        "input scenario failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Node.js output is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn missing_host_apis_return_canonical_unsupported_error() {
    let lines = run_input_scenario(
        r#"
function report(label) {
    const result = __moth_io_input_new();
    if (result.tag === "ok") {
        console.log(label + " ok");
        return;
    }
    console.log(label + " " + __moth_error_code(result.value) + " " + __moth_error_message(result.value));
}
delete globalThis.window;
delete globalThis.document;
report("no-window");
installHost();
delete globalThis.document;
report("no-document");
installHost();
delete window.PointerEvent;
report("no-pointer-events");
installHost();
delete globalThis.AbortController;
report("no-abort-controller");
installHost();
report("complete-host");
"#,
    );

    let unsupported = BuiltinErrorCode::Unsupported;
    let failure = format!("{} {}", unsupported.as_u32(), unsupported.default_message());
    assert_eq!(
        lines,
        vec![
            format!("no-window {failure}"),
            format!("no-document {failure}"),
            format!("no-pointer-events {failure}"),
            format!("no-abort-controller {failure}"),
            "complete-host ok".to_owned(),
        ]
    );
}

#[test]
fn reads_observe_only_the_snapshot_published_by_update() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
pointer("pointerdown", 0, 10, 20);
pointer("pointermove", -1, 30, 40);
snapshot(input);
snapshot(input);
update(input);
snapshot(input);
pointer("pointermove", -1, 50, 60);
keyUp("KeyA", "a");
snapshot(input);
update(input);
snapshot(input);
"#,
    );

    let published =
        "keys=[a] +[a] -[] buttons=[left] +[left] -[] at=30,40 last=a/- last_button=left/-";
    assert_eq!(
        lines,
        vec![
            NEUTRAL,
            NEUTRAL,
            published,
            published,
            "keys=[] +[] -[a] buttons=[left] +[] -[] at=50,60 last=-/a last_button=-/-",
        ]
    );
}

#[test]
fn held_state_persists_across_updates_while_transitions_clear() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
update(input);
snapshot(input);
update(input);
snapshot(input);
keyUp("KeyA", "a");
update(input);
snapshot(input);
update(input);
snapshot(input);
"#,
    );

    assert_eq!(
        lines,
        vec![
            "keys=[a] +[a] -[] buttons=[] +[] -[] at=0,0 last=a/- last_button=-/-",
            "keys=[a] +[] -[] buttons=[] +[] -[] at=0,0 last=-/- last_button=-/-",
            "keys=[] +[] -[a] buttons=[] +[] -[] at=0,0 last=-/a last_button=-/-",
            NEUTRAL,
        ]
    );
}

#[test]
fn press_and_release_between_updates_report_both_transitions() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
keyUp("KeyA", "a");
pointer("pointerdown", 0, 1, 2);
pointer("pointerup", 0, 1, 2);
update(input);
snapshot(input);
"#,
    );

    assert_eq!(
        lines,
        vec!["keys=[] +[a] -[a] buttons=[] +[left] -[left] at=1,2 last=a/a last_button=left/left"]
    );
}

#[test]
fn repeated_keydown_and_pointerdown_do_not_repeat_presses() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
keyDown("KeyA", "a");
pointer("pointerdown", 0, 0, 0);
pointer("pointerdown", 0, 0, 0);
update(input);
snapshot(input);
keyDown("KeyA", "a");
keyDown("KeyA", "a");
pointer("pointerdown", 0, 0, 0);
update(input);
snapshot(input);
"#,
    );

    assert_eq!(
        lines,
        vec![
            "keys=[a] +[a] -[] buttons=[left] +[left] -[] at=0,0 last=a/- last_button=left/-",
            "keys=[a] +[] -[] buttons=[left] +[] -[] at=0,0 last=-/- last_button=-/-",
        ]
    );
}

#[test]
fn transitions_coalesce_per_name_and_last_reads_keep_the_most_recent() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
keyUp("KeyA", "a");
keyDown("KeyA", "a");
keyDown("KeyB", "b");
keyUp("KeyB", "b");
pointer("pointerdown", 0, 0, 0);
pointer("pointerdown", 2, 0, 0);
pointer("pointerup", 0, 0, 0);
pointer("pointerdown", 3, 0, 0);
update(input);
snapshot(input);
"#,
    );

    // Button 3 has no portable name, so it records no transition.
    assert_eq!(
        lines,
        vec![
            "keys=[a] +[a,b] -[a,b] buttons=[right] +[left,right] -[left] at=0,0 last=b/b last_button=right/left"
        ]
    );
}

#[test]
fn keys_release_by_identity_recorded_at_keydown() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("Digit1", "1");
keyDown("ShiftLeft", "Shift");
keyUp("Digit1", "!");
keyUp("ShiftLeft", "Shift");
update(input);
snapshot(input);

keyDown("ShiftLeft", "Shift");
keyDown("ShiftRight", "Shift");
keyUp("ShiftLeft", "Shift");
update(input);
snapshot(input);
keyUp("ShiftRight", "Shift");
update(input);
snapshot(input);

keyDown("", "a");
keyUp("Unidentified", "a");
keyDown("KeyB", "b");
keyUp("", "b");
update(input);
snapshot(input);
"#,
    );

    assert_eq!(
        lines,
        vec![
            "keys=[] +[1,Shift] -[1,Shift] buttons=[] +[] -[] at=0,0 last=Shift/Shift last_button=-/-",
            "keys=[Shift] +[Shift] -[] buttons=[] +[] -[] at=0,0 last=Shift/- last_button=-/-",
            "keys=[] +[] -[Shift] buttons=[] +[] -[] at=0,0 last=-/Shift last_button=-/-",
            "keys=[] +[a,b] -[a,b] buttons=[] +[] -[] at=0,0 last=b/b last_button=-/-",
        ]
    );
}

#[test]
fn key_names_lowercase_ascii_letters_only() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "A");
keyDown("Space", " ");
keyDown("KeyE", "É");
update(input);
snapshot(input);
console.log(__moth_io_input_key_down(input, "A") + " " + __moth_io_input_key_down(input, " "));
"#,
    );

    assert_eq!(
        lines,
        vec![
            "keys=[a,Space,É] +[a,Space,É] -[] buttons=[] +[] -[] at=0,0 last=É/- last_button=-/-",
            "true true",
        ]
    );
}

#[test]
fn focus_and_visibility_loss_release_held_input_at_the_next_update() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
pointer("pointerdown", 0, 5, 6);
update(input);
window.dispatch("blur", {});
snapshot(input);
update(input);
snapshot(input);
keyUp("KeyA", "a");
update(input);
snapshot(input);

keyDown("KeyB", "b");
update(input);
document.dispatch("visibilitychange", {});
update(input);
snapshot(input);
document.hidden = true;
document.dispatch("visibilitychange", {});
update(input);
snapshot(input);

pointer("pointerdown", 2, 7, 8);
update(input);
window.dispatch("pointercancel", {});
update(input);
snapshot(input);
"#,
    );

    assert_eq!(
        lines,
        vec![
            "keys=[a] +[a] -[] buttons=[left] +[left] -[] at=5,6 last=a/- last_button=left/-",
            "keys=[] +[] -[a] buttons=[] +[] -[left] at=5,6 last=-/a last_button=-/left",
            "keys=[] +[] -[] buttons=[] +[] -[] at=5,6 last=-/- last_button=-/-",
            "keys=[b] +[] -[] buttons=[] +[] -[] at=5,6 last=-/- last_button=-/-",
            "keys=[] +[] -[b] buttons=[] +[] -[] at=5,6 last=-/b last_button=-/-",
            "keys=[] +[] -[] buttons=[] +[] -[right] at=7,8 last=-/- last_button=-/right",
        ]
    );
}

#[test]
fn close_clears_state_aborts_listeners_and_stays_inert() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
const keydownListener = window.registrations.find((entry) => entry.type === "keydown").listener;
keyDown("KeyA", "a");
pointer("pointerdown", 0, 3, 4);
update(input);
keyDown("KeyB", "b");
__moth_io_input_close(input);
snapshot(input);
console.log("listeners " + window.registrations.length + " " + document.registrations.length);
__moth_io_input_close(input);
update(input);
snapshot(input);
keydownListener({ code: "KeyC", key: "c" });
keyDown("KeyB", "b");
pointer("pointerdown", 2, 9, 9);
update(input);
snapshot(input);
"#,
    );

    assert_eq!(lines, vec![NEUTRAL, "listeners 0 0", NEUTRAL, NEUTRAL]);
}

#[test]
fn listeners_are_passive_and_never_prevent_default() {
    let lines = run_input_scenario(
        r#"
const input = openInput();
keyDown("KeyA", "a");
keyUp("KeyA", "a");
pointer("pointermove", -1, 1, 1);
pointer("pointerdown", 0, 1, 1);
pointer("pointerup", 0, 1, 1);
window.dispatch("pointercancel", {});
window.dispatch("blur", {});
document.hidden = true;
document.dispatch("visibilitychange", {});
const registrations = window.registrations.concat(document.registrations);
console.log(registrations.every((entry) => entry.options.passive === true) + " " + preventedEvents.length);
"#,
    );

    assert_eq!(lines, vec!["true 0"]);
}

#[test]
fn repeated_events_do_not_grow_handle_storage() {
    let lines = run_input_scenario(
        r#"
function storageSize(value) {
    if (value instanceof Set || value instanceof Map) return value.size;
    if (Array.isArray(value)) return value.length;
    if (value === null || typeof value !== "object" || value instanceof FakeAbortController) return 0;
    let total = 0;
    for (const field of Object.values(value)) total += storageSize(field);
    return total;
}
function deliverRound() {
    keyDown("KeyA", "a");
    keyDown("KeyA", "a");
    keyUp("KeyA", "a");
    pointer("pointerdown", 0, 1, 1);
    pointer("pointermove", -1, 2, 2);
    pointer("pointerup", 0, 3, 3);
}
const input = openInput();
deliverRound();
const afterOneRound = storageSize(input);
for (let round = 0; round < 10000; round += 1) deliverRound();
console.log(afterOneRound + " " + storageSize(input));
"#,
    );

    assert_eq!(lines, vec!["4 4"]);
}
