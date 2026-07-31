-- Test file: Basic variable operations that should pass borrow checker
-- Tests requirement 7.1: borrow checker integration with variable system

-- Simple variable declaration and assignment
data = "test data"

-- Immutable variable usage (should be fine)
result = data

-- Mutable variable declaration
counter ~= 0

-- Mutable variable mutation (should be fine)
counter = counter + 1

-- Multiple immutable references (should be allowed)
ref1 = data
ref2 = data