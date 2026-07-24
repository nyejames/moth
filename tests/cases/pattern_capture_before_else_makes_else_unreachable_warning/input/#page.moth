value ~= "hello"
result ~= "unset"

if value is:
    captured => result = captured
    else => result = "fallback"
;

[:pattern_capture_before_else_makes_else_unreachable_warning result=[result]]
