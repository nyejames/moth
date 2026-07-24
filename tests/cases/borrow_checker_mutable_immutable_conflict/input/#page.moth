-- Test file: Mutable and immutable borrow conflict (should fail borrow checker)

-- Declare a mutable variable
data ~= 42

-- Create immutable reference
immutable_ref = data

-- Try to create mutable reference while immutable reference exists
mutable_ref ~= data

-- Use both references (this should trigger borrow checker error)
-- Because if immutable_ref is not used after the mutable_ref, 
-- then its safe. But since both are used, an error should occur
result = immutable_ref