-- Test host functions with control flow structures
condition = true
counter ~= 0

io.line("Starting control flow test")

if condition:
    io.line("Condition is true")
    counter = counter + 1
    io.line([: Counter incremented: [counter]])
else
    io.line("Condition is false")
;

-- Test with variables and expressions
x = 10
y = 5

if x > y:
    result = x + y
    io.line([: [x] is greater: [x] + [y] = [result]])
else
    result = x - y
    io.line([: [x] is not greater: [x] - [y] = [result]])
;

io.line("Control flow test complete")
