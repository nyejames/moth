// Run Tree-sitter from each grammar root so generated sources and corpus stay local.
const { spawnSync } = require("node:child_process");
const path = require("node:path");

const action = process.argv[2];
if (!["generate", "test"].includes(action)) {
    console.error("Usage: node scripts/grammar.cjs generate|test");
    process.exit(2);
}

const root = path.resolve(__dirname, "..");
const cli = path.join(root, "node_modules", "tree-sitter-cli", "tree-sitter" + (process.platform === "win32" ? ".exe" : ""));

for (const language of ["moth", "mtf", "mon"]) {
    console.log(language + ": " + action);
    const args = action === "generate" ? ["generate", "--abi", "14"] : ["test"];
    const result = spawnSync(cli, args, {
        cwd: path.join(root, "grammars", language),
        stdio: "inherit",
    });
    if (result.error) {
        console.error(result.error.message);
        process.exit(1);
    }
    if (result.status !== 0) {
        process.exit(result.status ?? 1);
    }
}
