import @helper { Counter as C }

run || -> String:
    c = C(0)
    result = c.tick()
    return "source_type_alias_visibility_ok"
;

io.line([: [run()]])
