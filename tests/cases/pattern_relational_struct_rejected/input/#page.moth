-- Relational pattern on struct should fail

Person = |
    name String,
    age Int,
|

value ~= Person("Alice", 30)
result ~= ""

if value is:
    < Person("Bob", 25) => result = "bad"
    else => result = "fallback"
;

[:should_not_reach=[result]]
