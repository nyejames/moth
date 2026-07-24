-- Test file: Function calls with borrow checker
-- Tests requirement 7.3: borrow checker integration with function calls

simple_function |value ~Int| -> Int:
    result = value + 1
    return result
;

string_function |text String| -> String:
    return [text]
;

number ~= 42
result = simple_function(~number)

message = "hello"
echo = string_function(message)

value1 ~= 10
value2 = simple_function(~value1)
