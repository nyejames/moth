-- Test recursive block emission with control flow

add |x Int, y Int| -> Int:
    return x + y
;

result = add(5, 3)
io.line([: [result]])

-- Test if-else
if result > 5:
    io.line("Greater than 5")
else
    io.line("Not greater than 5")
;

-- Test loop
loop 0 to 3 |i|:
    io.line([: [i]])
;
