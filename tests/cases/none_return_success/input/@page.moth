-- Validates that `none` is accepted as a return value when the function's
-- return slot is an explicit optional type.

make_name || -> String?:
    return none
;

name = make_name()
if name is none:
    io.line("none-return=absent")
;
