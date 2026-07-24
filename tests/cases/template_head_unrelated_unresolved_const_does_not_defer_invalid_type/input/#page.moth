-- A later unrelated constant placeholder should not defer invalid type validation.

Point = |
    x Int,
    y Int,
|

point #= Point(1, 2)
bad #= [point]
later #= 42

[: [bad]]
