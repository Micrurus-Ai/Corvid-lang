//! The Corvid Application Contract (Phase 51 slice 51a).
//!
//! A machine-readable description of a Corvid application's PUBLIC
//! surface — routes, public agents and prompts, the types they
//! exchange, and (uniquely) the AI-native capabilities of each:
//! whether it streams, whether its output is `Grounded`, whether its
//! input carries untrusted `Tainted` content, whether it can raise
//! approvals, its confidence floor, and its worst-case cost/latency.
//!
//! This is the sibling of [`crate::emit::emit_abi`]: the ABI
//! descriptor describes the C/cdylib export surface; the application
//! contract describes the HTTP + agent surface a FRONTEND consumes.
//! Both are emitted after resolve + type/effect checking and share
//! the same [`crate::type_description`] type shapes.
//!
//! Slice 51a ships the structural spine. The OpenAPI 3.1 projection
//! (51b), the richer AI-native event/approval metadata (51c), `@ui`
//! hints (51d), and typed-error presentation (51e) layer on top of
//! this model.

use corvid_ast::{Decl, DimensionValue, EffectDecl, File, Refinement, TypeRef};
use corvid_resolve::Resolved;
use corvid_types::{compute_worst_case_cost, effects::EffectRegistry, Checked};
use serde::{Deserialize, Serialize};

/// The full application contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationContract {
    /// Contract format version (independent of the compiler version).
    pub contract_version: u32,
    pub compiler_version: String,
    pub generated_at: String,
    pub source_path: String,
    /// Public types the surface exchanges, with their field
    /// refinements projected as value constraints.
    pub types: Vec<ContractType>,
    /// Declared HTTP routes.
    pub routes: Vec<ContractRoute>,
    /// Public agents (agent invocations, not ordinary REST).
    pub agents: Vec<ContractCallable>,
    /// Public prompts exposed to callers.
    pub prompts: Vec<ContractCallable>,
}

/// The current contract format version. Bumped only on a breaking
/// change to the emitted shape — the same compatibility discipline
/// the ABI descriptor carries.
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractType {
    pub name: String,
    /// Record fields (empty for a sum type).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ContractField>,
    /// Sum-type variant names (empty for a record).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractField {
    pub name: String,
    /// JSON-Schema-flavored type name for the field.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Value constraints from a `where` refinement (slice 50j),
    /// projected into the JSON-Schema vocabulary so frontends get
    /// client-side validation for free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRoute {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_params: Vec<ContractParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_type: Option<String>,
    pub response_type: String,
    /// Permissions the route requires (from its effect row's trust /
    /// dangerous surface). Empty = no approval-requiring effect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractParam {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

/// A public agent or prompt, with its AI-native capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractCallable {
    pub name: String,
    pub inputs: Vec<ContractParam>,
    pub output_type: String,
    pub capabilities: Capabilities,
}

/// What a frontend must know to invoke a callable correctly — the
/// information ordinary API specs cannot express.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Output is a `Stream<T>` — the client consumes SSE events, not
    /// a single response.
    pub streaming: bool,
    /// Output is `Grounded<T>` — carries citation sources.
    pub grounded: bool,
    /// An input is `Tainted<T>` — the callee reads untrusted content
    /// (slice 50i); the frontend should treat inputs accordingly.
    pub tainted_input: bool,
    /// The callee (transitively) reaches an approval-requiring call,
    /// so an `approval_required` event is possible mid-invocation.
    pub approvals_possible: bool,
    /// Confidence floor, when the callee is confidence-gated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_min: Option<f64>,
    /// Worst-case USD cost the checker proved for the callee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    /// Latency class from the composed effect row (`fast`/`medium`/
    /// `slow`), when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<String>,
}

/// Options threaded from the caller (compiler version, timestamp).
pub struct ContractOptions<'a> {
    pub source_path: &'a str,
    pub compiler_version: &'a str,
    pub generated_at: &'a str,
}

