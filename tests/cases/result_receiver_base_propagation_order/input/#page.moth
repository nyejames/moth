User = |
    score Int,
|

Bag = |
    values {Int},
|

read_score |this User| -> Int:
    return this.score
;

method_left || -> Int:
    io.line("method-left")
    return 1
;

collection_left || -> Int:
    io.line("collection-left")
    return 2
;

load_user || -> User, Error!:
    return! Error("missing user")
;

load_bag || -> Bag, Error!:
    return! Error("missing bag")
;

method_compute || -> Int, Error!:
    return method_left() + load_user()!.read_score()
;

collection_compute || -> Int, Error!:
    return collection_left() + load_bag()!.values.length()
;

method_value = method_compute() catch:

    then 0

;
collection_value = collection_compute() catch:
    then 0
;
io.line([:receiver propagation method=[method_value] collection=[collection_value]])
