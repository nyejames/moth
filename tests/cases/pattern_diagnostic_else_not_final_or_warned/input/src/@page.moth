-- 'else =>' must not appear before any pattern arm.

value ~= 1
result ~= ""

if value is:
    else => result = "first"
    1 => result = "second"
;

[:should_not_reach=[result]]
