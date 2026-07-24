value Int? = 42
result ~= "unset"

if value is |m|:
    result = [:single=[m]]
;

if value is:
    |n| if n is 42 => result = [:guard=[n]]
    |n| => result = [:full=[n]]
    none => result = "missing"
;

[:option_present_capture_success result=[result]]
