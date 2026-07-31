-- Relational pattern on collection should fail

value ~= {1, 2, 3}
result ~= ""

if value is:
    < {4, 5} => result = "bad"
    else => result = "fallback"
;

[:should_not_reach=[result]]
