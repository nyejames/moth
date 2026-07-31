-- Test case: path-dependent mutable aliasing is valid when each mutable alias ends at its last use
-- This used to model a path-dependent ownership violation, but there is no move here.

x = "test_value"
condition = true

if condition:
    result1 ~= x
    io.line([: path_dependent_reborrow executed-path-result=[result1]])
else
    result2 ~= x
    io.line([: path_dependent_reborrow alternate-path-result=[result2]])
    io.line([: path_dependent_reborrow alternate-source=[x]])
;
