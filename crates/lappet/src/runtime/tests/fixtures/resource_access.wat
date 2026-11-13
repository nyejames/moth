;; Guest passes resource tokens through as i64. Plain core Wasm, not compiled Moth.
(module
  (import "host" "shared" (func $shared (param i64)))
  (import "host" "exclusive" (func $exclusive (param i64)))
  (import "host" "shared_both" (func $shared_both (param i64 i64)))
  (import "host" "exclusive_then_shared" (func $exclusive_then_shared (param i64)))
  (import "host" "panic_holding" (func $panic_holding (param i64)))

  (func (export "shared") (param $token i64)
    (call $shared (local.get $token)))
  (func (export "exclusive") (param $token i64)
    (call $exclusive (local.get $token)))
  (func (export "shared_both") (param $first i64) (param $second i64)
    (call $shared_both (local.get $first) (local.get $second)))
  (func (export "exclusive_then_shared") (param $token i64)
    (call $exclusive_then_shared (local.get $token)))
  (func (export "panic_holding") (param $token i64)
    (call $panic_holding (local.get $token))))
