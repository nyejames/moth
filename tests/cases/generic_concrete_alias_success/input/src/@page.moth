Box type T = |
    value T,
|

Pair type Left, Right = |
    left Left,
    right Right,
|

IntBox as Box of Int
StringBox as Box of String
NamedCount as Pair of String, Int

alias_value IntBox = Box(1)
raw Box of Int = alias_value

item StringBox = Box("text")
raw_text Box of String = item

count NamedCount = Pair("items", 4)

io.line([: [raw_text.value]])
io.line([: [count.left]])

[: concrete generic alias]
