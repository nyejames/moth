-- Regression: mutable RHS short-circuit call with runtime capture of result and counter.

rhs_capture_dual |calls ~Int| -> Bool:
    calls = calls + 1
    return true
;

lhs = false
calls ~= 0
value = lhs and rhs_capture_dual(~calls)

[:short_circuit_mutable_rhs_runtime_capture_dual value=[value] calls=[calls]]
