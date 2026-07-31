Cell = |
    label String,
|

prefix |s String| -> String:
    return s + "|"
;

maybe String? = cast 1
labels {String} = { cast 'A', cast true }
keyed {String = Int} = { cast 1 = 100 }
valued {Int = String} = { 2 = cast 30 }
cell = Cell(label = cast 40)
step = prefix(cast 50)
result String = if true then cast 60 else "none"

maybe_text = if maybe is |value| then value else "none"

[: target propagation: [maybe_text]|[cell.label]|[step]|[result]]
