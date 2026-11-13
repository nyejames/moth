;; Passes one i64 token argument through to a host import.
(module
  (import "host" "accept" (func $accept (param i64)))

  (func (export "pass") (param $token i64)
    (call $accept (local.get $token))))
