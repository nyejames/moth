-- Negative regression: true branch-only move with no later use should remain inconsistent.

rhs_inconsistent |calls ~Int| -> Bool:
    calls = calls + 1
    return true
;

lhs = false
calls ~= 0
value = lhs and rhs_inconsistent(~calls)
