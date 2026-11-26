//! Type system and effect checker.
//!
//! Walks a parsed, resolved `File` and validates type and effect rules.
//! The headline check is **approve-before-dangerous**: any call to a tool
//! declared `dangerous` must be preceded by a matching `approve` in the
//! same block, or compilation fails.
//!
//! See `ARCHITECTURE.md` §5–§6.

mod approval_reachability;
pub mod builtin_methods;
pub mod checker;
pub mod config;
pub mod determinism;
pub mod effects;
pub mod errors;
pub mod law_check;
pub mod repl;
pub mod table_ops;
pub mod table_columns;
pub mod text_parameters;
pub mod type_receiver;
pub mod types;

pub use checker::{
    typecheck, typecheck_with_config, typecheck_with_config_and_modules, typecheck_with_modules,
    Checked, ImportedCallKind, ImportedCallTarget,
};
pub use config::{
    CorvidConfig, CustomDimensionConfig, CustomDimensionMeta, DimensionConfigError,
    DimensionValueType, EffectSystemConfig, PackagePolicyConfig, BUILTIN_DIMENSION_NAMES,
};
pub use determinism::{
    classify_call_target, NondeterminismSource, NondeterministicBuiltin,
    KNOWN_NONDETERMINISTIC_BUILTINS,
};
pub use effects::{
    analyze_effects, check_grounded_returns, compose_dimension_public, compute_worst_case_cost, compute_worst_case_cost_with_types,
    render_cost_tree, AgentEffectSummary, ComposedProfile, ConstraintViolation, CostEstimate,
    CostNodeKind, CostTreeNode, CostWarning, CostWarningKind, EffectProfile, EffectRegistry,
    ProvenanceViolation,
};
pub use errors::{TypeError, TypeErrorKind, TypeWarning, TypeWarningKind};
pub use law_check::{
    check_dimension, laws_for_rule, DimensionUnderTest, Law, LawCheckResult, Verdict,
    DEFAULT_SAMPLES,
};
pub use repl::{CheckedTurn, ReplLocal, ReplSession, ReplTurnBuild, REPL_RESULT_NAME};
pub use builtin_methods::{
    builtin_method, builtin_type_method, builtin_type_receiver, decimal_rounding_mode, duration_reading_unit, is_hmac_key_type, json_error_fields, number_literal_text, refine_builtin_method_kind, regex_match_fields, type_method,
    BuiltinMethodKind, BuiltinMethodSig, JSON_ERROR_TYPE_ID, REGEX_MATCH_TYPE_ID, RESPONSE_FIELDS,
    RESPONSE_RECORD_IDS, RESPONSE_TYPE_ID,
};
pub use table_ops::{table_call_target, table_decl, table_method_target, TableOp};
pub use type_receiver::{type_receiver, TypeReceiver};
pub use builtin_methods::{CONFLICT_TYPE_ID, UNRESOLVED_DEF_ID};
pub use types::{ConflictCase, ConflictCases, Type};

#[cfg(test)]
mod tests;
