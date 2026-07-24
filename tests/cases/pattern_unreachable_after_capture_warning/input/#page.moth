value ~= "hello"
result ~= "unset"

if value is:
    captured => result = captured
    "one" => result = "one"
    "two" => result = "two"
    else => result = "other"
;

[:pattern_unreachable_after_capture_warning result=[result]]
