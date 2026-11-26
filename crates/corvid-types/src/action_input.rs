//! The shape of an action's input, as every caller outside Corvid code
//! sends it: a form, a JSON client, an assistant, the CLI.
//!
//! A parameter of type `Caller` is the caller, filled from the verified
//! session and never part of the input. Of the other parameters: one of
//! a declared type is the whole input, so `action move_card(move:
//! MoveCard, by: Caller)` reads a `MoveCard` and a form posts `card`, `to`
//! and `position`. Anything else reads an object with one field per
//! parameter: `action rename(card: Int, title: String)` reads `{"card":
//! 3, "title": "..."}`.
//!
//! The contract, the OpenAPI document and `corvid serve` all decide the
//! shape here, so a client is never told one shape and sent through
//! another.

use crate::types::Type;

/// Where actions are served: `POST /actions/<name>`.
pub const ACTIONS_PATH: &str = "/actions/";

/// A parameter the server fills with the signed-in caller.
pub fn is_caller(ty: &Type) -> bool {
    *ty == Type::actor()
}

/// The declared type that is the whole input, or `None` when the input
/// is an object with one field per parameter. Caller parameters are not
/// part of the input.
pub fn whole_input_type<'a>(param_types: impl IntoIterator<Item = &'a Type>) -> Option<&'a Type> {
    let mut inputs = param_types.into_iter().filter(|ty| !is_caller(ty));
    match (inputs.next(), inputs.next()) {
        (Some(only), None) if matches!(only, Type::Struct(_) | Type::ImportedStruct(_)) => Some(only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_resolve::DefId;

    #[test]
    fn one_declared_type_is_the_whole_input() {
        let record = Type::Struct(DefId(4));
        assert_eq!(whole_input_type([&record]), Some(&record));
        let caller = Type::actor();
        assert_eq!(whole_input_type([&record, &caller]), Some(&record), "the caller is not input");
    }

    #[test]
    fn anything_else_is_one_field_per_parameter() {
        let record = Type::Struct(DefId(4));
        assert_eq!(whole_input_type([&Type::Int]), None);
        assert_eq!(whole_input_type([&record, &Type::Int]), None);
        assert_eq!(whole_input_type(std::iter::empty()), None);
        assert_eq!(whole_input_type([&Type::actor()]), None);
    }
}
