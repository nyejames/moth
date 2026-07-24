pair_ok |name String| -> String, Int, Error!:
    return name, 2
;

forward |name String| -> String, Int, Error!:
    first, count = pair_ok(name)!
    return first, count
;

first, count = forward("Ana") catch:

    then "fallback", 0

;
io.line([: [first]])
io.line([: [count]])
