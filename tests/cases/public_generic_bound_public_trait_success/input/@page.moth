import @sub { PUBLIC_DISPLAY, identity }

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

Label must PUBLIC_DISPLAY

label = identity(Label("public"))
[:public_generic_bound_public_trait_success [label.text]]
