//! Connector startup closure (slice 52g-3c) — the connector arm of the
//! Phase 52 "prove it or refuse to start" invariant.
//!
//! A `connector` declares the execution modes it is ALLOWED to run in
//! (`modes: [mock, replay, real]`); the deployment selects exactly one
//! at start. Before a program that declares connectors runs, the
//! backend proves that the selected mode is real and executable for
//! every operation — or it refuses to start, naming the decision. This
//! is the connector analogue of [`crate::contract_closure`] for HTTP
//! routes.
//!
//! Enforced here (all are startup refusals, never first-call errors):
//!
//! 1. A program with connectors and NO selected mode refuses to start —
//!    whether an operation reaches a real provider is a consequential
//!    choice the deployment must make explicitly (no hidden default).
//! 2. The selected mode must be in the connector's declared `modes`.
//! 3. `real` requires resolvable `secret(...)` references AND explicit
//!    outbound-network permission (the `[http] allow` egress list).
//! 4. Every operation must have an executable path in the selected mode:
//!    `mock` needs a declared mock payload, `replay` needs a recorded
//!    source present, `real` needs the credentials + egress from (3).
//!
//! The selected mode is immutable after startup: it is set once when the
//! runtime is built and never changes for the process.

use corvid_ast::ConnectorMode;
use corvid_ir::{IrConnector, IrConnectorAuth, IrFile};

/// A reason the backend refuses to start because a connector cannot
/// execute in the selected mode. Named precisely enough that the
/// developer knows which connector/operation to look at and what is
/// missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorStartupError {
    /// The program declares connectors but the deployment selected no
    /// mode. Whether an operation reaches a real provider is a
    /// consequential choice — silence is a refusal, not a default.
    NoModeSelected { connectors: Vec<String> },
    /// The selected mode is not one the connector declared it may run in.
    ModeNotAllowed {
        connector: String,
        selected: ConnectorMode,
        allowed: Vec<ConnectorMode>,
    },
    /// `real` mode, but a `secret(...)` the connector's auth references
    /// does not resolve. Only the secret NAME appears here — never a
    /// value.
    RealMissingSecret { connector: String, secret: String },
    /// `real` mode, but no explicit outbound-network permission is
    /// configured (`[http] allow` / `CORVID_HTTP_ALLOW`). A connector
    /// never reaches the network by default.
    RealMissingEgress { connector: String },
    /// `replay` mode, but no recorded interaction source is present —
    /// replay must serve from a cassette and never fall through to real.
    ReplayWithoutSource { connector: String },
    /// The selected mode is declared and otherwise valid, but the
    /// runtime does not execute it yet (it arrives in a later slice).
    /// Refusing at startup keeps the "prove it or refuse" invariant:
    /// selecting a mode the runtime can't serve is never a first-call
    /// error.
    ModeNotExecutableYet {
        connector: String,
        mode: ConnectorMode,
    },
    /// An operation has no executable path in the selected mode (e.g.
    /// `mock` mode but the operation declares no mock payload).
    OperationNotServeable {
        connector: String,
        operation: String,
        mode: ConnectorMode,
        reason: String,
    },
}

