Pair = |
    x Int,
    y Int,
|

pair ~= Pair(10, 20)

x_ref = pair.x
y_ref = pair.y
sum = x_ref + y_ref

pair.x = sum
pair.y = pair.y + 1
io.line([: disjoint_field_borrows final-fields=[pair.x] [pair.y]])
