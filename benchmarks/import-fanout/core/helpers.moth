import @core/constants {max_items_per_page, default_page_offset}
import @core/constants {max_page_limit as limit, enable_cache}

compute_offset |page Int| -> Int:
    return page * max_items_per_page + default_page_offset
;

is_valid_limit |value Int| -> Bool:
    return value <= limit
;
