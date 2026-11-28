const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const root = path.join(__dirname, "..");
const readJson = relative => JSON.parse(readFileSync(path.join(root, relative), "utf8"));
const manifest = readJson("package.json");
const grammars = new Map(manifest.contributes.grammars.map(entry => [
    entry.scopeName, readJson(entry.path),
]));

function visit(value, callback) {
    if (!value || typeof value !== "object") return;
    callback(value);
    for (const child of Object.values(value)) visit(child, callback);
}

test("every local grammar and cross-grammar repository reference resolves", () => {
    for (const [scope, grammar] of grammars) {
        visit(grammar, rule => {
            if (typeof rule.include !== "string") return;
            const [owner, key] = rule.include.split("#");
            if (!key) return;
            const target = owner ? grammars.get(owner) : grammar;
            if (target) assert.ok(target.repository[key], scope + ": " + rule.include);
        });
    }
});

test("the three file formats use distinct scopes and real configuration paths", () => {
    assert.deepEqual(manifest.contributes.languages.map(entry => entry.extensions[0]).sort(),
        [".mon", ".moth", ".mtf"]);
    for (const language of manifest.contributes.languages) readJson(language.configuration);
    assert.deepEqual([...grammars.keys()].sort(), ["source.mon", "source.moth", "source.mtf"]);
});

test("retired source scopes and grammar roles are removed", () => {
    const grammar = grammars.get("source.moth");
    assert.ok(!Object.hasOwn(grammar.repository, "raw-strings"));
    assert.ok(!JSON.stringify(grammar.repository.keywords).includes("cast!"));
    assert.ok(!JSON.stringify(grammar.repository.constants).includes("Wire|Route"));
    assert.ok(!JSON.stringify(grammar.repository.keywords).includes("async|checked|yield"));
    assert.equal(grammar.repository.punctuation.patterns.find(rule => rule.match === ";").name,
        "invalid.illegal.semicolon.moth");
    assert.ok(grammar.repository.keywords.patterns.some(rule => rule.match?.includes("|case|")));
});

test("literal listing aliases have explicit guest rules ahead of generic fallback", () => {
    const grammar = grammars.get("source.moth");
    const rules = grammar.repository["template-nested"].patterns.map(rule => rule.include);
    for (const id of ["js", "ts", "py", "rs", "shell", "moth", "html", "markdown", "toml", "json", "yaml", "css", "c", "sql"]) {
        assert.ok(rules.indexOf("#template-code-" + id) >= 0);
        assert.ok(rules.indexOf("#template-code-" + id) < rules.indexOf("#template-code-generic"));
    }
});
