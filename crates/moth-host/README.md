# moth-host

`moth-host` owns the host contract vocabulary that the Moth compiler and native runtimes must agree on. It holds the semantic host descriptors, canonical binding identities, import and export requirements, explicit export selection and pure-Rust preflight shared by the compiler and native runtimes.

The crate has no dependencies. It holds no Wasmtime type and no compiler-internal type. The compiler owns registry conversion, package validation and host-manifest production. It re-exports this crate as `moth::host` so compiler clients use the same Rust types through either path.

Use `moth_host` directly when code must read host descriptors and requirements without depending on the compiler, such as a native Wasm runtime. The [host package reference](../../docs/developer-docs/project-builders/host-packages.mtf) owns the accepted binding profile.
