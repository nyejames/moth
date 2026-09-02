ResultShape type T, E ::
    Ok | value T |,
    Err | error E |,
;

get_value ResultShape of String, Error = ResultShape::Ok("hello")

if get_value is:
    Ok(value) => io.line([: [value]])
    Err(error) => io.line([: [error.message]])
;

[: generic choice match success]
