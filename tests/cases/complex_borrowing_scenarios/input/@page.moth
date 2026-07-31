-- Test file: Complex borrowing scenarios (should pass)
-- Tests combinations of nested borrowing, field access, and templates

-- Initialize complex data structure (simulated)
user_name = "Bob"
user_score ~= 100
user_active = true

-- Create nested borrowing chains
name_ref1 = user_name
name_ref2 = name_ref1
name_ref3 = name_ref2

-- Use in templates (should not interfere with borrowing)
welcome_msg = [:[name_ref1] welcome]
status_msg = [:[name_ref2] is active]

-- After template usage, continue borrowing chain
name_ref4 = name_ref3

-- Test complex control flow with borrowing
if user_active:
    -- Create local references within scope
    local_name_ref = name_ref4
    local_score_ref = user_score
    
    -- Use references in templates within scope
    scope_msg = [:[local_name_ref] has score [local_score_ref]]
    
    -- Modify mutable data within scope
    user_score = user_score + 10
;

-- After scope, original references should still work
final_name = name_ref1
final_score = user_score

-- Test sequential mutable access after shared references are done
user_score_mut1 ~= user_score
user_score_mut1 = user_score_mut1 + 5

-- After last use, another mutable access should work
user_score_mut2 ~= user_score
user_score_mut2 = 200

-- Final template usage should work with all data
summary = [:[user_name] final summary]