/// Emit the application contract from the checked program.
pub fn emit_application_contract(
    file: &File,
    resolved: &Resolved,
    checked: &Checked,
    registry: &EffectRegistry,
    opts: &ContractOptions<'_>,
) -> ApplicationContract {
    let types = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Type(t) if t.visibility.is_callable_from_outside_file() => {
                Some(contract_type(t))
            }
            _ => None,
        })
        .collect();

    let routes = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Server(server) => Some(server),
            _ => None,
        })
        .flat_map(|server| server.routes.iter().map(contract_route))
        .collect();

    // `compute_worst_case_cost` is available for a tighter
    // per-body bound; the contract advertises the callable's
    // COMPOSED EFFECT-ROW surface (what a frontend should budget for),
    // which composes downstream effects through the call graph.
    let _ = (resolved, checked, compute_worst_case_cost);

    let agents = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Agent(a) if a.visibility.is_callable_from_outside_file() => Some(
                contract_callable(
                    &a.name.name,
                    &a.params,
                    &a.return_ty,
                    &a.effect_row,
                    registry,
                ),
            ),
            _ => None,
        })
        .collect();

    let prompts = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Prompt(p) if p.visibility.is_callable_from_outside_file() => Some(
                contract_callable(
                    &p.name.name,
                    &p.params,
                    &p.return_ty,
                    &p.effect_row,
                    registry,
                ),
            ),
            _ => None,
        })
        .collect();

    ApplicationContract {
        contract_version: CONTRACT_VERSION,
        compiler_version: opts.compiler_version.to_string(),
        generated_at: opts.generated_at.to_string(),
        source_path: opts.source_path.to_string(),
        types,
        routes,
        agents,
        prompts,
    }
}

fn contract_type(t: &corvid_ast::TypeDecl) -> ContractType {
    ContractType {
        name: t.name.name.clone(),
        fields: t.fields.iter().map(contract_field).collect(),
        variants: t.variants.iter().map(|v| v.name.name.clone()).collect(),
    }
}

fn contract_field(f: &corvid_ast::Field) -> ContractField {
    let mut field = ContractField {
        name: f.name.name.clone(),
        type_name: type_ref_name(&f.ty),
        minimum: None,
        maximum: None,
        min_length: None,
        max_length: None,
    };
    match f.refinement {
        Some(Refinement::Between { min, max }) => {
            field.minimum = Some(min);
            field.maximum = Some(max);
        }
        Some(Refinement::LenBetween { min, max }) => {
            field.min_length = Some(min);
            field.max_length = Some(max);
        }
        None => {}
    }
    field
}

fn contract_route(r: &corvid_ast::HttpRouteDecl) -> ContractRoute {
    ContractRoute {
        method: r.method.as_str().to_string(),
        path: r.path.clone(),
        path_params: r
            .path_params
            .iter()
            .map(|p| ContractParam {
                name: p.name.name.clone(),
                type_name: type_ref_name(&p.ty),
            })
            .collect(),
        query_type: r.query_ty.as_ref().map(type_ref_name),
        body_type: r.body_ty.as_ref().map(type_ref_name),
        response_type: type_ref_name(&r.response.ty),
        // A route inherits the permission surface of its effect row;
        // 51h wires named permissions. For 51a we surface the effect
        // names that require approval.
        requires: r
            .effect_row
            .effects
            .iter()
            .map(|e| e.name.name.clone())
            .collect(),
    }
}

/// Collect the file's effect declarations to build the registry the
/// contract analysis composes against.
pub fn effect_decls_of(file: &File) -> Vec<EffectDecl> {
    file.decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Effect(e) => Some(e.clone()),
            _ => None,
        })
        .collect()
}

fn contract_callable(
    name: &str,
    params: &[corvid_ast::Param],
    return_ty: &TypeRef,
    effect_row: &corvid_ast::EffectRow,
    registry: &EffectRegistry,
) -> ContractCallable {
    let inputs = params
        .iter()
        .map(|p| ContractParam {
            name: p.name.name.clone(),
            type_name: type_ref_name(&p.ty),
        })
        .collect::<Vec<_>>();

    let tainted_input = params.iter().any(|p| type_ref_is_tainted(&p.ty));

    let composed = registry.compose(
        &effect_row
            .effects
            .iter()
            .map(|e| e.name.name.as_str())
            .collect::<Vec<_>>(),
    );
    let latency_class = match composed.dimensions.get("latency") {
        Some(DimensionValue::Name(class)) => Some(class.clone()),
        _ => None,
    };
    // Only advertise a confidence floor when there is a REAL gate;
    // 1.0 is the "fully confident / no floor declared" default and
    // would mislead a frontend into showing a strict gate.
    let confidence_min = match composed.dimensions.get("confidence") {
        Some(DimensionValue::Number(v)) if *v < 1.0 => Some(*v),
        _ => None,
    };
    let max_cost_usd = match composed.dimensions.get("cost") {
        Some(DimensionValue::Cost(v)) if *v > 0.0 => Some(*v),
        Some(DimensionValue::Number(v)) if *v > 0.0 => Some(*v),
        _ => None,
    };
    let approvals_possible = corvid_types::effects::effect_row_trust_requires_approval(
        effect_row, registry,
    )
    .is_some();

    ContractCallable {
        name: name.to_string(),
        inputs,
        output_type: type_ref_name(return_ty),
        capabilities: Capabilities {
            streaming: type_ref_head_is(return_ty, "Stream"),
            grounded: type_ref_head_is(return_ty, "Grounded"),
            tainted_input,
            approvals_possible,
            confidence_min,
            max_cost_usd,
            latency_class,
        },
    }
}

