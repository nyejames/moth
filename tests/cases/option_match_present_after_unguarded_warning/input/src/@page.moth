value Int? = 5
result ~= "unset"

if value is:
    |amount| => result = [:present=[amount]]
    5 => result = "five"
    none => result = "missing"
;

io.line([:option_match_present_after_unguarded_warning result=[result]])
