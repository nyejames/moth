import @web/canvas { get_canvas, context_2d, Canvas2d }

uses_grouped_canvas_imports || -> String, Error!:
    canvas = get_canvas("game_canvas")!
    context ~Canvas2d = context_2d(canvas)!
    return "grouped_canvas_binding_ready"
;

status = uses_grouped_canvas_imports() catch:
    then "grouped_canvas_binding_unavailable"
;

io.line([: [status]])
