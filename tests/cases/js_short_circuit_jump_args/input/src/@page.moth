-- Exercise short-circuit CFG merge transport through Jump.args block arguments.

rhs_probe |calls ~Int| -> Bool:
    calls = calls + 1
    return true
;

lhs = false
calls ~= 0
value = lhs and rhs_probe(~calls)

[:js_short_circuit_jump_args value=[value] calls=[calls]]
