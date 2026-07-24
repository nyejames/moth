-- Validate struct/collection semantics through rendered values.

Point = |
    x Int,
    y Int,
|

Person = |
    name String,
    age Int,
|

render_structs_and_collections || -> String:
    p1 = Point(10, 20)
    person = Person("Alice", 30)

    p2 ~= Point(0, 0)
    p2.x = 5
    p2.y = 10

    empty {Int} = {}
    numbers = {1, 2, 3, 4, 5}
    names = {"Alice", "Bob", "Charlie"}
    matrix = {{1, 2}, {3, 4}}

    empty_len = empty.length()
    numbers_len = numbers.length()
    first_name = names.get(0) catch:
        then "none"
    ;
    matrix_rows = matrix.length()

    return [:structs_and_collections p1=[p1.x]-[p1.y] person=[person.name]-[person.age] p2=[p2.x]-[p2.y] lens=[empty_len]-[numbers_len]-[matrix_rows] first_name=[first_name]]
;

[:[render_structs_and_collections()]]
