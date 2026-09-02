-- Regression: mutable RHS short-circuit call with later plain local reuse.

rhs_plain |calls ~Int| -> Bool:
    calls = calls + 1
    return true
;

lhs = false
calls ~= 0
value = lhs and rhs_plain(~calls)
after = calls

[:short_circuit_mutable_rhs_later_plain_use value=[value] after=[after]]
