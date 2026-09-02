-- Exercise control-flow semantics and surface outcomes in the final HTML artifact.

x = 10
if_result ~= "unset"
if x > 5:
    if_result = "gt5"
else
    if_result = "le5"
;

y = 15
nested_result ~= "unset"
if y > 10:
    if y < 20:
        nested_result = "between"
    else
        nested_result = "outside"
    ;
else
    nested_result = "below"
;

value = 42
match_result ~= "unset"
if value is:
    0 => match_result = "zero"
    42 => match_result = "forty-two"
    else => match_result = "other"
;

sum ~= 0
loop 1 to 5 |i|:
    sum = sum + i
;

total ~= 0
loop 1 to 5 |num|:
    total = total + num
;

[:control_flow if=[if_result] nested=[nested_result] match=[match_result] sum=[sum] total=[total]]
