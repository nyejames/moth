-- Relational pattern on Bool should fail

value ~= true
result ~= ""

if value is:
    < false => result = "bad"
    else => result = "fallback"
;

[:should_not_reach=[result]]
