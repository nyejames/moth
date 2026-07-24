-- Test file: String memory management with borrow checker
-- Tests requirement 7.2: borrow checker integration with string memory management

-- Basic string slice declaration and assignment
message = "Hello, world!"

-- String variable usage (should be fine)
greeting = message

-- Mutable string declaration
buffer ~= [:initial content]

-- String mutation (should be fine)
buffer = [:updated content]

-- Multiple string references (should be allowed)
ref1 = message
ref2 = message