impl ConnectorStartupError {
    /// The single-line `E5205` startup-refusal message.
    pub fn message(&self) -> String {
        match self {
            Self::NoModeSelected { connectors } => format!(
                "E5205 Connector mode not selected: this program declares connector(s) [{}], \
                 but the deployment selected no execution mode. Select one of the connector's \
                 declared `modes` at start (e.g. `--mode mock`). Whether an operation reaches a \
                 real provider is a consequential choice — the backend refuses to start rather \
                 than pick one silently.",
                connectors.join(", ")
            ),
            Self::ModeNotAllowed {
                connector,
                selected,
                allowed,
            } => format!(
                "E5205 Connector mode not allowed: connector `{connector}` was started in \
                 `{}` mode, but it only declares `modes: [{}]`. Select a declared mode.",
                selected.as_str(),
                allowed
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::RealMissingSecret { connector, secret } => format!(
                "E5205 Connector credential unresolved: connector `{connector}` runs in `real` \
                 mode but the secret `{secret}` it authenticates with does not resolve. Provide \
                 the secret before starting in real mode.",
            ),
            Self::RealMissingEgress { connector } => format!(
                "E5205 Connector outbound network not permitted: connector `{connector}` runs in \
                 `real` mode but no outbound-network permission is configured. Add its host to \
                 `[http] allow` (or `CORVID_HTTP_ALLOW`) — a connector never reaches the network \
                 by default.",
            ),
            Self::ReplayWithoutSource { connector } => format!(
                "E5205 Connector replay source missing: connector `{connector}` runs in `replay` \
                 mode but no recorded interaction source is present. Replay serves from a \
                 recorded cassette and never falls through to a real call.",
            ),
            Self::ModeNotExecutableYet { connector, mode } => format!(
                "E5205 Connector mode not executable yet: connector `{connector}` was started in \
                 `{}` mode, which this runtime does not execute yet. Select an executable mode.",
                mode.as_str()
            ),
            Self::OperationNotServeable {
                connector,
                operation,
                mode,
                reason,
            } => format!(
                "E5205 Connector operation not executable: `{connector}.{operation}` has no \
                 executable path in `{}` mode ({reason}). The backend refuses to start rather \
                 than advertise an operation it cannot serve.",
                mode.as_str()
            ),
        }
    }
}

/// The runtime facts the startup check consults, supplied by the
/// deployment wiring: the selected mode, whether outbound network is
/// permitted, whether a recorded-interaction source is present, and a
/// resolver that answers whether a named secret resolves. The secret
/// resolver takes only the NAME and returns a bool — a secret value
/// never enters this check.
pub struct ConnectorRuntimeContext<'a> {
    pub selected_mode: Option<ConnectorMode>,
    pub egress_configured: bool,
    pub replay_source_present: bool,
    pub secret_present: &'a dyn Fn(&str) -> bool,
    /// The connector execution modes this runtime tier actually
    /// executes as of the current slice (mirrors
    /// [`crate::contract_closure::RuntimeCapabilities`]). Selecting a
    /// declared-but-not-yet-implemented mode is a startup refusal, not a
    /// first-call error. Grows as each mode's dispatch lands.
    pub executable_modes: &'a [ConnectorMode],
}

/// The secret reference NAMES a connector's auth depends on. Never the
/// values — those stay in the secret store and never enter startup
/// diagnostics.
pub fn connector_secret_names(connector: &IrConnector) -> Vec<String> {
    match &connector.auth {
        None => Vec::new(),
        Some(IrConnectorAuth::Bearer { secret }) => vec![secret.clone()],
        Some(IrConnectorAuth::Header { secret, .. }) => vec![secret.clone()],
        Some(IrConnectorAuth::Basic {
            username_secret,
            password_secret,
        }) => vec![username_secret.clone(), password_secret.clone()],
    }
}

