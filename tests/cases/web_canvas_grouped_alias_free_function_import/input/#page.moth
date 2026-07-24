import @web/canvas { Canvas2d, get_canvas, context_2d, fill_rect as draw }

draw_rectangle || -> String, Error!:
    canvas = get_canvas("game_canvas")!
    context ~Canvas2d = context_2d(canvas)!
    draw(~context, 0.0, 0.0, 10.0, 10.0)
    return "alias_free_function_import_ready"
;

status = draw_rectangle() catch:
    then "alias_free_function_binding_unavailable"
;

io.line([: [status]])
