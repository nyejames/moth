outer ~= 1
result ~= "unset"

if outer is:
    1 =>
        inner ~= 2
        if inner is:
            2 => result = "one-two"
            else => result = "one-other"
        ;
    else => result = "other"
;

[:match_nested_in_branches result=[result]]
