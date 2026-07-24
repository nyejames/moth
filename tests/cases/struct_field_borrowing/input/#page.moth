Person = |
    name String,
    age Int,
|

person ~= Person("Alice", 30)

name_ref = person.name
age_ref = person.age

io.line([: borrowed-person=[name_ref] age=[age_ref]])
person.age = person.age + 1
io.line([: mutated-age=[person.age]])
