Problem = |
    detail String,
|

to_error |this Problem| -> Error:
    return Error(message = this.detail, code = 0)
;

Problem must CASTABLE_TO_ERROR

problem = Problem(detail = "something went wrong")
err Error = cast problem

[:user same-file error: [err.message]]
