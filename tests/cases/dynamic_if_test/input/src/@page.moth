-- Validate runtime-dependent if behavior through final artifact output.

x ~= 10
y ~= 5

result ~= -1
if x > y:
    result = 1
else
    result = 0
;

another_result ~= -1
if y > 3:
    another_result = 42
else
    another_result = 24
;

[:dynamic_if result=[result] another=[another_result]]
