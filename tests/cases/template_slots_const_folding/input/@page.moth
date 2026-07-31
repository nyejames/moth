single_slot #= [: before [$slot] after]
wrapped_content #= [single_slot: this content is now wrapped]

basic_slots #= [: before [$slot("first")] in the middle [$slot("second")] afterwards]
template_using_slots #= [basic_slots:[$insert("first"): This goes into the first slot][$insert("second"): This goes into the second slot]]

io.line([: [wrapped_content]])
io.line([: [template_using_slots]])
