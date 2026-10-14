from pathlib import Path

for name in [
    'crates/lappet/roadmap.md',
    'docs/src/developer-docs/wasm-optimisation/@page.moth',
    'docs/src/developer-docs/wasm-optimisation/wasm-optimisation.mtf',
    'docs/src/docs/lappet/lappet.mtf',
]:
    path = Path(name)
    text = path.read_text().replace('programmes', 'programs').replace('programme', 'program')
    text = text.replace('including `just\nvalidate-full`', 'including\n`just validate-full`')
    path.write_text(text)

path = Path('docs/src/docs/packages/core/@page.moth')
text = path.read_text()
old = '    preludes, Moth exposes only a small always-available surface such as `io`.'
assert text.count(old) == 1
text = text.replace(old, '    preludes, Moth reserves only a small implicit namespace surface such as `io`.')
path.write_text(text)

path = Path('docs/src/docs/packages/core/prelude/prelude.mtf')
text = path.read_text()
old = 'The builder provides the prelude. The HTML builder registers the `io` namespace alias through the Core prelude registration. The prelude surface forms the minimal universal starting point. Builders must provide it, but the actual implementations live in specific Core packages such as `@core/io`.'
assert text.count(old) == 1
text = text.replace(old, '''The compiler defines the prelude's meaning and reserved names. Builders use the
canonical registration, so `io` remains reserved even when a custom environment
implements none of its operations. Namespace visibility does not supply an
executable host capability. Calls still require the selected target's documented
implementation or unsupported outcome.

Official environments provide the Core catalogue under its capability contracts.
Custom builders may implement any subset for local code without claiming official
Core certification. Merely having the unused prelude does not make a managed
package Core-dependent. The @../../../lappet (Lappet) reference explains the
separate package-admission policy.''')
path.write_text(text)

path = Path('docs/src/docs/packages/package-origins-and-backing.mtf')
text = path.read_text()
old = '@../../lappet (Lappet) for scope and the package-manager design.'
assert text.count(old) == 1
path.write_text(text.replace(old, '@../lappet (Lappet) for scope and the package-manager design.'))
