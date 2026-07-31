-- Propagation (!) inside a template slot value within a Result-returning function.

make_content || -> String, Error!:
    return "ok_value"
;

render || -> String, Error!:
    return [:propagate_template content=[make_content()!]]
;

result = render() catch:

    then "template_fallback"

;
io.line([: [result]])
