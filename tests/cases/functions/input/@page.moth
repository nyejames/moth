-- Validate function declarations/calls by rendering computed results.

add |x Int, y Int| -> Int:
    return x + y
;

multiply |x Int, y Int| -> Int:
    return x * y
;

greet |name String| -> String:
    return "Hello, " + name
;

factorial |n Int| -> Int:
    if n <= 1:
        return 1
    ;
    return n * factorial(n - 1)
;

sum = add(5, 3)
product = multiply(4, 6)
message = greet("World")
fact5 = factorial(5)

[:functions sum=[sum] product=[product] message=[message] fact5=[fact5]]
