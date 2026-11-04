-- Unresolved type names in struct field annotations must produce a user-visible
-- diagnostic rather than silently falling back to builtin `none`.

Bad = |
    field UnknownType,
|

bad #= Bad(1)

io.line("should not reach")
