-- Prove short-circuit behavior by surfacing RHS invocation count.

rhs_or |calls ~Int| -> Bool:
    calls = calls + 1
    return false
;

lhs = true
calls ~= 0
value = lhs or rhs_or(~calls)

[:logical_short_circuit_or value=[value] calls=[calls]]
