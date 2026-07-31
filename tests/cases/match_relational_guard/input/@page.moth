-- Relational match with guard

value ~= 5
enabled ~= true
result ~= ""

if value is:
    < 0 => result = "negative"
    <= 10 if enabled => result = "small and enabled"
    else => result = "other"
;

[:relational_guard_result=[result]]
