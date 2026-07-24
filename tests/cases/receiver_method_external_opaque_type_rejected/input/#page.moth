import @core/time { Duration, duration_from_seconds }

DURATION_LABEL must:
    label |This| -> String
;

label |this Duration| -> String:
    return [:duration=[this.as_seconds()]]
;

Duration must DURATION_LABEL

duration = duration_from_seconds(2.5)
[:receiver_method_external_opaque_type_rejected [duration.label()]]
