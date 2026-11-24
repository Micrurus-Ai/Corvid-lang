//! Contract type names parsed into a tree.
//!
//! The Application Contract writes a field's type as text such as
//! `Map<String, Result<Int, Line>>`. Splitting that text on the first
//! comma reads it wrongly, so every SDK emitter parses it once here.

/// A type name: a head such as `List` or `Line`, and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeName {
    pub head: String,
    pub args: Vec<TypeName>,
}

impl TypeName {
    pub fn parse(text: &str) -> TypeName {
        let (name, rest) = parse_at(text.trim());
        debug_assert!(rest.trim().is_empty(), "trailing text after type `{text}`: `{rest}`");
        name
    }

    /// The first argument, for one-argument heads such as `List<T>`.
    pub fn arg(&self, index: usize) -> &TypeName {
        &self.args[index]
    }
}

/// Parse one type name from the start of `text`; return it and the rest.
fn parse_at(text: &str) -> (TypeName, &str) {
    let end = text.find(['<', ',', '>']).unwrap_or(text.len());
    let head = text[..end].trim().to_string();
    let mut rest = &text[end..];
    let mut args = Vec::new();
    if let Some(after_open) = rest.strip_prefix('<') {
        rest = after_open;
        loop {
            let (arg, after) = parse_at(rest);
            args.push(arg);
            let after = after.trim_start();
            if let Some(next) = after.strip_prefix(',') {
                rest = next;
            } else {
                rest = after.strip_prefix('>').unwrap_or(after);
                break;
            }
        }
    }
    (TypeName { head, args }, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(head: &str) -> TypeName {
        TypeName { head: head.into(), args: vec![] }
    }

    #[test]
    fn nested_arguments_with_commas_parse_as_a_tree() {
        let parsed = TypeName::parse("Map<String, Result<Int, List<Line>>>");
        assert_eq!(
            parsed,
            TypeName {
                head: "Map".into(),
                args: vec![
                    named("String"),
                    TypeName {
                        head: "Result".into(),
                        args: vec![named("Int"), TypeName { head: "List".into(), args: vec![named("Line")] }],
                    },
                ],
            }
        );
        assert_eq!(TypeName::parse("Answer"), named("Answer"));
    }
}
