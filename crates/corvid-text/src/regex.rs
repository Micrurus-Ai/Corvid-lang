//! Regular expressions: the checks the compiler runs on pattern and
//! replacement literals, and the matching both tiers run.
//!
//! The syntax is the `regex` crate's: Perl-style classes, groups and
//! repetition, Unicode-aware by default, without backreferences or
//! lookaround, so matching time is linear in the length of the text.
//! [`check_pattern`] is the compiler's gate; the matching functions
//! receive only patterns that passed it, so their `Err` side reports an
//! internal error, never bad user input.
//!
//! Positions count Unicode scalar values, as `String.length()` and
//! `substring` do, so `text.substring(m.start, m.end) == m.text`.

use ::regex::{NoExpand, Regex};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// What the compiler learns from a valid pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternInfo {
    /// The name of each group, in group-number order. Group 0, the
    /// whole match, is always unnamed.
    pub group_names: Vec<Option<String>>,
    /// Whether the pattern can match the empty string, which
    /// `split_regex` refuses.
    pub can_match_empty: bool,
}

impl PatternInfo {
    /// The number of groups, counting group 0.
    pub fn group_count(&self) -> usize {
        self.group_names.len()
    }
}

/// Check a pattern the way the runtime will compile it. The error is
/// one line naming the problem and its character position.
pub fn check_pattern(pattern: &str) -> Result<PatternInfo, String> {
    let hir = regex_syntax::ParserBuilder::new()
        .build()
        .parse(pattern)
        .map_err(|error| syntax_error_message(pattern, &error))?;
    let compiled = Regex::new(pattern).map_err(|error| match error {
        ::regex::Error::CompiledTooBig(limit) => {
            format!("the pattern compiles to more than {limit} bytes; split it into simpler patterns")
        }
        other => other.to_string(),
    })?;
    Ok(PatternInfo {
        group_names: compiled.capture_names().map(|name| name.map(str::to_string)).collect(),
        can_match_empty: hir.properties().minimum_len() == Some(0),
    })
}

fn syntax_error_message(pattern: &str, error: &regex_syntax::Error) -> String {
    let (kind, offset) = match error {
        regex_syntax::Error::Parse(error) => (error.kind().to_string(), error.span().start.offset),
        regex_syntax::Error::Translate(error) => (error.kind().to_string(), error.span().start.offset),
        other => (other.to_string(), 0),
    };
    format!("{kind} at position {}", char_position(pattern, offset))
}

/// Why a replacement template was refused: the problem, and what to
/// write instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementError {
    pub message: String,
    pub hint: String,
}

/// Check a literal replacement template against the pattern's groups.
/// `$1`, `${1}`, `$name` and `${name}` insert a group and `$$` inserts
/// `$`. The references are read exactly as the `regex` crate reads
/// them, so a template that passes expands as written; anything the
/// crate would silently expand to nothing is refused here instead.
pub fn check_replacement(template: &str, info: &PatternInfo) -> Result<(), ReplacementError> {
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '$' {
            index += 1;
            continue;
        }
        let dollar = index;
        match chars.get(index + 1) {
            Some('$') => index += 2,
            Some('{') => {
                let Some(close) = chars[index + 2..].iter().position(|c| *c == '}') else {
                    return Err(ReplacementError {
                        message: format!("the `${{` at position {dollar} is never closed"),
                        hint: "close the group reference with `}`, or write `$$` for a dollar sign".into(),
                    });
                };
                let name: String = chars[index + 2..index + 2 + close].iter().collect();
                check_reference(&name, true, dollar, info)?;
                index += close + 3;
            }
            Some(c) if is_reference_char(*c) => {
                let length = chars[index + 1..].iter().take_while(|c| is_reference_char(**c)).count();
                let name: String = chars[index + 1..index + 1 + length].iter().collect();
                check_reference(&name, false, dollar, info)?;
                index += length + 1;
            }
            _ => {
                return Err(ReplacementError {
                    message: format!("the `$` at position {dollar} does not start a group reference"),
                    hint: "write `$$` for a dollar sign, or `$1` or `${name}` for a group".into(),
                })
            }
        }
    }
    Ok(())
}

/// The characters the `regex` crate reads after an unbraced `$`.
fn is_reference_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn check_reference(name: &str, braced: bool, position: usize, info: &PatternInfo) -> Result<(), ReplacementError> {
    let groups = describe_groups(info);
    if let Ok(number) = name.parse::<usize>() {
        if number < info.group_count() {
            return Ok(());
        }
        return Err(ReplacementError {
            message: format!("the replacement refers to group {number} at position {position}, but {groups}"),
            hint: "use a group the pattern defines, or write `$$` for a dollar sign".into(),
        });
    }
    if !name.is_empty() && info.group_names.iter().any(|group| group.as_deref() == Some(name)) {
        return Ok(());
    }
    let digits = name.chars().take_while(char::is_ascii_digit).count();
    let hint = if !braced && digits > 0 {
        let (number, rest) = name.split_at(digits);
        format!("`${name}` is read as one group name; write `${{{number}}}{rest}` to follow group {number} with `{rest}`")
    } else {
        "use a group the pattern defines, or write `$$` for a dollar sign".into()
    };
    Err(ReplacementError {
        message: format!("the replacement refers to a group named `{name}` at position {position}, but {groups}"),
        hint,
    })
}

