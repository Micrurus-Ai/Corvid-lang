use super::assert_parity_bool;
use super::scalar_harness::assert_parity_text;

#[test]
fn float_addition_eq_check() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return 1.5 + 2.5 == 4.0\n",
        true,
    );
}

#[test]
fn float_subtraction_and_multiplication() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return (5.0 - 1.5) * 2.0 == 7.0\n",
        true,
    );
}

#[test]
fn float_division_exact() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return 9.0 / 3.0 == 3.0\n",
        true,
    );
}

#[test]
fn mixed_int_float_promotes_to_float() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return 3 + 0.5 == 3.5\n",
        true,
    );
    assert_parity_bool(
        "agent f() -> Bool:\n    return 0.5 + 3 == 3.5\n",
        true,
    );
}

#[test]
fn float_ordering() {
    assert_parity_bool("agent f() -> Bool:\n    return 1.5 < 2.0\n", true);
    assert_parity_bool("agent f() -> Bool:\n    return 2.0 <= 2.0\n", true);
    assert_parity_bool("agent f() -> Bool:\n    return 3.14 > 3.0\n", true);
    assert_parity_bool("agent f() -> Bool:\n    return 3.0 >= 3.0\n", true);
}

#[test]
fn float_unary_negation() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return -2.5 == 0.0 - 2.5\n",
        true,
    );
}

#[test]
fn float_div_by_zero_is_infinity_not_trap() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return 1.0 / 0.0 > 1.0\n",
        true,
    );
}

#[test]
fn nan_inequality_is_true() {
    assert_parity_bool(
        "agent f() -> Bool:\n    return (0.0 / 0.0) != (0.0 / 0.0)\n",
        true,
    );
    assert_parity_bool(
        "agent f() -> Bool:\n    return (0.0 / 0.0) == (0.0 / 0.0)\n",
        false,
    );
}

#[test]
fn float_in_local_binding() {
    assert_parity_bool(
        "\
agent f() -> Bool:
    pi = 3.14
    tau = pi * 2.0
    return tau > 6.0
",
        true,
    );
}

/// A Float result prints as `corvid run` prints it on the interpreter,
/// always with its decimal point. Native code used `%.17g`, which printed
/// `0.1` as `0.10000000000000001`; both tiers once printed `2.0` as `2`.
#[test]
fn float_results_print_alike() {
    for (src, expected) in [
        ("agent f() -> Float:\n    return 0.1\n", "0.1"),
        ("agent f() -> Float:\n    return 2.0\n", "2.0"),
        ("agent f() -> Float:\n    return 0.1 + 0.2\n", "0.30000000000000004"),
        ("agent f() -> Float:\n    return 1.0 / 3.0\n", "0.3333333333333333"),
    ] {
        assert_eq!(assert_parity_text(src), expected, "{src}");
    }
}

/// `to_string()` on Int, Float and Bool compiles natively and writes the
/// interpreter's text: a Float always shows its decimal point.
#[test]
fn scalars_to_string_alike() {
    let text = assert_parity_text(
        "agent f() -> String:\n    return 0.1.to_string() + \" \" + 2.0.to_string() + \" \" + (0 - 7).to_string() + \" \" + true.to_string() + \" \" + (1 < 0).to_string() + \" \" + (1.0 / 0.0).to_string()\n",
    );
    assert_eq!(text, "0.1 2.0 -7 true false inf");
}
