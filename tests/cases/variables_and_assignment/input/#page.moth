-- Test variable declarations and assignments in JavaScript backend
-- Validates that the JS backend correctly handles let declarations
-- and reassignments with proper scoping

-- First assignment should use 'let'
x ~= 10
io.line([: x = [x]])

-- Second assignment should be reassignment
x = 20
io.line([: x = [x]])

-- Multiple variables
a = 1
b = 2
c = 3
io.line([: a = [a], b = [b], c = [c]])

-- String variables
name = "Alice"
greeting = "Hello"
io.line([: [greeting + ", " + name]])

-- Boolean variables
flag1 = true
flag2 = false
io.line([: flag1 = [flag1], flag2 = [flag2]])
