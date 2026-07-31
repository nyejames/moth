import @./helpers as helpers
import @./helpers { Box as ImportedBox, Status as ImportedStatus }

value = helpers.identity(21)
box ImportedBox of Int = ImportedBox(value = value)
status ImportedStatus = ImportedStatus::Ready

status_label |input ImportedStatus| -> String:
    if input is:
        ImportedStatus::Ready => return "ready"
        Failed => return "failed"
    ;
;

label = status_label(status)

[:semantic dispatch value=[box.value] status=[label] label=[box.label]]
