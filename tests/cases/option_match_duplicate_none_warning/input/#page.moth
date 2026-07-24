value Int? = none
result ~= "unset"

if value is:
    none => result = "missing"
    none => result = "missing again"
    else => result = "present"
;

io.line([:option_match_duplicate_none_warning result=[result]])
