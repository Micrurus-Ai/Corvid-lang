//! The shape of an action's input, as every caller outside Corvid code
//! sends it: a form, a JSON client, an assistant, the CLI.
//!
//! An action that takes one parameter of a declared type takes that
//! value as its whole input: `action move_card(move: MoveCard)` reads a
//! `MoveCard`, so a form posts `card`, `to` and `position`. Any other
//! action reads an object with one field per parameter: `action
//! rename(card: Int, title: String)` reads `{"card": 3, "title": "..."}`.
//!
//! The contract, the OpenAPI document and `corvid serve` all decide the
//! shape here, so a client is never told one shape and sent through
//! another.

use crate::types::Type;

/// Where actions are served: `POST /actions/<name>`.
pub const ACTIONS_PATH: &str = "/actions/";

/// The declared type that is the whole input, or `None` when the input
/// is an object with one field per parameter.
pub fn whole_input_type<'a>(mut param_types: impl ExactSizeIterator<Item = &'a Type>) -> Option<&'a Type> {
    if param_types.len() != 1 {
        return None;
    }
    param_types
        .next()
        .filter(|ty| matches!(ty, Type::Struct(_) | Type::ImportedStruct(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_resolve::DefId;

    #[test]
    fn one_declared_type_is_the_whole_input() {
        let record = Type::Struct(DefId(4));
        assert_eq!(whole_input_type([&record].into_iter()), Some(&record));
    }

    #[test]
    fn anything_else_is_one_field_per_parameter() {
        let record = Type::Struct(DefId(4));
        assert_eq!(whole_input_type([&Type::Int].into_iter()), None);
        assert_eq!(whole_input_type([&record, &Type::Int].into_iter()), None);
        assert_eq!(whole_input_type(std::iter::empty()), None);
    }
}
