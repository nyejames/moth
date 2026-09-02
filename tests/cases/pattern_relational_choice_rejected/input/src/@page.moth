-- Relational pattern on choice should fail

Status :: Ready, Loading;

value ~= Status::Ready
result ~= ""

if value is:
    < Status::Loading => result = "bad"
    else => result = "fallback"
;

[:should_not_reach=[result]]
