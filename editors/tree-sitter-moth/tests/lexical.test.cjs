const assert = require("node:assert/strict");
const test = require("node:test");
const { lexicalRules } = require("../common/lexical");

// These are JS regex spelling checks, not parser acceptance or codec validation.
function matches(rule, source) {
    const regex = rule();
    return new RegExp("^(?:" + regex.source + ")$", regex.flags).test(source);
}

const moth = lexicalRules();
const mon = lexicalRules(true);

test("identifier spelling includes Unicode alphabetic and numeric characters", () => {
    for (const value of ["_", "display_name", "Moth", "\u03bb2", "\u03bb\u00b2"]) {
        assert.ok(matches(moth.identifier, value), value);
        assert.ok(matches(mon.identifier, value), value);
    }
    for (const value of ["2name", "some-name", "\u{1f98b}"]) {
        assert.ok(!matches(moth.identifier, value), value);
    }
});

test("numeric spelling is exact and shared without width inference", () => {
    for (const value of ["42", "-42", "1_000", "0.5", "1e6", "1.0e-21", "18446744073709551615"]) {
        assert.ok(matches(moth.number, value), value);
        assert.ok(matches(mon.number, value), value);
    }
    for (const value of ["+1", "- 1", "1E3", "0x10", "1__2", "1_", ".5", "1.", "1e", "NaN"]) {
        assert.ok(!matches(moth.number, value), value);
        assert.ok(!matches(mon.number, value), value);
    }
});

test("MON and Moth share ordinary escapes but have distinct Unicode and newline rules", () => {
    for (const rules of [moth, mon]) {
        assert.ok(matches(rules.string, String.raw`"hi\nthere"`));
        assert.ok(matches(rules.string, '"[literal]"'));
        assert.ok(!matches(rules.string, String.raw`"\x41"`));
        assert.ok(matches(rules.character, "'\u{1f98b}'"));
        assert.ok(!matches(rules.character, "'ab'"));
    }
    const unicodeString = String.raw`"\u{41}"`;
    assert.ok(matches(mon.string, unicodeString));
    assert.ok(!matches(moth.string, unicodeString));
    assert.ok(matches(mon.character, String.raw`'\u{1F98B}'`));
    assert.ok(!matches(mon.character, String.raw`'\u{}'`));
    assert.ok(matches(mon.string, '"line\nline"'));
    assert.ok(!matches(moth.string, '"line\nline"'));
});
