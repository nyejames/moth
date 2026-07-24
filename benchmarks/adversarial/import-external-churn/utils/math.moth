import @core/math { PI, sin, cos, sqrt }
import @data/constants { base_count, scale_factor }

scaled_count || -> Int:
    return base_count * scale_factor
;

wave |phase Float| -> Float:
    return sin(phase) + cos(phase * 2.0)
;

hypotenuse |a Float, b Float| -> Float:
    return sqrt((a * a) + (b * b))
;

pi_value || -> Float:
    return PI
;
