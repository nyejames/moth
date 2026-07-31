-- TODO(display): Replace this failure fixture once io.line([: []]) accepts Display-style struct values.
-- AST currently rejects struct values passed to io.line([: []]).

Point = |
    x Int,
    y Int,
|

point = Point(1, 2)
io.line(point)
