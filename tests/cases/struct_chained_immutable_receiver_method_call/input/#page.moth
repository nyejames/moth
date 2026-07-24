Counter = |
    count Int,
|

bumped |this Counter| -> Counter:
    return Counter(this.count + 1)
;

c = Counter(1)
[:struct_chained_immutable_receiver_method_call value=[c.bumped().bumped().count]]
