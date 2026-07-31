Maybe type T ::
    Some | value T |,
    Empty,
;

left Maybe of Int = Maybe::Some(1)
right Maybe of Int = Maybe::Some(1)
other Maybe of Int = Maybe::Some(2)

if left is right:
    io.line("same")
else
    io.line("different")
;

if left is other:
    io.line("unexpected")
else
    io.line("not same")
;
