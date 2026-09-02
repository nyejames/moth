pair type A, B |a A, b B| -> A:
    return a
;

forward type X, Y |x X, y Y| -> X:
    return pair(x, y)
;

swap_forward type X, Y |x X, y Y| -> Y:
    return pair(y, x)
;

left Int = forward(7, "ignored")
right String = swap_forward(7, "kept")

io.line([: generic forward [left] [right]])
