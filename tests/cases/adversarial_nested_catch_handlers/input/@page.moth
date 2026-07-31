parse_user |name String| -> String, Float, Error!:
    if name is "":
        return! Error("name is empty")
    ;

    return name, 1.5
;

render_user |name String| -> String:
    user, score = parse_user(name) catch |err|:
        io.line([: nested-catch-code=[err.code]])
        then "guest", 0.0
    ;

    return [:[user]-[score]]
;

output = render_user("")
io.line([: nested-catch-user=[output]])
