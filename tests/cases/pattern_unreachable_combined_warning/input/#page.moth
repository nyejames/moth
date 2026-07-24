value = 1
result ~= "unset"

if value is:
    1 => result = "one"
    1 => result = "one again"
    captured => result = "captured"
    2 => result = "two"
    3 => result = "three"
    else => result = "other"
;

[:pattern_unreachable_combined_warning result=[result]]
