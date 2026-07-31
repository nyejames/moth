value ~= 2
allow ~= false
result ~= "unset"

if value is:
    2 if allow => result = "blocked"
    2 => result = "allowed"
    else => result = "other"
;

[:match_guard_literal_success result=[result]]
