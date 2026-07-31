render_inner |source $Int| -> String:
    return [:inner=[$(source)]]
;

render_outer |source $Int| -> String:
    return [:outer=[render_inner(source)]]
;

count $Int = 11

[:reactive_template_parameter_source_identity_success [render_outer(count)]]
