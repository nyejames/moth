# Moth Tree-sitter grammars

Status: initial candidate grammar sources and hand-authored corpus. Generation and real parser tests are not yet verified.

The [design](../design.md), [highlighting vocabulary](../highlighting.md) and [roadmap](../roadmap.md) own scope. The three root languages share ordinary JavaScript rule definitions. Tree-sitter generates the parser sources and node metadata per grammar.

- `common/`: lexical leaves, named records and maps, constructions and shared Moth expressions.
- `grammars/moth/`: code root, simple bindings, basic type names and documentation operands.
- `grammars/mtf/`: implicit template root.
- `grammars/mon/`: explicit/implicit literal record root, sequences and maps.
- `scripts/grammar.cjs`: native CLI generation and corpus execution per root.
- Each grammar's `queries/` and `test/corpus/`: initial query and exact-tree expectations.

From this directory, with Node.js LTS, npm, a native C compiler and network access:

```sh
npm install
npm run generate
npm test
```

The CLI is pinned to 0.25.10 and generation targets ABI 14. The initial npm lockfile and generated C/metadata must be produced by a real successful install/generate run and committed after review. Use `npm ci` thereafter. Do not handwrite generated artifacts. Validate grammar metadata discovery from each nested root. Native/Wasm equivalence, host versions and package layout remain prototype gates.

This slice includes shared literal/record syntax, bracket heads, raw bodies, basic formatter heads, balanced notes and code listings. It deliberately omits full statements, precedence, qualified constructors, typed containers, slot/control grammar, comment/spacing edge cases, contextual names and recovery rules. Corpus trees are candidate assertions, not recorded execution output. Add malformed source cases after verifying the parser's recovery shapes. Query experiments cover HTML/CSS chunks and a literal Moth code listing. They do not group fragments per parent body, supply full guest inventories or implement Markdown. Full Moth Markdown and documentation Markdown highlighting require the next prototype.

Use the delivered syntax authorities before expanding the grammar. No legacy dialect, fake semantic validation or editor-specific grammar copy.
