//! `corvid act <action> --field value ...` (B10-4): call a declared
//! action from the command line.
//!
//! The input is read by the same rules as every other caller: flags are
//! text values read like form fields (one per record field when the
//! action takes one record, otherwise one per parameter; a `List`
//! repeats its flag, an `Option` may be left out), or `--json` gives the
//! input as JSON. The result prints as public JSON, the same form an HTTP
//! caller receives. The command line has no signed-in person yet, so an
//! action that requires one is refused by name rather than run as no
//! one.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use corvid_driver::{compile_to_ir_with_config_at_path, load_corvid_config_for, render_all_pretty, run_ir_with_runtime, Value};
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;

use crate::action_calls::{catalog, decode_json_input, decode_text_input, TextInputRefusal};

pub(crate) fn cmd_act(file: &Path, action: &str, json: Option<&str>, mode: Option<&str>, flags: &[String]) -> Result<u8> {
    let connector_mode = crate::run_cmd::parse_connector_mode(mode)?;
    let source = std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let config = load_corvid_config_for(file);
    let ir = match compile_to_ir_with_config_at_path(&source, file, config.as_ref()) {
        Ok(ir) => ir,
        Err(diags) => {
            eprint!("{}", render_all_pretty(&diags, file, &source));
            return Ok(1);
        }
    };
    let actions = catalog(&ir)?;
    let Some(endpoint) = actions.iter().find(|a| a.name == action) else {
        let declared: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        eprintln!(
            "error: `{}` declares no action `{action}`; its actions are: {}",
            file.display(),
            if declared.is_empty() { "none".to_string() } else { declared.join(", ") }
        );
        return Ok(1);
    };
    if endpoint.policy.requires_auth() {
        eprintln!(
            "error: action `{action}` requires a signed-in caller, and `corvid act` runs as no one: there is no sign-in from the command line yet (on-behalf-of authority, B11). Call it through `corvid serve` with a session."
        );
        return Ok(1);
    }

    let types_by_id: HashMap<DefId, &IrType> = ir.types.iter().map(|t| (t.id, t)).collect();
    let inputs = match json {
        Some(text) => {
            if !flags.is_empty() {
                eprintln!("error: give the input as `--json` or as flags, not both");
                return Ok(1);
            }
            let value: serde_json::Value = serde_json::from_str(text).context("`--json` is not JSON")?;
            match decode_json_input(endpoint, &value, &types_by_id) {
                Ok(inputs) => inputs,
                Err(refused) => {
                    eprintln!("error: {}", refused["detail"].as_str().unwrap_or("the input was refused"));
                    return Ok(1);
                }
            }
        }
        None => {
            let given = match flag_values(flags) {
                Ok(given) => given,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Ok(1);
                }
            };
            if let Some(unknown) = given.keys().find(|name| !takes_flag(endpoint, name, &types_by_id)) {
                eprintln!("error: action `{action}` takes no `--{unknown}`");
                return Ok(1);
            }
            match decode_text_input(endpoint, &given, &types_by_id, "flag") {
                Ok(inputs) => inputs,
                Err(TextInputRefusal::Unsupported(detail) | TextInputRefusal::Refused(detail)) => {
                    eprintln!("error: {detail}");
                    return Ok(1);
                }
            }
        }
    };

    let Some(runtime) = corvid_driver::interpreter_runtime(file, &ir, connector_mode)? else {
        return Ok(1);
    };
    runtime.emit_host_event("action", serde_json::json!({ "action": endpoint.name, "surface": "cli" }));
    let agent = ir.agents.iter().find(|a| a.name == endpoint.agent).expect("the catalog names a lowered agent");
    let tokio_rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let printed = tokio_rt.block_on(async {
        let value = run_ir_with_runtime(&ir, Some(&endpoint.agent), inputs, &runtime)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        print_result(value, &agent.return_ty, &types_by_id).await
    });
    match printed {
        Ok(()) => Ok(0),
        Err(err) => {
            eprintln!("error: {err}");
            Ok(1)
        }
    }
}

/// `--name value` and `--name=value` pairs, every value kept under its
/// name in order, so a `List` parameter repeats its flag.
fn flag_values(flags: &[String]) -> Result<HashMap<String, Vec<String>>, String> {
    let mut given: HashMap<String, Vec<String>> = HashMap::new();
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        let Some(name) = flag.strip_prefix("--") else {
            return Err(format!("expected `--name value`, got `{flag}`"));
        };
        let (name, value) = match name.split_once('=') {
            Some((name, value)) => (name.to_string(), value.to_string()),
            None => {
                let value = rest.next().ok_or_else(|| format!("`--{name}` needs a value"))?;
                (name.to_string(), value.clone())
            }
        };
        given.entry(name).or_default().push(value);
    }
    Ok(given)
}

/// Whether `--name` names part of the action's input: a field of the
/// record it takes, or one of its parameters.
fn takes_flag(endpoint: &crate::action_calls::ActionEndpoint, name: &str, types_by_id: &HashMap<DefId, &IrType>) -> bool {
    match &endpoint.whole_input {
        Some(Type::Struct(id)) => types_by_id.get(id).is_some_and(|t| t.fields.iter().any(|f| f.name == name)),
        Some(Type::ImportedStruct(record)) => {
            types_by_id.get(&record.def_id).is_some_and(|t| t.fields.iter().any(|f| f.name == name))
        }
        _ => endpoint.params.iter().any(|(param, _)| param == name),
    }
}

/// The result as public JSON; a stream prints one JSON value per line as
/// each item arrives.
async fn print_result(value: Value, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Result<(), anyhow::Error> {
    let encode = |value: &Value, ty: &Type| {
        corvid_vm::encode_public_json(value, ty, types_by_id).map_err(|reason| anyhow::anyhow!("the result has no JSON form: {reason}"))
    };
    match (value, ty) {
        (Value::Stream(stream), Type::Stream(item)) => {
            while let Some(next) = stream.next().await {
                let chunk = next.map_err(|err| anyhow::anyhow!("{err:?}"))?;
                println!("{}", encode(&chunk, item)?);
            }
            Ok(())
        }
        (value, ty) => {
            println!("{}", serde_json::to_string_pretty(&encode(&value, ty)?)?);
            Ok(())
        }
    }
}
