import @./traits { DISPLAY_TEXT }
import @./models { Label }
import @./renderers { render }

display |this Label| -> String:
    return this.text
;

Label must DISPLAY_TEXT

[:generic_bound_nonlocal_receiver_method_rejected [render(Label("local"))]]
