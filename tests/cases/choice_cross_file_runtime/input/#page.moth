import @choices { Traffic }
import @choices { identity }

initial Traffic = Traffic::Red
next Traffic = identity(initial)

label ~= "unset"
if next is:
    Red => label = "red"
    Yellow => label = "yellow"
    Green => label = "green"
;
[:choice_cross_file_runtime initial=[label] next=[label]]
