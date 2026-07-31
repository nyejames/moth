-- Validate descending inference and explicit step semantics.

count_desc ~= 0
loop 5 to 0 |i|:
    count_desc = count_desc + 1
;

sum_even_desc ~= 0
loop 10 to & 0 by 2 |i|:
    sum_even_desc = sum_even_desc + i
;

sum_even_desc_negative ~= 0
loop 10 to & 0 by -2 |i|:
    sum_even_desc_negative = sum_even_desc_negative + i
;

omitted_start_desc_count ~= 0
loop to 0 |i|:
    omitted_start_desc_count = omitted_start_desc_count + 1
;

omitted_start_inclusive_desc_sum ~= 0
loop to & 0 by 2 |i|:
    omitted_start_inclusive_desc_sum = omitted_start_inclusive_desc_sum + i
;

[:loop_range_direction_and_step count_desc=[count_desc] sum_even_desc=[sum_even_desc] sum_even_desc_negative=[sum_even_desc_negative] omitted_desc=[omitted_start_desc_count] omitted_inclusive_desc=[omitted_start_inclusive_desc_sum]]
