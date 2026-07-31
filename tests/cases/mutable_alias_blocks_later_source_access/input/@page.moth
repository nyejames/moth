-- A mutable alias blocks later access through its source binding.
data ~= 42

-- Create a mutation-capable alias of the source.
mutable_alias ~= data

-- INVALID: Access the source while the mutable alias remains live.
invalid_source_access = data

-- Keep a later alias read explicit after the invalid source access.
later_alias_read = mutable_alias
