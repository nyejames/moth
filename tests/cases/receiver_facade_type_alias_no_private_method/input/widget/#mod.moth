import @./impl { Counter, make_counter_impl }

export:
    import @./impl { Counter }

    PublicCounter as Counter

    make_counter || -> PublicCounter:
        return make_counter_impl()
    ;
;
