-- Assertion messages must be string literals; reactive template messages never reach the backend.

count $Int = 1

assert(true, [:message [$(count)]])
