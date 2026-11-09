-- Struct placeholder visible before resolution.
-- Tests that a function defined before a struct can still return the struct
-- because dependency sorting produces a complete declaration placeholder list.

make_user || -> User:
    return User(name = "Nye")
;

User = |
    name String
|

user = make_user()
[:[user.name]]
