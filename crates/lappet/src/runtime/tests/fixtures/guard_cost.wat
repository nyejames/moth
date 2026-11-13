;; One bare host call and one call that passes a resource token. Plain core Wasm.
(module
  (import "host" "bare" (func $bare))
  (import "host" "guarded" (func $guarded (param i64)))

  (func (export "bare")
    (call $bare))

  (func (export "guarded") (param $token i64)
    (call $guarded (local.get $token))))
