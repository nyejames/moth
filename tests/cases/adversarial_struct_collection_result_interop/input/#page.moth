Point = |
    x Int,
    y Int,
|

load_index |items ~{Int}, idx Int| -> Int, Error!:
    return items.get(idx)!
;

point = Point(1, 2)
values ~= {point.x, point.y, 3}
~values.set(2, 8) catch:
;
selected = load_index(~values, 1) catch:
    then 0
;
io.line([: selected=[selected]])
io.line([: length=[values.length()]])
