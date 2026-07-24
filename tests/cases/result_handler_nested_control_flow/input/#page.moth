inner |name String| -> String, Int, Error!:
    if name is "":
        return! Error("empty")
    ;

    return name, 1
;

outer |name String| -> String, Int, Error!:
    value, number = inner(name) catch |err|:
        return! err
    ;

    return value, number
;

ok_name, ok_number = outer("Ana") catch:

    then "ok-fallback", 0

;
missing_name, missing_number = outer("") catch:
    then "missing-fallback", -1
;
io.line([: [ok_name]])
io.line([: [ok_number]])
io.line([: [missing_name]])
io.line([: [missing_number]])
