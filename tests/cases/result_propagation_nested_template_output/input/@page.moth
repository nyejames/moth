-- Prove result propagation/fallback remains correct when values flow into a template.

make_label |name String| -> String, Error!:
    return name
;

render_label || -> String:
    label = make_label("hello") catch:
        then "fallback_label"
    ;
    return [:propagation_template label=[label]]
;

[:[render_label()]]
