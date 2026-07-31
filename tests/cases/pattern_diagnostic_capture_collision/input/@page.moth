-- General capture pattern must not shadow an existing visible binding.

helper ||:
    io.line("helper")
;

value ~= 1
if value is:
    helper => io.line([: [helper]])
    else => io.line("other")
;
