import builtins as __corvid_builtins


def __corvid_wrap_i64(value):
    value &= (1 << 64) - 1
    return value - (1 << 64) if value >= (1 << 63) else value


def __corvid_checked_i64(value):
    if not -(1 << 63) <= value < (1 << 63):
        raise __corvid_builtins.OverflowError("Corvid Int overflow")
    return value


def __corvid_div_i64(left, right):
    magnitude = __corvid_builtins.abs(left) // __corvid_builtins.abs(right)
    return __corvid_checked_i64(-magnitude if (left < 0) != (right < 0) else magnitude)


def __corvid_rem_i64(left, right):
    magnitude = __corvid_builtins.abs(left) % __corvid_builtins.abs(right)
    return -magnitude if left < 0 else magnitude
