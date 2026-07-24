import @helper { Counter, alias_of, fresh_counter, bump }

describe |value Counter| -> String:
    return [:facade_reexport_effect_summary_success alias-step=[value.step]]
;

-- A re-exported return alias stays readable while live.
c ~= Counter(5)
a = alias_of(c)
summary_prefix = describe(a)

-- After the alias reaches final use, the caller may mutate the original place.
c.step = 9

-- A re-exported fresh return permits independent caller mutation.
fresh ~= fresh_counter()
fresh.step = 7

-- A re-exported mutable parameter succeeds with authored `~` and the mutation is observed.
bump(~c)
[:[summary_prefix] after-mutate=9 fresh-step=[fresh.step] bumped=[c.step]]
