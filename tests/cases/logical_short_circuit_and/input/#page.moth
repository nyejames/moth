-- Prove short-circuit behavior by surfacing RHS invocation count.

rhs_and |calls ~Int| -> Bool:
    calls = calls + 1
    return true
;

lhs = false
calls ~= 0
value = lhs and rhs_and(~calls)

[:logical_short_circuit_and value=[value] calls=[calls]]
