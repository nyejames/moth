import @helper { Counter, alias_of }

c ~= Counter(5)
a = alias_of(c)
c.step = 9
[:facade_reexport_return_alias_live_blocks_mutation after-mutate=[a.step]]
