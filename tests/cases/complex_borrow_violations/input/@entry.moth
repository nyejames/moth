-- Test file: Complex borrow violations (should fail)
-- Tests that complex borrowing patterns are properly rejected

-- Test nested borrowing with mutable conflict
data ~= 42
ref1 = data
ref2 = ref1

-- This should fail: trying to create mutable access while shared references exist
mut_ref ~= data  -- ERROR: shared/mutable conflict

-- Use the shared references after the mutable borrow to keep them live
result = ref1 + ref2