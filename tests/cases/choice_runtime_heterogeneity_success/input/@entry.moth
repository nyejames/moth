Label = |
    text String,
|

Button = |
    text String,
|

Renderable ::
    LabelItem | value Label |,
    ButtonItem | value Button |,
;

render |item Renderable| -> String:
    if item is:
        LabelItem(value as label) => return label.text
        ButtonItem(value as button) => return button.text
    ;
;

first = Renderable::LabelItem(Label("label"))
second = Renderable::ButtonItem(Button("button"))

io.line([:choice_runtime_heterogeneity_success [render(first)] [render(second)]])
