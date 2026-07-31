-- Generic struct constructor through a facade import, using named arguments
-- and a defaulted field that is omitted at the call site.

import @utils { Wrapper }

wrapper Wrapper of Int = Wrapper(value = 42)

io.line([: [wrapper.value]])
io.line([: [wrapper.label]])
