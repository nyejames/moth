import @./impl { Label, make_label_impl }

export:
    import @./impl { Label }

    PublicLabel as Label

    make_label |text String| -> PublicLabel:
        return make_label_impl(text)
    ;
;
