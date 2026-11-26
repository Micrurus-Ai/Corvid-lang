//! How a number is written as text, in the one place both tiers read.
//!
//! The interpreter and native code each write a `Float` in three
//! places, and each place has one form:
//!
//! - **`to_string()`** always shows a decimal point (`2.0`, `0.1`), the
//!   language's rule that a `Float` reads as one.
//! - **A printed result** (`corvid run` printing what the entry agent
//!   returned) is Rust's shortest form (`2`, `0.1`).
//! - **JSON** (a value interpolated into a prompt, an argument or result
//!   in a trace) is the JSON number text (`2.0`, `0.1`), with `null` for
//!   a value JSON cannot hold.
//!
//! Native code used C's `%.17g`, which writes `0.1` as
//! `0.10000000000000001`, so a native program printed, prompted and
//! recorded different text from the interpreter. Both now use these.

/// `float.to_string()`.
pub fn float_to_string(value: f64) -> String {
    let rendered = format!("{value}");
    if rendered.contains('.')
        || rendered.contains('e')
        || rendered.contains('E')
        || rendered.contains("inf")
        || rendered.contains("NaN")
    {
        rendered
    } else {
        format!("{rendered}.0")
    }
}

/// A `Float` result as `corvid run` prints it.
pub fn float_display(value: f64) -> String {
    format!("{value}")
}

/// A `Float` as JSON number text, `null` when it is not finite.
pub fn float_json(value: f64) -> String {
    serde_json::Value::from(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_place_writes_its_own_form() {
        assert_eq!(float_to_string(2.0), "2.0");
        assert_eq!(float_to_string(0.1), "0.1");
        assert_eq!(float_display(2.0), "2");
        assert_eq!(float_display(0.1), "0.1");
        assert_eq!(float_json(2.0), "2.0");
        assert_eq!(float_json(0.1), "0.1");
        assert_eq!(float_json(f64::NAN), "null");
        assert_eq!(float_to_string(0.1 + 0.2), "0.30000000000000004");
    }
}
