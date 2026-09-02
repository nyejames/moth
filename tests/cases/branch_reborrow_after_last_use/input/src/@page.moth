-- A branch-local mutable alias can be rebound after its final use.

data = "shared_data"
some_condition = true

first_alias ~= data

if some_condition:
    io.line([: branch_reborrow_after_last_use first-alias=[first_alias]])
    second_alias ~= data
    io.line([: branch_reborrow_after_last_use second-alias=[second_alias]])
;
