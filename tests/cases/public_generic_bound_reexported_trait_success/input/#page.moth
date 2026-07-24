import @sub { PUBLIC_DISPLAY, identity }

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

Label must PUBLIC_DISPLAY

label = identity(Label("reexported"))
[:public_generic_bound_reexported_trait_success [label.text]]