/// A frontend-facing name for a `TypeRef`. Named types keep their
/// name (referencing the `types` section); generics render their
/// head + inner recursively so `Stream<Answer>` and `Grounded<Doc>`
/// read the way a client expects.
fn type_ref_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named { name, .. } => name.name.clone(),
        TypeRef::Generic { name, args, .. } => {
            let inner = args.iter().map(type_ref_name).collect::<Vec<_>>().join(", ");
            format!("{}<{}>", name.name, inner)
        }
        TypeRef::Qualified { name, .. } => name.name.clone(),
        TypeRef::Weak { inner, .. } => format!("Weak<{}>", type_ref_name(inner)),
        // Function types don't appear on the public exchange surface.
        other => format!("{other:?}"),
    }
}

fn type_ref_head_is(ty: &TypeRef, head: &str) -> bool {
    matches!(ty, TypeRef::Generic { name, .. } if name.name == head)
}

fn type_ref_is_tainted(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Generic { name, .. } if name.name == "Tainted" => true,
        TypeRef::Generic { args, .. } => args.iter().any(type_ref_is_tainted),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_for(src: &str) -> ApplicationContract {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
        emit_application_contract(
            &file,
            &resolved,
            &checked,
            &registry,
            &ContractOptions {
                source_path: "app.cor",
                compiler_version: "test",
                generated_at: "now",
            },
        )
    }

    #[test]
    fn public_agent_capabilities_reflect_return_type_and_effects() {
        let contract = contract_for(
            "\
effect ask:
    cost: $0.05
    latency: fast

public type Answer:
    text: String
    score: Int where between(0, 100)

public agent support(question: String) -> Answer uses ask:
    return Answer(\"hi\", 90)
",
        );
        assert_eq!(contract.contract_version, CONTRACT_VERSION);
        let ty = contract.types.iter().find(|t| t.name == "Answer").unwrap();
        let score = ty.fields.iter().find(|f| f.name == "score").unwrap();
        assert_eq!(score.minimum, Some(0));
        assert_eq!(score.maximum, Some(100));

        let agent = contract.agents.iter().find(|a| a.name == "support").unwrap();
        assert!(!agent.capabilities.streaming);
        assert_eq!(agent.capabilities.latency_class.as_deref(), Some("fast"));
        assert_eq!(agent.capabilities.max_cost_usd, Some(0.05));
    }

    #[test]
    fn streaming_and_grounded_capabilities_are_detected() {
        let contract = contract_for(
            "\
effect retrieval:
    data: grounded

public agent chat(message: String) -> Stream<String>:
    return stream_answer(message)

tool stream_answer(m: String) -> Stream<String>

public agent lookup(q: String) -> Grounded<String> uses retrieval:
    return fetch(q)

tool fetch(q: String) -> Grounded<String> uses retrieval
",
        );
        let chat = contract.agents.iter().find(|a| a.name == "chat").unwrap();
        assert!(chat.capabilities.streaming);
        let lookup = contract.agents.iter().find(|a| a.name == "lookup").unwrap();
        assert!(lookup.capabilities.grounded);
    }

    #[test]
    fn confidence_floor_appears_only_when_gated() {
        let contract = contract_for(
            "effect judged:
    confidence: 0.85

public agent classify(text: String) -> String uses judged:
    return text

public agent plain(text: String) -> String:
    return text
",
        );
        let classify = contract.agents.iter().find(|a| a.name == "classify").unwrap();
        assert_eq!(classify.capabilities.confidence_min, Some(0.85));
        let plain = contract.agents.iter().find(|a| a.name == "plain").unwrap();
        assert_eq!(plain.capabilities.confidence_min, None);
    }

    #[test]
    fn private_agents_are_not_in_the_contract() {
        let contract = contract_for(
            "\
agent internal_helper(x: String) -> String:
    return x
",
        );
        assert!(contract.agents.is_empty(), "private agents must not leak");
    }
}
