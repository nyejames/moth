-- Complex multi-file module test with various features
import @utils/logger { log }
import @utils/logger { init_logger }
import @utils/calculator { compute }
import @utils/calculator { init_calculator }
import @data/storage {save, load}
import @data/storage { init_storage }

io.line("Starting complex module test")
init_logger()
init_calculator()
init_storage()

-- Test host function with imported module interaction
log("Initializing calculator")
result = compute(10, 5)
log("Calculation complete")

-- Test storage operations
save("result", result)
saved_value = load("result")
io.line([: Saved and loaded value: [saved_value]])

io.line("Complex module test complete")
