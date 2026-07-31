-- A postfix access ending immediately after the authored `.` must report the
-- missing member name at the owning dot boundary instead of a generic
-- unexpected-token or receiver-start fallback.
Point = |
    x Int,
    y Int,
|

point = Point(x = 1, y = 2)
value = point.
