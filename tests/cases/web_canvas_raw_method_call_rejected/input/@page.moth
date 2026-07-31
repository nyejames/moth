import @web/canvas { get_canvas, Canvas2d, context_2d }

run |id String| -> String, Error!:
    canvas_ref = get_canvas(id)!
    ctx ~Canvas2d = context_2d(canvas_ref)!
    ~ctx.fill_rect(0.0, 0.0, 40.0, 40.0)
    return "raw_method_bad"
;

result = run("game") catch:
    then "error"
;
io.line([: [result]])
