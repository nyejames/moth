-- Relational pattern on struct with a scalar literal should produce a clear
-- relational-type diagnostic rather than a literal-type mismatch.

Person = |
    name String,
    age Int,
|

value ~= Person("Alice", 30)
result ~= ""

if value is:
    < 5 => result = "bad"
    else => result = "fallback"
;

[:should_not_reach=[result]]