fn describe_groups(info: &PatternInfo) -> String {
    let named: Vec<String> = info.group_names.iter().flatten().map(|name| format!("`{name}`")).collect();
    let numbered = match info.group_count() {
        1 => "the pattern has only group 0, the whole match".to_string(),
        count => format!("the pattern has groups 0 to {}", count - 1),
    };
    if named.is_empty() {
        numbered
    } else {
        format!("{numbered}, named {}", named.join(", "))
    }
}

/// One match: its text, its character span and every group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexMatch {
    pub text: String,
    /// Character position of the first matched character.
    pub start: i64,
    /// Character position just past the match.
    pub end: i64,
    /// Group `i` at index `i`; index 0 is the whole match. A group
    /// that took no part in the match is `None`.
    pub groups: Vec<Option<String>>,
}

/// `text.contains_regex(pattern)`.
pub fn contains(text: &str, pattern: &str) -> Result<bool, String> {
    Ok(compiled(pattern)?.is_match(text))
}

/// `text.find_regex(pattern)`: the first match.
pub fn find(text: &str, pattern: &str) -> Result<Option<RegexMatch>, String> {
    let regex = compiled(pattern)?;
    let mut cursor = CharCursor::new(text);
    Ok(regex.captures(text).map(|captures| to_match(&captures, &mut cursor)))
}

/// `text.find_all_regex(pattern)`: every non-overlapping match, left
/// to right.
pub fn find_all(text: &str, pattern: &str) -> Result<Vec<RegexMatch>, String> {
    let regex = compiled(pattern)?;
    let mut cursor = CharCursor::new(text);
    Ok(regex.captures_iter(text).map(|captures| to_match(&captures, &mut cursor)).collect())
}

/// `text.replace_regex(pattern, template)` with a literal template the
/// compiler checked: group references expand.
pub fn replace(text: &str, pattern: &str, template: &str) -> Result<String, String> {
    Ok(compiled(pattern)?.replace_all(text, template).into_owned())
}

/// `text.replace_regex(pattern, replacement)` with a replacement
/// computed at run time: inserted exactly as written.
pub fn replace_verbatim(text: &str, pattern: &str, replacement: &str) -> Result<String, String> {
    Ok(compiled(pattern)?.replace_all(text, NoExpand(replacement)).into_owned())
}

/// `text.split_regex(pattern)`: the text between matches. The compiler
/// refuses patterns that can match the empty string.
pub fn split(text: &str, pattern: &str) -> Result<Vec<String>, String> {
    Ok(compiled(pattern)?.split(text).map(str::to_string).collect())
}

fn to_match(captures: &::regex::Captures<'_>, cursor: &mut CharCursor<'_>) -> RegexMatch {
    let whole = captures.get(0).expect("group 0 takes part in every match");
    let start = cursor.position_of(whole.start());
    let end = cursor.position_of(whole.end());
    RegexMatch {
        text: whole.as_str().to_string(),
        start,
        end,
        groups: captures.iter().map(|group| group.map(|g| g.as_str().to_string())).collect(),
    }
}

/// Converts increasing byte offsets to character positions without
/// rescanning the text from the start for each match.
struct CharCursor<'a> {
    text: &'a str,
    byte: usize,
    chars: i64,
}

impl<'a> CharCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, byte: 0, chars: 0 }
    }

    fn position_of(&mut self, byte: usize) -> i64 {
        if byte < self.byte {
            self.byte = 0;
            self.chars = 0;
        }
        self.chars += self.text[self.byte..byte].chars().count() as i64;
        self.byte = byte;
        self.chars
    }
}

/// The compiled form of a checked pattern, cached per process. Patterns
/// are source literals, so the cache is bounded by the program text.
fn compiled(pattern: &str) -> Result<Regex, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(regex) = cache.lock().unwrap_or_else(|poison| poison.into_inner()).get(pattern) {
        return Ok(regex.clone());
    }
    let regex = Regex::new(pattern)
        .map_err(|error| format!("internal error: the pattern `{pattern}` reached the runtime unchecked: {error}"))?;
    cache
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(pattern.to_string(), regex.clone());
    Ok(regex)
}

