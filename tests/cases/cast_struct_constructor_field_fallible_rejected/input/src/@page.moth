-- Plain `cast` with a fallible source in a struct constructor field is rejected.

Point = |
    x Int,
    y Int,
|

value = Point(x = cast "1", y = 0)
