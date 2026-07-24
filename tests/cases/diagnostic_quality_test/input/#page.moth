-- Test diagnostic message quality for various error types

-- Syntax error
["unclosed string

-- Type error  
number = 42
io.line([: [number + "string"]])

-- Undefined variable
io.line([: [undefined_variable]])

-- Host function error
io.line([: [123]])  -- Wrong argument type