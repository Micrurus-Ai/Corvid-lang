//! Executes the provenance benchmark with deterministic, offline dependencies.
use anyhow::{anyhow, bail, Result};
use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime, Runtime, Value};
use corvid_types::Type;
use serde_json::{json, Value as Json};
use std::{collections::BTreeSet, path::Path, sync::Arc};

const TEXT: &str = "alpha beta gamma delta epsilon";

fn fixture(ty: &Type) -> Result<Json> {
    match ty {
        Type::Grounded(inner) => fixture(inner),
        Type::List(inner) => Ok(json!([fixture(inner)?])),
        Type::String => Ok(json!(TEXT)),
        Type::Int => Ok(json!(1)),
        Type::Float => Ok(json!(1.5)),
        Type::Struct(_) => Ok(json!({"name": TEXT, "price": 1.5, "description": TEXT})),
        _ => bail!("missing deterministic fixture for {ty:?}"),
    }
}

fn retrievals(chain: &corvid_runtime::ProvenanceChain, found: &mut BTreeSet<String>) {
    use corvid_runtime::ProvenanceKind;
    for entry in &chain.entries {
        match &entry.kind {
            ProvenanceKind::Retrieval => {
                found.insert(entry.name.clone());
            }
            ProvenanceKind::Derived { inputs, .. } => {
                for input in inputs {
                    retrievals(input, found);
                }
            }
            _ => {}
        }
    }
}

fn sources(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Grounded(g) => {
            retrievals(&g.provenance, found);
            sources(&g.inner.get(), found);
        }
        Value::List(items) => {
            for item in items.iter_cloned() {
                sources(&item, found);
            }
        }
        _ => {}
    }
}

async fn measure(path: &Path) -> Result<(bool, String)> {
    let source = std::fs::read_to_string(path)?;
    let ir = compile_to_ir_with_config_at_path(&source, path, None).map_err(|ds| {
        anyhow!(
            "compile: {}",
            ds.iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let agent = ir
        .agents
        .last()
        .ok_or_else(|| anyhow!("missing chain agent"))?;
    let args = agent
        .params
        .iter()
        .map(|p| {
            corvid_vm::json_to_value(fixture(&p.ty)?, &p.ty, &Default::default())
                .map_err(|e| anyhow!("{e:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut runtime = Runtime::builder();
    let mut expected = BTreeSet::new();
    for tool in &ir.tools {
        // One deterministic source document per retrieval endpoint. Its
        // source identity is the retrieval name Corvid stores in the chain.
        if tool.name.starts_with("retrieve") {
            expected.insert(tool.name.clone());
        }
        let value = fixture(&tool.return_ty)?;
        runtime = runtime.tool(tool.name.clone(), move |_| {
            let value = value.clone();
            async move { Ok(value) }
        });
    }
    if expected.is_empty() {
        bail!("fixture declares no retrieval source");
    }
    let mut model = corvid_runtime::MockAdapter::new("fixture");
    for prompt in &ir.prompts {
        model = model.reply(&prompt.name, fixture(&prompt.return_ty)?);
    }
    let runtime = runtime
        .llm(Arc::new(model))
        .default_model("fixture")
        .build();
    let value = run_ir_with_runtime(&ir, Some(&agent.name), args, &runtime)
        .await
        .map_err(|e| anyhow!("{}", e.user_facing_detail()))?;
    let mut found = BTreeSet::new();
    sources(&value, &mut found);
    Ok((
        expected.is_subset(&found),
        format!("expected {expected:?}, returned {found:?}"),
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow!("expected a Corvid source file"))?;
    let (verdict, note) = match measure(Path::new(&path)).await {
        Ok((true, note)) => ("preserved", note),
        Ok((false, note)) => ("lost", note),
        Err(e) => ("error", e.to_string()),
    };
    println!("{}", json!({"verdict": verdict, "note": note}));
    Ok(())
}
