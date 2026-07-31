import @types { TaskResult }
import @types { format_task }

-- Const payload choice
default_success #= TaskResult::Success(42, "default")

-- Function returning choice
make_result |n Int| -> TaskResult:
    if n is 0:
        return TaskResult::Pending
    ;
    if n > 2:
        return TaskResult::Failure("too big", 99)
    ;
    return TaskResult::Success(n, "num")
;

-- Function accepting choice
handle_result |r TaskResult| -> String:
    if r is:
        Success(value, label) =>
            if value > 10:
                return "big"
            ;
            return "small"
        Failure(reason, code) => return "failed"
        Pending => return "waiting"
    ;
;

-- Helper for nested match (avoids shadowing)
describe_task |t TaskResult| -> String:
    if t is:
        Success(value, label) => return "inner_ok"
        Failure(reason, code) => return "inner_fail"
        Pending => return "inner_pending"
    ;
;

-- Loop containing match with payload captures
summary ~= ""
i ~= 0
loop i < 4:
    current = make_result(i)
    if current is:
        Success(value, label) => summary = "ok"
        Failure(reason, code) => summary = "fail"
        Pending => summary = "pending"
    ;
    i = i + 1
;

-- Nested match via helper to avoid shadowing
outer = TaskResult::Success(2, "nested")
nested_result ~= ""
if outer is:
    Success(value, label) =>
        inner = make_result(value)
        nested_result = describe_task(inner)
    Failure(reason, code) =>
        nested_result = "outer_fail"
    Pending =>
        nested_result = "outer_pending"
;

-- Use imported formatter
formatted = format_task(default_success)

-- Result interop: convert choice to result
read_task |n Int| -> TaskResult, Error!:
    if n is 0:
        return! Error("zero")
    ;
    return TaskResult::Success(n, "ok")
;

interop_result = read_task(5) catch:

    then TaskResult::Failure("err", 0)

;
-- Extract label from choice into a function to avoid borrow checker issues with captures
extract_label |t TaskResult| -> String:
    if t is:
        Success(value, label) => return label
        Failure(reason, code) => return "fail"
        Pending => return "pending"
    ;
;

interop_label = extract_label(interop_result)

[:adversarial_choice_payload_control_flow summary=[summary] nested=[nested_result] formatted=[formatted] interop=[interop_label]]
