import @web/canvas { get_canvas, Canvas2d, context_2d, fill_rect }

run |id String| -> String, Error!:
    canvas_ref = get_canvas(id)!
    ctx ~Canvas2d = context_2d(canvas_ref)!
    fill_rect(~ctx, 0.0, 0.0, 40.0, 40.0)
    return "free_function_ok"
;

result = run("game") catch:
    then "error"
;
io.line([: [result]])
