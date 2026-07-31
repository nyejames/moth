identity type T |value T| -> T:
    return value
;

first type T |value T, fallback T| -> T:
    return value
;

int_value = identity(42)
text_value = identity("hello")
chosen = first(1, 2)
named = first(value = 3, fallback = 4)

io.line([: [int_value] [text_value] [chosen] [named]])
