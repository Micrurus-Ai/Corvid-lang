//! B03 regular expressions on the interpreter, run from Corvid source.
//! Matching rules are tested in `corvid-text`; these tests pin the
//! method surface, the `RegexMatch` record and the literal-template
//! rule for replacements.

use super::*;

async fn run_main(src: &str) -> Value {
    let ir = ir_of(src);
    run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run")
}

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

#[tokio::test]
async fn contains_regex_searches_anywhere() {
    let src = "\
agent main() -> List<Bool>:
    return [\"order 42\".contains_regex(\"[0-9]+\"), \"no digits\".contains_regex(\"[0-9]+\"), \"naïve\".contains_regex(\"^\\\\w+$\")]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(items.iter_cloned(), vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)]);
}

#[tokio::test]
async fn find_regex_returns_a_match_record_with_character_positions() {
    let src = "\
agent main() -> String:
    text = \"née 2024-07\"
    return match text.find_regex(\"(?<year>\\\\d{4})-(\\\\d{2})(x)?\"):
        Some(m) -> m.text + \"|\" + m.start.to_string() + \"|\" + m.end.to_string() + \"|\" + m.groups[1].unwrap_or(\"?\") + \"|\" + m.groups[3].unwrap_or(\"none\") + \"|\" + text.substring(m.start, m.end)
        None -> \"no match\"
";
    assert_eq!(run_main(src).await, text("2024-07|4|11|2024|none|2024-07"));
}

#[tokio::test]
async fn find_regex_returns_none_without_a_match() {
    let src = "\
agent main() -> Bool:
    return \"abc\".find_regex(\"\\\\d\").is_none()
";
    assert_eq!(run_main(src).await, Value::Bool(true));
}

#[tokio::test]
async fn find_all_regex_returns_every_match_in_order() {
    let src = "\
agent main() -> List<String>:
    words: List<String> = []
    for m in \"one, two,three\".find_all_regex(\"\\\\w+\"):
        words.append(m.text + \"@\" + m.start.to_string())
    return words
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(items.iter_cloned(), vec![text("one@0"), text("two@5"), text("three@9")]);
}

#[tokio::test]
async fn replace_regex_expands_a_literal_template() {
    let src = "\
agent main() -> String:
    return \"ada@example and bob@corvid\".replace_regex(\"(?<user>\\\\w+)@(\\\\w+)\", \"${2}:$user $$\")
";
    assert_eq!(run_main(src).await, text("example:ada $ and corvid:bob $"));
}

#[tokio::test]
async fn replace_regex_inserts_run_time_text_verbatim() {
    let src = "\
agent mask(text: String, replacement: String) -> String:
    return text.replace_regex(\"\\\\d+\", replacement)

agent main() -> String:
    return mask(\"pin 1234, code 99\", \"$1 ${x}\")
";
    assert_eq!(run_main(src).await, text("pin $1 ${x}, code $1 ${x}"));
}

#[tokio::test]
async fn split_regex_returns_the_pieces_between_matches() {
    let src = "\
agent main() -> List<String>:
    return \"a, b,,c\".split_regex(\",\\\\s*\")
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(items.iter_cloned(), vec![text("a"), text("b"), text(""), text("c")]);
}

#[tokio::test]
async fn the_reference_examples_run() {
    // docs/reference/text-and-encoding.md
    let src = "\
agent order_numbers(text: String) -> List<String>:
    numbers: List<String> = []
    for m in text.find_all_regex(\"#(\\\\d+)\"):
        numbers.append(m.groups[1].unwrap_or(\"\"))
    return numbers

agent us_to_iso(text: String) -> String:
    return text.replace_regex(\"(?<month>\\\\d{2})/(?<day>\\\\d{2})/(?<year>\\\\d{4})\", \"${year}-${month}-${day}\")

agent mask(text: String, replacement: String) -> String:
    return text.replace_regex(\"\\\\d+\", replacement)

agent main() -> Bool:
    numbers = order_numbers(\"orders #12 and #345\") == [\"12\", \"345\"]
    iso = us_to_iso(\"due 07/04/2026\") == \"due 2026-07-04\"
    masked = mask(\"pin 1234\", \"$1\") == \"pin $1\"
    split = \"a, b,,c\".split_regex(\",\\\\s*\") == [\"a\", \"b\", \"\", \"c\"]
    return numbers and iso and masked and split
";
    assert_eq!(run_main(src).await, Value::Bool(true));
}

#[tokio::test]
async fn regex_methods_are_allowed_in_deterministic_agents() {
    let src = "\
@deterministic
agent slug(title: String) -> String:
    return title.to_lower().replace_regex(\"[^a-z0-9]+\", \"-\")

agent main() -> String:
    return slug(\"Hello, Corvid World\")
";
    assert_eq!(run_main(src).await, text("hello-corvid-world"));
}
