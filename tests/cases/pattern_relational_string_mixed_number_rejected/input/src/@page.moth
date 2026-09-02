-- Relational string pattern with numeric literal should fail

value ~= "hello"
result ~= ""

if value is:
    < 5 => result = "bad"
    else => result = "fallback"
;

[:should_not_reach=[result]]
