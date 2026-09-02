pair || -> String, Int, Error!:
    return "ok", 1
;

render |value String| -> String:
    return value
;

recover || -> String, Error!:
    return render(pair()!)
;

value = recover() catch:
    then "fallback"
;

io.line([: [value]])
