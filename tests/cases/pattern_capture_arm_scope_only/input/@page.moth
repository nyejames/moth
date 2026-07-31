value ~= "hello"

if value is:
    captured => io.line([: [captured]])
    else => io.line("fallback")
;

-- captured should not be visible outside the arm
io.line(captured)