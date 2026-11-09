-- A live shared alias prevents mutation-capable rebinding.
data = "test data"
shared_alias = data

-- INVALID: The shared alias is still live when mutation-capable access begins.
mutable_alias ~= data

-- Keep the shared alias use after the conflict explicit.
later_shared_alias_use = shared_alias