fn char_position(text: &str, offset: usize) -> usize {
    text.char_indices().take_while(|(index, _)| *index < offset).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pattern: &str) -> PatternInfo {
        check_pattern(pattern).expect("valid pattern")
    }

    #[test]
    fn invalid_patterns_name_the_problem_and_its_position() {
        assert_eq!(check_pattern("(ab").unwrap_err(), "unclosed group at position 0");
        assert_eq!(check_pattern("é[a-").unwrap_err(), "unclosed character class at position 1");
        assert_eq!(check_pattern("a{2,1}").unwrap_err(), "invalid repetition count range, the start must be <= the end at position 1");
        assert!(check_pattern("(?<n>a)(?<n>b)").unwrap_err().starts_with("duplicate capture group name"));
        assert!(check_pattern(r"(a)\1").unwrap_err().starts_with("backreferences are not supported"));
        assert!(check_pattern("(?=a)").unwrap_err().starts_with("look-around"));
    }

    #[test]
    fn valid_patterns_report_their_groups() {
        let date = info(r"(?<year>\d{4})-(\d{2})-(?P<day>\d{2})");
        assert_eq!(
            date.group_names,
            vec![None, Some("year".to_string()), None, Some("day".to_string())]
        );
        assert_eq!(date.group_count(), 4);
        assert!(!date.can_match_empty);
        assert!(info("a*").can_match_empty);
        assert!(info("x|").can_match_empty);
        assert!(info(r"\b").can_match_empty);
        assert!(!info(r"\s+").can_match_empty);
    }

    #[test]
    fn replacement_references_must_name_existing_groups() {
        let date = info(r"(?<year>\d{4})-(\d{2})");
        for good in ["$1", "${2}", "$year", "${year}", "$$5", "[$0]", "no references", "${year}x"] {
            assert_eq!(check_replacement(good, &date), Ok(()), "{good}");
        }
        let missing = check_replacement("$3", &date).unwrap_err();
        assert_eq!(
            missing.message,
            "the replacement refers to group 3 at position 0, but the pattern has groups 0 to 2, named `year`"
        );
        let named = check_replacement("x ${month}", &date).unwrap_err();
        assert!(named.message.starts_with("the replacement refers to a group named `month` at position 2"));
    }

    #[test]
    fn a_number_followed_by_letters_is_one_name_and_the_hint_says_so() {
        let error = check_replacement("$1a", &info("(x)")).unwrap_err();
        assert_eq!(error.hint, "`$1a` is read as one group name; write `${1}a` to follow group 1 with `a`");
    }

    #[test]
    fn a_stray_or_unclosed_dollar_is_refused() {
        let plain = info("x");
        assert!(check_replacement("costs $", &plain).unwrap_err().message.contains("does not start a group reference"));
        assert!(check_replacement("$ 5", &plain).is_err());
        assert!(check_replacement("${1", &plain).unwrap_err().message.contains("is never closed"));
        assert!(check_replacement("${}", &plain).is_err());
    }

    #[test]
    fn matches_report_character_positions_and_groups() {
        let found = find("née 2024-07 et 2025-01", r"(\d{4})-(\d{2})(x)?").unwrap().unwrap();
        assert_eq!(found.text, "2024-07");
        assert_eq!((found.start, found.end), (4, 11));
        assert_eq!(
            found.groups,
            vec![Some("2024-07".into()), Some("2024".into()), Some("07".into()), None]
        );
        let all = find_all("née 2024-07 et 2025-01", r"\d{4}").unwrap();
        let spans: Vec<(i64, i64)> = all.iter().map(|m| (m.start, m.end)).collect();
        assert_eq!(spans, vec![(4, 8), (15, 19)]);
        assert_eq!(find("abc", r"\d").unwrap(), None);
    }

    #[test]
    fn unicode_classes_and_dot_work_on_characters() {
        assert!(contains("naïve", r"^\w+$").unwrap());
        let found = find("☕☕x", "^..").unwrap().unwrap();
        assert_eq!((found.text.as_str(), found.start, found.end), ("☕☕", 0, 2));
        assert!(contains("STRASSE", "(?i)strasse").unwrap());
    }

    #[test]
    fn replace_expands_a_checked_template_and_inserts_run_time_text_verbatim() {
        let pattern = r"(?<user>\w+)@(\w+)";
        assert_eq!(replace("ada@example", pattern, "${2}:$user $$").unwrap(), "example:ada $");
        assert_eq!(replace_verbatim("ada@example", pattern, "$2 ${user}").unwrap(), "$2 ${user}");
        assert_eq!(replace("a1b22c", r"\d+", "#").unwrap(), "a#b#c");
        assert_eq!(replace("abc", "x*", "-").unwrap(), "-a-b-c-");
    }

    #[test]
    fn split_keeps_empty_pieces_like_string_split() {
        assert_eq!(split("a, b,,c", r",\s*").unwrap(), vec!["a", "b", "", "c"]);
        assert_eq!(split("a,", ",").unwrap(), vec!["a", ""]);
        assert_eq!(split("", ",").unwrap(), vec![""]);
    }

    #[test]
    fn the_runtime_refuses_an_unchecked_pattern_without_panicking() {
        assert!(contains("x", "(").unwrap_err().starts_with("internal error"));
    }
}
