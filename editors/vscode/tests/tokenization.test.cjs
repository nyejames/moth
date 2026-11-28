// Based on the original extension's real vscode-textmate / Oniguruma harness.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const oniguruma = require("vscode-oniguruma");
const textmate = require("vscode-textmate");

const root = path.join(__dirname, "..");
const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const grammarPaths = new Map(manifest.contributes.grammars.map(entry => [
    entry.scopeName, path.join(root, entry.path),
]));

let registryPromise;
function loadRegistry() {
    if (!registryPromise) {
        registryPromise = (async () => {
            const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
            await oniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
            return new textmate.Registry({
                onigLib: Promise.resolve({
                    createOnigScanner: patterns => new oniguruma.OnigScanner(patterns),
                    createOnigString: source => new oniguruma.OnigString(source),
                }),
                loadGrammar: async scope => {
                    const filename = grammarPaths.get(scope);
                    return filename ? JSON.parse(fs.readFileSync(filename, "utf8")) : null;
                },
            });
        })();
    }
    return registryPromise;
}

async function tokenize(scope, lines) {
    const registry = await loadRegistry();
    const grammar = await registry.loadGrammar(scope);
    let stack = textmate.INITIAL;
    return lines.map(line => {
        const result = grammar.tokenizeLine(line, stack);
        stack = result.ruleStack;
        return result.tokens.map(token => ({
            text: line.slice(token.startIndex, token.endIndex),
            scopes: token.scopes,
        }));
    });
}

function hasScope(tokens, text, scope) {
    return tokens.some(token => token.text.trim() === text && token.scopes.includes(scope));
}

test("braces, case and new arm separator have code scopes", async () => {
    const [header, arm, end] = await tokenize("source.moth", [
        "if status is {", "    case Ready: then 1", "}",
    ]);
    assert.ok(hasScope(header, "{", "punctuation.section.block.moth"));
    assert.ok(hasScope(arm, "case", "keyword.control.flow.moth"));
    assert.ok(hasScope(arm, ":", "punctuation.separator.colon.moth"));
    assert.ok(hasScope(end, "}", "punctuation.section.block.moth"));
});

test("ordinary code and template body comment markers differ", async () => {
    const [code, body] = await tokenize("source.moth", ["-- comment", "message = [: -- text]"]);
    assert.ok(code.some(token => token.scopes.includes("comment.line.double-dash.moth")));
    assert.ok(!body.some(token => token.scopes.includes("comment.line.double-dash.moth")));
    assert.ok(body.some(token => token.text.includes("-- text") &&
        token.scopes.includes("string.unquoted.template.body.moth")));
});

test("documentation gets Markdown scopes and explicit raw overrides", async () => {
    const [doc, raw] = await tokenize("source.moth", ["$doc [: **important**]", "$doc [$raw: **literal**]"]);
    assert.ok(doc.some(token => token.text.includes("important") &&
        token.scopes.includes("meta.embedded.block.markdown.moth")));
    assert.ok(!raw.some(token => token.scopes.includes("meta.embedded.block.markdown.moth")));
});

test("traits are distinguished in declaration and conformance positions", async () => {
    const [declaration, conformance] = await tokenize("source.moth", [
        "DISPLAY_TEXT must {", "Label must DISPLAY_TEXT",
    ]);
    assert.ok(hasScope(declaration, "DISPLAY_TEXT", "entity.name.type.trait.moth"));
    assert.ok(hasScope(conformance, "DISPLAY_TEXT", "entity.name.type.trait.moth"));
});

test("MON uses literal string escapes, not template interpolation", async () => {
    const [line] = await tokenize("source.mon", ['name = "\\u{41}[name]",']);
    assert.ok(line.some(token => token.scopes.includes("constant.character.escape.mon")));
    assert.ok(!line.some(token => token.scopes.includes("meta.template.moth")));
});

test("literal guest listing and nested Moth template use distinct scopes", async () => {
    const [listing, template] = await tokenize("source.mtf", [
        '[$code "json": {"names": ["Priya"]}]', '[$html:<b>[name]</b>]',
    ]);
    assert.ok(listing.some(token => token.scopes.includes("meta.embedded.block.code.json.moth")));
    assert.ok(template.some(token => token.text === "name" &&
        token.scopes.includes("meta.template.head.moth")));
});
