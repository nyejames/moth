import @web/canvas

run |id String| -> String, Error!:
    canvas_ref ~= canvas.get_canvas(id)!
    canvas.set_canvas_size(~canvas_ref, 320, 180)

    ctx ~= canvas.context_2d(canvas_ref)!
    canvas.save(~ctx)
    canvas.set_fill_style(~ctx, "#112233")
    canvas.set_stroke_style(~ctx, "white")
    canvas.set_line_width(~ctx, 2.0)
    canvas.set_line_cap(~ctx, "round")
    canvas.set_line_join(~ctx, "round")
    canvas.set_global_alpha(~ctx, 0.95)
    canvas.set_image_smoothing_enabled(~ctx, true)
    canvas.clear_rect(~ctx, 0.0, 0.0, 320.0, 180.0)
    canvas.fill_rect(~ctx, 4.0, 4.0, 80.0, 32.0)
    canvas.stroke_rect(~ctx, 4.0, 4.0, 80.0, 32.0)

    canvas.begin_path(~ctx)
    canvas.move_to(~ctx, 12.0, 70.0)
    canvas.line_to(~ctx, 52.0, 100.0)
    canvas.quadratic_curve_to(~ctx, 76.0, 120.0, 96.0, 80.0)
    canvas.arc(~ctx, 96.0, 80.0, 12.0, 0.0, 6.28)!
    canvas.close_path(~ctx)
    canvas.stroke(~ctx)

    gradient ~= canvas.create_linear_gradient(ctx, 0.0, 0.0, 120.0, 0.0)!
    canvas.add_color_stop(~gradient, 0.0, "red")!
    canvas.add_color_stop(~gradient, 1.0, "blue")!
    canvas.set_fill_gradient(~ctx, gradient)
    canvas.fill_rect(~ctx, 120.0, 8.0, 80.0, 32.0)

    canvas.translate(~ctx, 10.0, 10.0)
    canvas.rotate(~ctx, 0.1)
    canvas.scale(~ctx, 1.0, 1.0)
    canvas.reset_transform(~ctx)

    canvas.set_font(~ctx, "16px sans-serif")
    canvas.set_text_align(~ctx, "left")
    canvas.set_text_baseline(~ctx, "top")
    canvas.set_direction(~ctx, "ltr")
    canvas.set_letter_spacing(~ctx, "0px")
    canvas.set_word_spacing(~ctx, "0px")
    canvas.set_font_kerning(~ctx, "normal")
    canvas.set_text_rendering(~ctx, "geometricPrecision")
    canvas.set_lang(~ctx, "en")
    canvas.fill_text(~ctx, "Canvas", 10.0, 130.0)
    metrics = canvas.measure_text(ctx, "Canvas")
    width = canvas.text_width(metrics)

    image_data ~= canvas.create_image_data(ctx, 2, 2)!
    canvas.image_data_set_pixel(~image_data, 0, 0, 255, 0, 0, 255)!
    red = canvas.image_data_get_red(image_data, 0, 0)!
    image_width = canvas.image_data_width(image_data)
    canvas.put_image_data(~ctx, image_data, 210.0, 8.0)!

    exported = canvas.to_data_url_type(canvas_ref, "image/png")!
    context_lost = canvas.is_context_lost(ctx)
    canvas.restore(~ctx)

    return "expanded_canvas_ok"
;

result = run("game") catch:
    then "expanded_canvas_error"
;
io.line([: [result]])
