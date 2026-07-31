-- The shared alias reaches its final use before mutation-capable rebinding.

data ~= 42
shared = data
io.line([: borrow_conflict_resolved_by_reordering shared-alias=[shared]])
mutable_alias ~= data
mutable_alias = 7
io.line([: borrow_conflict_resolved_by_reordering mutable-alias=[mutable_alias]])
