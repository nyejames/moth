-- A reactive parameter preserves source identity without granting mutation permission.
-- Assigning through the parameter is rejected because the binding stays immutable.

render_count |count $Int| -> String:
    count = 2
    return [:count=[$(count)]]
;

count $Int = 1

[:reactive_parameter_mutation_permission_rejected [render_count(count)]]
