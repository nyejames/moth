-- Regression coverage for the ExpressionKind::Coerced runtime path.
--
-- The result of add_one(4) is a non-constant Int expression (a function call
-- return value), so it cannot be constant-folded to a Float literal at parse
-- time. Instead the frontend must wrap it in a Coerced node and lower it
-- through HIR to a runtime conversion.

add_one |value Int| -> Int:
    return value + 1
;

result Float = add_one(4)

io.line("runtime int to float coercion passed")
