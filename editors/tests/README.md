# Shared editor integration tests

Status: shared test design. Initial grammar-local corpus expectations now live with each grammar. Cross-host fixture runner and real-host tests are still pending.

Grammar-local corpus and query tests belong in [tree-sitter-moth](../tree-sitter-moth/README.md). This directory owns the shared editor-facing fixtures and cross-host conformance checks. Adapter-specific API and lifecycle tests live with the adapter that owns them.

## Fixture ownership

Select positive examples from the final canonical language references and existing executable compiler/codec fixtures once the syntax migration lands. Reference or derive them where possible instead of copying the compiler test suite. Add only editor-specific witnesses that exercise highlighting, recovery, navigation or editing. A syntax tree is not proof of semantic validity. Accepted but unimplemented syntax receives a separately labelled expectation.

Use one shared fixture per behaviour where practical. Each editor checks the same authored byte ranges and structural roles, translated into its native capture or position representation. Theme colours are not the golden contract. Keep expectations compact and deterministic, using MON for owned machine-readable fixtures where useful and native Tree-sitter corpus formats where required.

## First prototype cases

- A Moth code comment beside the same spelling in template text.
- A head-only bracket construction with different semantic receivers, nested collection and map delimiters, and an empty map.
- An HTML body containing an interpolated attribute and a nested Markdown template. Verify Moth owns the interpolation and guest captures do not consume it.
- CSS stylesheet rules and inline declarations, including a nested contribution.
- Moth Markdown headings, emphasis, inline code, Moth-aware links and IDs. Verify unsupported CommonMark forms do not receive structural captures.
- A Moth code listing containing bracket constructions, plus a listing in another supported language. A listing never becomes live outer interpolation.
- Every literal supported `$code` language and alias, generic code, plain text, raw/escaped bodies and balanced notes.
- Two sibling bodies of the same guest language, with an unclosed child in one. Verify unrelated siblings do not merge or lose highlighting.
- An implicit .mtf body with nested default Markdown and an explicit formatter override.
- A MON implicit/explicit root, nested records, sequences and maps, Unicode escapes and forbidden source-only expressions.
- An incomplete statement, missing arm separator, missing bracket and partially typed directive. Use final syntax decisions to select valid recovery witnesses.
- Edits opening, closing or changing a formatter body, including deletion. Compare incremental and fresh tree/capture results.

## Integration and release checks

Test native and Wasm parser generation from the same source, exact CST shapes, query compilation, fixture capture ranges, deterministic edit replay and missing asset handling. Test all three file types and guest language installation in VS Code, Zed, Neovim and Emacs. Validate real editor rendering, not just the shared query output. Include a representative large source file and MTF page for cold and incremental parse evidence.

Compiler integration tests must use a real saved-source report and the agreed `--mon` contract when available. Include success, source diagnostics, source-less failures, related sites, discovery stops, stale edits, Unicode/CRLF, cancelled or failed processes and unusual file paths. The client must not show old diagnostics as current. Missing compilers must not disable syntax features.

Choose exact test commands, versions and fixture formats when a runnable owner exists. A future test must have a concrete wrong outcome it would detect. Follow the repository's [testing guide](../../docs/developer-docs/style-guide/testing.mtf). Record actual results and limitations at each milestone in the PR rather than claiming tests that have not run.
