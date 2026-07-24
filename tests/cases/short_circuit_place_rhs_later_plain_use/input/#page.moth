-- Regression: place RHS short-circuit merge should not alias to the source local.

lhs = false
rhs ~= true
value = lhs and rhs
after = rhs

[:short_circuit_place_rhs_later_plain_use value=[value] after=[after]]
