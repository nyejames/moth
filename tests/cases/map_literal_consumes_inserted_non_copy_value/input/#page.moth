-- A map literal consumes an inserted non-copy value; a later use is invalid.
value ~= "hello"
scores ~{String = String} = {"key" = value}
io.line([: [value]])
