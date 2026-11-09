-- Test file: Invalid reassignment/shadowing attempts (should fail)

-- Declare a mutable variable
test_var ~= 42

-- INVALID: Trying to use ~= for reassignment (this should be an error)
-- ~= should only be used for initial declarations, not reassignments
test_var ~= 84