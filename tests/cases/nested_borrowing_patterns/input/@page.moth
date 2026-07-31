-- Test file: Nested borrowing patterns (should pass)
-- Tests that borrowing from borrowed values works correctly

-- Initialize data
original_data = "hello world"

-- Create first level of borrowing
first_ref = original_data

-- Create nested borrowing (borrow from a borrow)
second_ref = first_ref
third_ref = second_ref

-- All references should be usable
result1 = first_ref
result2 = second_ref  
result_three = third_ref

-- Use the results to ensure they're valid
final_result = result1

-- Test with mutable data
mutable_data ~= 42

-- Create shared reference to mutable data
shared_ref = mutable_data

-- Create nested shared references
nested_shared1 = shared_ref
nested_shared2 = nested_shared1

-- All should be usable
sum = shared_ref + nested_shared1 + nested_shared2

-- After last use of shared references, mutable access should work
mutable_data = mutable_data + 10