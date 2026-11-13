# moth-host

`moth-host` owns the host contract vocabulary that the Moth compiler and native runtimes must agree on. It holds the manual descriptor types a builder uses to describe one Rust host function: the closed V1 type set, one optional layer and the builtin `Error!` channel.

The crate has no dependencies. It holds no Wasmtime type and no compiler-internal type. The compiler owns everything semantic, including converting a descriptor to its external-package registry specification and validating a package, and it re-exports this crate as `moth::host` so compiler clients use the same Rust types through either path.

Use `moth_host` directly when code must read host descriptors without depending on the compiler, such as a native Wasm runtime. The [host package reference](../../docs/developer-docs/project-builders/host-packages.mtf) owns the accepted binding profile.
