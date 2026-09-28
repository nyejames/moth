# Contributing to Moth

Moth is an early-stage project. Right now, things are moving very fast. While core design is shifting less and less, there is still a lot of foundational work to do.

This project is **closed to contributions** until its reached more of a hardening, optimisation and "extra feature proposals" phase. 

Get in touch if you're interested in discussing the project. Please don't yeet PRs at the repo, they won't be accepted.

This codebase uses AI in a measured capacity. See the [AI policy](./HUMANS.md) for more info about what this project considers acceptable AI generated code.

---

## Language and current support

- [User-facing documentation](docs/src/docs/)
- [Progress matrix](docs/src/docs/progress/@page.moth)
- [Roadmap](docs/roadmap/roadmap.md)

## Compiler and memory design

More technical details about the language, compiler and build system.

- [Compiler design overview](docs/compiler-design-overview.md)
- [Build system overview](docs/build-system-design.md)
- [Memory-management overview](docs/src/developer-docs/memory-management/overview.mtf)
- [Design scope](docs/src/docs/design-scope/)
- [Repository index](index.md)

## Development standards

- [Code style](docs/src/developer-docs/style-guide/style-guide.mtf)
- [Testing standards](docs/src/developer-docs/style-guide/testing.mtf)
- [Validation gates](docs/src/developer-docs/style-guide/validation.mtf)
- [Benchmark and profiling workflow](benchmarks/README.md)

See the hosted docs site for the compiled versions of these mtf files.

## Testing

The complete testing policy is in [testing.mtf](docs/src/developer-docs/style-guide/testing.mtf).

Run the integration suite during iteration with:

```sh
cargo run --quiet -- tests
```

## Command guide

- `cargo run build docs --release` - final gate for a strictly documentation-only change
- `cargo run --quiet -- tests` - fast integration-suite iteration
- `cargo run --quiet -- tests --case <id> [--backend <id>]` - exact focused integration run
- `cargo run --quiet -- tests --tag <tag> [--tag <tag>]` - logical AND tag selection
- `cargo run --quiet -- tests --list [filters]` - list selected suite metadata without compiling
- `cargo run --quiet -- tests --audit` - validate and write the complete suite inventory
- `just validate` - required final gate for code-bearing changes
- `just bench-check` - performance sanity check (does not write any files)
- `just bench` - intentional benchmark-history recording (updates the benchmark log)
- `just bench-report` - inspect local benchmark history
- `just profile-case <case> [filter]` - profile a selected benchmark case

