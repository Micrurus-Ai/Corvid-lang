use super::*;

pub(crate) fn cmd_connectors(command: ConnectorsCommand) -> Result<u8> {
    match command {
        ConnectorsCommand::List { json } => {
            let entries = connectors_cmd::run_list()?;
            if json {
                let out = serde_json::to_string_pretty(
                    &entries
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "name": e.name,
                                "provider": e.provider,
                                "modes": e.modes,
                                "scope_count": e.scope_count,
                                "write_scopes": e.write_scopes,
                                "rate_limit": e.rate_limit_summary,
                                "redaction_count": e.redaction_count,
                            })
                        })
                        .collect::<Vec<_>>(),
                )?;
                println!("{out}");
            } else {
                println!(
                    "{:<10} {:<14} {:<22} {:<6} {:<28} {}",
                    "NAME", "PROVIDER", "MODES", "SCOPES", "RATE LIMIT", "WRITE SCOPES"
                );
                for e in &entries {
                    println!(
                        "{:<10} {:<14} {:<22} {:<6} {:<28} {}",
                        e.name,
                        e.provider,
                        e.modes.join(","),
                        e.scope_count,
                        if e.rate_limit_summary.len() > 27 {
                            // U+2026 HORIZONTAL ELLIPSIS. This literal was
                            // mojibake — the same round-trip through
                            // Windows-1252 documented for the checkmarks
                            // below, which the earlier fix missed here.
                            format!("{}\u{2026}", &e.rate_limit_summary[..26])
                        } else {
                            e.rate_limit_summary.clone()
                        },
                        e.write_scopes.join(","),
                    );
                }
            }
            Ok(0)
        }
        ConnectorsCommand::Check {
            live,
            baseline,
            observed,
            narrate,
            json,
        } => {
            // Drift-detection mode: both --baseline and --observed
            // supplied. Runs the schema-agnostic detector and
            // exits non-zero on any drift. Hermetic (no network).
            if let (Some(baseline_path), Some(observed_path)) =
                (baseline.as_ref(), observed.as_ref())
            {
                let (report, narrations) = if narrate {
                    connectors_cmd::run_contract_drift_with_narration(
                        baseline_path,
                        observed_path,
                    )?
                } else {
                    let report = connectors_cmd::run_contract_drift(
                        baseline_path,
                        observed_path,
                    )?;
                    (report, Vec::new())
                };
                if json {
                    let mut payload = serde_json::json!({
                        "added_paths": report.added_paths,
                        "removed_paths": report.removed_paths,
                        "type_changed_paths": report
                            .type_changed_paths
                            .iter()
                            .map(|c| serde_json::json!({
                                "path": c.path,
                                "baseline_type": c.baseline_type,
                                "observed_type": c.observed_type,
                            }))
                            .collect::<Vec<_>>(),
                        "total": report.total(),
                    });
                    if narrate {
                        payload["narration"] = serde_json::Value::Array(
                            narrations
                                .iter()
                                .map(|n| serde_json::json!({
                                    "path": n.path,
                                    "kind": n.kind,
                                    "consequence": n.consequence,
                                    "severity": n.severity,
                                    "sources": n.sources,
                                }))
                                .collect(),
                        );
                    }
                    let out = serde_json::to_string_pretty(&payload)?;
                    println!("{out}");
                } else if report.is_empty() {
                    println!("contract drift: none (0 sites)");
                } else if narrate {
                    println!("contract drift: {} site(s)", report.total());
                    for n in &narrations {
                        println!(
                            "  [{:<10}] {:<13} {}",
                            n.severity, n.kind, n.path
                        );
                        println!("    {}", n.consequence);
                        for source in &n.sources {
                            println!("    source: {source}");
                        }
                    }
                } else {
                    println!("contract drift: {} site(s)", report.total());
                    for path in &report.removed_paths {
                        println!("  removed:        {path}");
                    }
                    for path in &report.added_paths {
                        println!("  added:          {path}");
                    }
                    for change in &report.type_changed_paths {
                        println!(
                            "  type-changed:   {}  ({} -> {})",
                            change.path, change.baseline_type, change.observed_type
                        );
                    }
                }
                return Ok(if report.is_empty() { 0 } else { 1 });
            }
            // Default mode: manifest schema validation. `--live`
            // without --baseline/--observed remains the
            // operational gate per `35V2-P41-E-LR-live-provider-ci-matrix`.
            let entries = connectors_cmd::run_check(live)?;
            let any_invalid = entries.iter().any(|e| !e.valid);
            if json {
                let out = serde_json::to_string_pretty(
                    &entries
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "name": e.name,
                                "valid": e.valid,
                                "diagnostics": e.diagnostics,
                            })
                        })
                        .collect::<Vec<_>>(),
                )?;
                println!("{out}");
            } else {
                println!("{:<12} {:<7} DIAGNOSTICS", "NAME", "VALID");
                for e in &entries {
                    // U+2713 CHECK MARK / U+2717 BALLOT X. The
                    // previous literals were mojibake: the UTF-8
                    // bytes of these glyphs interpreted as
                    // Windows-1252 (`â` `œ` `“` / `—`) and
                    // re-saved as UTF-8 by an editor that
                    // assumed the wrong source encoding. Rust
                    // string literals are UTF-8, so a re-encoded
                    // mojibake literal stays mojibake all the
                    // way to stdout — the `check_passes_default_manifests`
                    // test asserts on the real checkmark and was
                    // failing because the binary was emitting
                    // the round-tripped garbage instead.
                    let status = if e.valid { "\u{2713}" } else { "\u{2717}" };
                    println!("{:<12} {:<7} {}", e.name, status, e.diagnostics.join("; "));
                }
            }
            Ok(if any_invalid { 1 } else { 0 })
        }
        ConnectorsCommand::Run {
            connector,
            operation,
            scope,
            mode,
            payload,
            mock,
            approval_id,
            replay_key,
            tenant_id,
            actor_id,
            token_id,
            now_ms,
        } => {
            let payload_value = match payload {
                Some(path) => {
                    let raw = std::fs::read_to_string(&path)
                        .with_context(|| format!("reading payload from `{}`", path.display()))?;
                    Some(serde_json::from_str(&raw).with_context(|| "payload is not JSON")?)
                }
                None => None,
            };
            let mock_value = match mock {
                Some(path) => {
                    let raw = std::fs::read_to_string(&path)
                        .with_context(|| format!("reading mock from `{}`", path.display()))?;
                    Some(serde_json::from_str(&raw).with_context(|| "mock is not JSON")?)
                }
                None => None,
            };
            let resolved_now_ms = now_ms.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            });
            let output = connectors_cmd::run_run(connectors_cmd::ConnectorRunArgs {
                connector,
                operation,
                scope_id: scope,
                mode,
                payload: payload_value,
                mock_payload: mock_value,
                approval_id,
                replay_key,
                tenant_id,
                actor_id,
                token_id,
                now_ms: resolved_now_ms,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "connector": output.connector,
                    "operation": output.operation,
                    "mode": output.mode,
                    "payload": output.payload,
                }))?
            );
            Ok(0)
        }
        ConnectorsCommand::Simulate {
            file,
            operation,
            deny,
            json,
        } => {
            let simulations = connectors_cmd::run_simulate(&file, operation.as_deref())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&connectors_cmd::to_json(&simulations))?
                );
            } else {
                print!("{}", connectors_cmd::render(&simulations));
            }
            let code = connectors_cmd::exit_code(&simulations, &deny);
            if code != 0 && !json {
                eprintln!("denied by --deny:");
                for (operation, detail) in connectors_cmd::denied_findings(&simulations, &deny) {
                    eprintln!("  {operation}: {detail}");
                }
            }
            Ok(code)
        }
        ConnectorsCommand::Oauth { command } => match command {
            ConnectorsOauthCommand::Init {
                provider,
                client_id,
                redirect_uri,
                scope,
            } => {
                let output = connectors_cmd::run_oauth_init(connectors_cmd::OauthInitArgs {
                    provider,
                    client_id,
                    redirect_uri,
                    scopes: scope,
                })?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "provider": output.provider,
                        "state": output.state,
                        "code_verifier": output.code_verifier,
                        "code_challenge": output.code_challenge,
                        "authorization_url": output.authorization_url,
                    }))?
                );
                Ok(0)
            }
            ConnectorsOauthCommand::Rotate {
                provider,
                token_id,
                access_token,
                refresh_token,
                client_id,
                client_secret,
            } => {
                let output = connectors_cmd::run_oauth_rotate(connectors_cmd::OauthRotateArgs {
                    provider,
                    token_id,
                    access_token,
                    refresh_token,
                    client_id,
                    client_secret,
                })?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "provider": output.provider,
                        "access_token": output.access_token,
                        "refresh_token": output.refresh_token,
                        "expires_at_ms": output.expires_at_ms,
                    }))?
                );
                Ok(0)
            }
        },
        ConnectorsCommand::VerifyWebhook {
            signature,
            secret_env,
            body_file,
            provider,
            headers,
        } => {
            let parsed_headers = headers
                .iter()
                .map(|h| {
                    let mut parts = h.splitn(2, '=');
                    let name = parts.next().unwrap_or_default().to_string();
                    let value = parts.next().unwrap_or_default().to_string();
                    (name, value)
                })
                .collect::<Vec<_>>();
            let output = connectors_cmd::run_verify_webhook(connectors_cmd::WebhookVerifyArgs {
                signature,
                secret_env,
                body_file,
                provider,
                headers: parsed_headers,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "valid": output.valid,
                    "algorithm": output.algorithm,
                    "outcome": output.outcome,
                }))?
            );
            Ok(if output.valid { 0 } else { 1 })
        }
    }
}
