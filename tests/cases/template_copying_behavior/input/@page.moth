-- Test file: Template copying behavior (should pass)
-- Tests that template usage creates copies, not borrows

-- Initialize data
data = "world"
number = 42

-- Use variables in templates - should create copies, not borrows
template1 = [:[data] world!]
template2 = [:[number] is the answer]

-- After template usage, original variables should still be available for borrowing
data_ref = data
number_ref = number

-- Use the references
result1 = data_ref
result2 = number_ref

-- Create more templates with the same variables - should still work
template3 = [:[data] updated]
template4 = [:[number] is new]

-- Original data should still be borrowable after template usage
final_ref = data

io.line([:template_copying result=[result1] number=[result2] final=[final_ref] [template1] [template2] [template3] [template4]])
