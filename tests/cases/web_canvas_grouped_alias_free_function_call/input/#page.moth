import @web/canvas { get_canvas as get, Canvas2d as Ctx, context_2d as get_ctx, fill_rect as paint }

run |id String| -> String, Error!:
    canvas_ref = get(id)!
    ctx ~Ctx = get_ctx(canvas_ref)!
    paint(~ctx, 0.0, 0.0, 25.0, 25.0)
    return "alias_ok"
;

result = run("game") catch:
    then "error"
;
io.line([: [result]])
