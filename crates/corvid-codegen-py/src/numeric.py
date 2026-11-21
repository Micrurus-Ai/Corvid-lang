import math


def __corvid_checked_i64(value):
    if not -(1 << 63) <= value < (1 << 63):
        raise OverflowError("Corvid Int overflow")
    return value


def __corvid_div(a, b):
    quotient = abs(a) // abs(b)
    return -quotient if (a < 0) != (b < 0) else quotient


def __corvid_rem(a, b):
    return a - __corvid_div(a, b) * b


def __corvid_fdiv(a, b):
    if b == 0.0:
        if a == 0.0 or math.isnan(a):
            return math.nan
        return math.copysign(math.inf, math.copysign(1.0, a) * math.copysign(1.0, b))
    return a / b


def __corvid_frem(a, b):
    if b == 0.0 or math.isinf(a):
        return math.nan
    return math.fmod(a, b)
