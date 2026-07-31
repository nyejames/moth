import @choices { Traffic }
import @choices { identity }

initial Traffic = Traffic::Red
next Traffic = identity(initial)

result ~= "unset"
if next is:
    Red => result = "red"
    Yellow => result = "yellow"
    Green => result = "green"
;
[:choice_cross_file_carrier_shape result=[result]]
