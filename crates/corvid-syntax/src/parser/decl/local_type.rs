use super::*;

impl Parser<'_> {
    pub(super) fn parse_local_type(&mut self, visibility: Visibility) -> Result<Decl, ParseError> {
        self.bump(); // local
        if !matches!(self.peek(), TokKind::KwType) {
            return Err(self.expected_local_type("type after local", self.peek_span()));
        }
        let record = self.parse_type_decl(visibility, false)?;
        if record.fields.is_empty() || !record.variants.is_empty() || record.alias.is_some() {
            return Err(
                self.expected_local_type("a record with bounded integer fields", record.span)
            );
        }
        let name = &record.name.name;
        let ui = format!("__local_ui_{name}");
        let mut fields = Vec::new();
        for field in &record.fields {
            let Some((min, max)) = field.browser_integer_bounds() else {
                return Err(self.expected_local_type(
                    "Int with ordered, browser-safe between bounds",
                    field.span,
                ));
            };
            let key = &field.name.name;
            fields.push(format!(
                "{ui}.IntegerField(\"{key}\", value.{key}, {min}, {max})"
            ));
        }
        let source = format!(
            "import \"std:ui\" as {ui}\nfn {name}_state(value: {name}) -> {ui}.RecordState:\n    return {ui}.RecordState([{}], [])\n",
            fields.join(", ")
        );
        let mut tokens = crate::lex(&source).expect("valid local type lowering");
        for (index, token) in tokens.iter_mut().enumerate() {
            token.span = record.name.span.expanded(index);
        }
        let (file, errors) = crate::parser::parse_file(&tokens);
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        self.generated.extend(file.decls);
        Ok(Decl::Type(record))
    }

    fn expected_local_type(&self, expected: &str, span: corvid_ast::Span) -> ParseError {
        ParseError {
            span,
            kind: ParseErrorKind::UnexpectedToken {
                got: "local type declaration".into(),
                expected: expected.into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_local_fields_without_browser_integer_bounds() {
        for field in [
            "value: String",
            "value: Int",
            "value: Int where between(5, 2)",
            "value: Int where between(0, 9007199254740992)",
        ] {
            let source = format!("local type Settings:\n    {field}\n");
            let tokens = crate::lex(&source).unwrap();
            let (_, errors) = crate::parser::parse_file(&tokens);
            assert!(!errors.is_empty(), "{source}");
        }
    }
}
