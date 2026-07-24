-- Test duplicate function names should be detected

simple_function |number Int| -> Int:
    return number + 1
;

simple_function |value Int| -> Int:
    return value + 2
;

result = simple_function(42)
["Result: " result]
