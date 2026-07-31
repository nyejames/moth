-- Test case: immutable aliasing remains valid while another immutable alias is live
-- Plain '=' creates another immutable alias, not a move, so this should compile.

data ~= [:important data]

data_ref = data
another_ref = data

result = another_ref
