-- Verifies that the entry start body sees all resolved top-level declarations after sort.
-- Calls format_greeting from a separate file, proving dep sort ordered all declarations
-- before start body lowering.

import @greeting { format_greeting }

result = format_greeting("World")
io.line([: entry_start_sees_sorted_declarations greeting=[result]])
