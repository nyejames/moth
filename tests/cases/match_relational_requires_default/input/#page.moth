-- Relational patterns without default should fail

value ~= 5
result ~= ""

if value is:
    < 0 => result = "negative"
    >= 0 => result = "non-negative"
;

[:should_not_reach=[result]]
