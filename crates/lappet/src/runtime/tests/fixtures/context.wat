;; One host import, called from the only export. Plain core Wasm.
(module
  (import "host" "touch" (func $touch))
  (import "host" "panic_after_touch" (func $panic_after_touch))

  (func (export "entry")
    (call $touch))

  (func (export "panic_entry")
    (call $panic_after_touch)))
