Counter = |
    value Int,
|

label |this Counter| -> String:
    return "counter"
;

import @helper { Badge }

counter = Counter(1)
badge = Badge(2)
[:receiver_same_file_and_imported_same_name_success [counter.label()] [badge.label()]]
