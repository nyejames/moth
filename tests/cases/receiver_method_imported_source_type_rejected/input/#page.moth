import @./models { Label }

LABEL_TEXT must:
    text |This| -> String
;

text |this Label| -> String:
    return this.value
;

Label must LABEL_TEXT

label = Label("imported")
[:receiver_method_imported_source_type_rejected [label.text()]]