/// Walk the connector surface and return every reason the backend must
/// refuse to start under the selected mode. An empty vec means every
/// connector operation has a real, executable path in the selected mode
/// and the backend may start.
pub fn check_connector_startup(
    ir: &IrFile,
    ctx: &ConnectorRuntimeContext,
) -> Vec<ConnectorStartupError> {
    let mut errors = Vec::new();
    if ir.connectors.is_empty() {
        return errors;
    }

    // (1) A program with connectors must have a selected mode.
    let Some(mode) = ctx.selected_mode else {
        errors.push(ConnectorStartupError::NoModeSelected {
            connectors: ir.connectors.iter().map(|c| c.name.clone()).collect(),
        });
        return errors;
    };

    for connector in &ir.connectors {
        // (2) The selected mode must be one the connector allows.
        if !connector.modes.contains(&mode) {
            errors.push(ConnectorStartupError::ModeNotAllowed {
                connector: connector.name.clone(),
                selected: mode,
                allowed: connector.modes.clone(),
            });
            // The remaining checks are mode-specific and would be
            // misleading for a mode this connector never allows.
            continue;
        }

        // The runtime must actually execute the selected mode. A
        // declared-but-not-yet-implemented mode refuses at startup, never
        // at first call.
        if !ctx.executable_modes.contains(&mode) {
            errors.push(ConnectorStartupError::ModeNotExecutableYet {
                connector: connector.name.clone(),
                mode,
            });
            continue;
        }

        // Slice 52h-2: verified provider protocols now HAVE an executable
        // path — the durable intent lifecycle (intent persisted before
        // submit, provider job id bound only from a typed response, every
        // transition checkpointed, resumed after a restart). Contract
        // Closure therefore no longer refuses a protocol-bearing
        // operation. The remaining requirement — that a protocol runs
        // inside a durable job so its intent can survive a restart — is
        // a property of the CALL SITE, not the startup surface, so the
        // runtime enforces it when the operation is invoked.

        // (3) + (4) Every operation must have an executable path in the
        // selected mode.
        match mode {
            ConnectorMode::Mock => {
                for op in &connector.operations {
                    if op.mock.is_none() {
                        errors.push(ConnectorStartupError::OperationNotServeable {
                            connector: connector.name.clone(),
                            operation: op.name.clone(),
                            mode,
                            reason: "no mock payload declared".to_string(),
                        });
                    }
                }
            }
            ConnectorMode::Real => {
                if !ctx.egress_configured {
                    errors.push(ConnectorStartupError::RealMissingEgress {
                        connector: connector.name.clone(),
                    });
                }
                for secret in connector_secret_names(connector) {
                    if !(ctx.secret_present)(&secret) {
                        errors.push(ConnectorStartupError::RealMissingSecret {
                            connector: connector.name.clone(),
                            secret,
                        });
                    }
                }
            }
            ConnectorMode::Replay => {
                if !ctx.replay_source_present {
                    errors.push(ConnectorStartupError::ReplayWithoutSource {
                        connector: connector.name.clone(),
                    });
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MODES: [ConnectorMode; 3] =
        [ConnectorMode::Mock, ConnectorMode::Replay, ConnectorMode::Real];

    fn ctx<'a>(
        mode: Option<ConnectorMode>,
        egress: bool,
        replay: bool,
        secret: &'a dyn Fn(&str) -> bool,
    ) -> ConnectorRuntimeContext<'a> {
        // Default: every mode is executable, so these tests exercise the
        // other gates. The not-executable-yet gate has its own test with
        // a restricted set.
        ConnectorRuntimeContext {
            selected_mode: mode,
            egress_configured: egress,
            replay_source_present: replay,
            secret_present: secret,
            executable_modes: &ALL_MODES,
        }
    }

    const GITHUB: &str = r#"
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_TOKEN"))
    modes: [mock, replay, real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{owner}/{repo}"
        mock: Repo(repo)
"#;

    fn ir(source: &str) -> IrFile {
        crate::compile_to_ir(source).expect("source compiles")
    }

    #[test]
    fn no_selected_mode_refuses_to_start() {
        let ir = ir(GITHUB);
        let always = |_: &str| true;
        let errs = check_connector_startup(&ir, &ctx(None, true, true, &always));
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            errs[0],
            ConnectorStartupError::NoModeSelected { .. }
        ));
        assert!(errs[0].message().contains("E5205"));
    }

    #[test]
    fn a_program_without_connectors_never_refuses() {
        let ir = ir("agent main() -> Int:\n    return 1\n");
        let always = |_: &str| true;
        // No mode selected, but no connectors → nothing to refuse.
        let errs = check_connector_startup(&ir, &ctx(None, false, false, &always));
        assert!(errs.is_empty());
    }

    #[test]
    fn disallowed_mode_refuses_to_start() {
        // A connector allowing only [mock] started in real mode.
        let src = GITHUB.replace("modes: [mock, replay, real]", "modes: [mock]");
        let ir = ir(&src);
        let always = |_: &str| true;
        let errs = check_connector_startup(&ir, &ctx(Some(ConnectorMode::Real), true, true, &always));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConnectorStartupError::ModeNotAllowed { .. })));
    }

    #[test]
    fn mock_mode_is_executable_when_every_operation_has_a_mock() {
        let ir = ir(GITHUB);
        let never = |_: &str| false;
        // Mock mode needs neither secrets nor egress.
        let errs = check_connector_startup(&ir, &ctx(Some(ConnectorMode::Mock), false, false, &never));
        assert!(errs.is_empty(), "mock mode should be executable: {errs:?}");
    }

    #[test]
    fn real_mode_refuses_without_egress_or_credentials() {
        let ir = ir(GITHUB);
        let missing = |_: &str| false;
        let errs =
            check_connector_startup(&ir, &ctx(Some(ConnectorMode::Real), false, false, &missing));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConnectorStartupError::RealMissingEgress { .. })));
        assert!(errs.iter().any(|e| matches!(
            e,
            ConnectorStartupError::RealMissingSecret { secret, .. } if secret == "GITHUB_TOKEN"
        )));
    }

    #[test]
    fn real_mode_is_executable_with_egress_and_resolvable_secret() {
        let ir = ir(GITHUB);
        let present = |name: &str| name == "GITHUB_TOKEN";
        let errs =
            check_connector_startup(&ir, &ctx(Some(ConnectorMode::Real), true, false, &present));
        assert!(errs.is_empty(), "real mode should be executable: {errs:?}");
    }

    #[test]
    fn a_declared_mode_the_runtime_cannot_execute_yet_refuses_at_startup() {
        // `real` is a declared+allowed mode with credentials and egress,
        // but if the runtime tier does not execute it yet, selecting it
        // is a startup refusal — never a first-call error.
        let ir = ir(GITHUB);
        let present = |_: &str| true;
        let mock_only = [ConnectorMode::Mock];
        let restricted = ConnectorRuntimeContext {
            selected_mode: Some(ConnectorMode::Real),
            egress_configured: true,
            replay_source_present: true,
            secret_present: &present,
            executable_modes: &mock_only,
        };
        let errs = check_connector_startup(&ir, &restricted);
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConnectorStartupError::ModeNotExecutableYet { .. })));
    }

    #[test]
    fn replay_mode_refuses_without_a_recorded_source() {
        let ir = ir(GITHUB);
        let always = |_: &str| true;
        let errs =
            check_connector_startup(&ir, &ctx(Some(ConnectorMode::Replay), false, false, &always));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConnectorStartupError::ReplayWithoutSource { .. })));

        // With a recorded source present, replay is executable.
        let ok =
            check_connector_startup(&ir, &ctx(Some(ConnectorMode::Replay), false, true, &always));
        assert!(ok.is_empty(), "replay with a source should be executable: {ok:?}");
    }

    #[test]
    fn a_verified_protocol_is_startup_clean_now_that_the_lifecycle_runtime_exists() {
        let source = r#"
connector video:
    base_url: "https://video.example.com"
    modes: [real]
    operation generate(input: String) -> String dangerous:
        POST "/generations" body input
        async:
            statuses: [queued, completed, failed]
            initial: queued
            terminal: [completed, failed]
            deadline: 600s
            deadline_target: failed
            idempotency: intent via header "Idempotency-Key"
            poll GET "/generations"
            every: 2s
            on_protocol_change: refuse
            state queued:
                on queued -> queued
                on completed -> completed
                on failed -> failed
"#;
        let ir = ir(source);
        let present = |_: &str| true;
        let errors = check_connector_startup(&ir, &ctx(Some(ConnectorMode::Real), true, false, &present));
        // Slice 52h-2 landed the durable intent lifecycle, so a
        // protocol-bearing operation now HAS an executable path and
        // Contract Closure no longer refuses it. (Until 52h-2 this
        // asserted the opposite — the refusal was the honest state while
        // the runtime could not execute a temporal contract.) The
        // remaining durability requirement — running inside a durable job
        // — is a call-site property the runtime enforces at invocation,
        // not something startup can decide.
        assert!(
            !errors.iter().any(|error| matches!(
                error,
                ConnectorStartupError::OperationNotServeable { reason, .. }
                    if reason.contains("protocol")
            )),
            "a protocol-bearing operation must be startup-clean now that the durable \
             lifecycle executes it; got: {errors:?}"
        );
    }
}
