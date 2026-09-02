-- Validate simple branch selection and nested branch semantics.

condition = true
result ~= 0
if condition:
    result = 42
else
    result = 0
;

x = 10
nested_result ~= -1
if x > 5:
    if x > 8:
        nested_result = 1
    else
        nested_result = 2
    ;
else
    nested_result = 0
;

[:simple_if result=[result] nested=[nested_result]]
