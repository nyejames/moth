-- Regression: mutable RHS short-circuit call with later runtime-fragment capture.

rhs_capture |calls ~Int| -> Bool:
    calls = calls + 1
    return true
;

lhs = false
calls ~= 0
value = lhs and rhs_capture(~calls)

[:short_circuit_mutable_rhs_later_runtime_capture calls=[calls]]
