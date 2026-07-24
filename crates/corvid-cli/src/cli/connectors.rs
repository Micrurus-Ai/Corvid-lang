use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ConnectorsCommand {
    /// List shipped connectors with their modes, scopes, and rate limits.
    List {
        /// Emit machine-readable JSON instead of a human table.
        #[arg(long)]
        json: bool,
    },
    /// Validate every shipped connector manifest. With
    /// `--baseline <FILE>` + `--observed <FILE>` runs the
    /// schema-agnostic contract-drift detector against two
    /// recorded JSON payloads (a CI-friendly way to catch
    /// provider-side breakage before deploy). The live-HTTP
    /// fetch path that would compute `observed` from a real
    /// provider call is gated on `CORVID_PROVIDER_LIVE=1` +
    /// per-provider credentials and is operational scope
    /// (`35V2-P41-E-LR-live-provider-ci-matrix`); use
    /// `--baseline`/`--observed` from CI scripts that already
    /// capture provider responses.
    Check {
        /// Detect manifest-vs-provider drift via real HTTP
        /// calls. Operational gate — refuses without
        /// `CORVID_PROVIDER_LIVE=1`. Prefer
        /// `--baseline`/`--observed` for hermetic CI runs.
        #[arg(long)]
        live: bool,
        /// JSON file holding the baseline response shape (a
        /// recorded mock fixture, the last successful live
        /// run, or a hand-authored expected shape).
        #[arg(long, value_name = "FILE")]
        baseline: Option<PathBuf>,
        /// JSON file holding the observed response shape (a
        /// recently-captured live response, the connector
        /// runtime's recorded output, etc.). When both
        /// `--baseline` and `--observed` are set, the command
        /// runs the structural drift detector and exits
        /// non-zero on any drift.
        #[arg(long, value_name = "FILE")]
        observed: Option<PathBuf>,
        /// Pair every drift site with a typed
        /// human-readable consequence (breaking vs.
        /// compatible) + Grounded<T> back-references to the
        /// detector evidence. Operators reviewing a CI
        /// failure read the narration first; the
        /// machine-readable JSON output is available via
        /// `--json --narrate`. Combined with `--baseline` +
        /// `--observed`.
        #[arg(long)]
        narrate: bool,
        /// Emit machine-readable JSON instead of a human report.
        #[arg(long)]
        json: bool,
    },
    /// Drive a connector operation against the chosen mode.
    Run {
        /// Connector name (gmail | slack | tasks | ms365 | calendar | files).
        #[arg(long)]
        connector: String,
        /// Operation name as defined in the connector manifest's
        /// replay rules (e.g. `search`, `read_metadata`, `draft`,
        /// `send`, `github_search`, `github_write`, `channel_read`).
        #[arg(long)]
        operation: String,
        /// Scope id from the connector's manifest (e.g.
        /// `gmail.read_metadata`).
        #[arg(long)]
        scope: String,
        /// Execution mode: `mock` (default), `replay`, `real`. Real
        /// requires `CORVID_PROVIDER_LIVE=1` and per-provider
        /// credentials.
        #[arg(long, default_value = "mock")]
        mode: String,
        /// JSON payload to forward to the operation (file path).
        #[arg(long, value_name = "FILE")]
        payload: Option<PathBuf>,
        /// JSON file with the canned mock response (mock/replay only).
        #[arg(long, value_name = "FILE")]
        mock: Option<PathBuf>,
        /// Approval id (required for write scopes).
        #[arg(long, default_value = "")]
        approval_id: String,
        /// Replay key (deterministic per logical operation).
        #[arg(long, default_value = "cli-run")]
        replay_key: String,
        /// Tenant id for the call.
        #[arg(long, default_value = "tenant-cli")]
        tenant_id: String,
        /// Actor id for the call.
        #[arg(long, default_value = "actor-cli")]
        actor_id: String,
        /// Token id (the encrypted-token reference; not the bearer).
        #[arg(long, default_value = "token-cli")]
        token_id: String,
        /// `now_ms` for rate-limit accounting (defaults to system time).
        #[arg(long)]
        now_ms: Option<u64>,
    },
    /// OAuth2 token lifecycle commands.
    Oauth {
        #[command(subcommand)]
        command: ConnectorsOauthCommand,
    },
    /// Explore what a provider could do to a declared `async:`
    /// protocol, without calling one.
    ///
    /// The checker proves a protocol is well-formed; this answers
    /// the different question of what its legal provider
    /// behaviours actually cost you. For every protocol-bearing
    /// operation in the file it reports the shortest provider
    /// behaviour reaching each terminal state, the behaviours that
    /// never terminate on their own (a slow provider holds the
    /// intent until the declared deadline fires), the worst-case
    /// observation count the budget is charged for, and terminal
    /// states no behaviour can reach.
    ///
    /// Informational by default: every behaviour it reports is
    /// LEGAL, so none of them is an error. Pass `--deny <KIND>` to
    /// make CI fail on a class your team has decided it will not
    /// accept — e.g. `--deny non_terminating` for a protocol that
    /// must not be stallable by a slow provider.
    Simulate {
        /// The `.cor` source file to explore.
        file: PathBuf,
        /// Limit exploration to one operation by name.
        #[arg(long, value_name = "NAME")]
        operation: Option<String>,
        /// Fail (exit 1) when a finding of this kind is present.
        /// Repeatable. Kinds: `non_terminating`,
        /// `immediate_terminal`, `deadline_reachable`,
        /// `unreachable_terminal`.
        #[arg(long, value_name = "KIND")]
        deny: Vec<String>,
        /// Emit machine-readable JSON instead of a human report.
        #[arg(long)]
        json: bool,
    },
    /// Verify an inbound webhook payload's HMAC-SHA256 signature
    /// against a manifest-declared secret stored in an env var.
    /// Exits 0 on a valid signature, 1 on mismatch. Pass
    /// `--provider github|slack|linear` to use the per-provider
    /// header conventions from
    /// `corvid-connector-runtime::webhook_verify` (Slack includes
    /// timestamp replay protection); without `--provider`, the
    /// generic HMAC-SHA256 verifier consumes the `--signature`
    /// value directly.
    VerifyWebhook {
        /// Provider's signature header value (e.g. `sha256=...`).
        /// Required for the generic mode; ignored when
        /// `--provider` is set (the per-provider verifier reads
        /// the `--header` entries instead).
        #[arg(long, default_value = "")]
        signature: String,
        /// Env-var name holding the shared HMAC secret.
        #[arg(long)]
        secret_env: String,
        /// File containing the raw webhook body bytes.
        #[arg(long, value_name = "FILE")]
        body_file: PathBuf,
        /// Provider preset: `github`, `slack`, or `linear`. Selects
        /// the per-provider header conventions and (Slack) the
        /// timestamp replay-protection window.
        #[arg(long)]
        provider: Option<String>,
        /// `Header-Name=value` pair (repeatable) to feed into the
        /// per-provider verifier. Required for `--provider` modes
        /// (e.g. `--header X-Hub-Signature-256=sha256=...` for
        /// github, plus `X-Slack-Signature` and
        /// `X-Slack-Request-Timestamp` for slack).
        #[arg(long = "header", value_name = "NAME=VALUE")]
        headers: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ConnectorsOauthCommand {
    /// Initiate an OAuth2 PKCE authorization flow. Generates a
    /// state + code verifier + code challenge and prints the
    /// provider's authorization URL the user should open.
    Init {
        /// Provider: `gmail`, `slack`, or `ms365`.
        provider: String,
        /// OAuth2 client id registered with the provider.
        #[arg(long)]
        client_id: String,
        /// Redirect URI registered with the provider. Defaults to
        /// `http://localhost:8765/oauth/callback`.
        #[arg(long)]
        redirect_uri: Option<String>,
        /// OAuth2 scopes (repeatable). Defaults to provider-shipped
        /// minimums (gmail: readonly + compose + send, slack:
        /// channels:history + chat:write, ms365: Mail.Read + Mail.Send + Calendars.Read).
        #[arg(long)]
        scope: Vec<String>,
    },
    /// Force-rotate an OAuth2 token by exercising the refresh
    /// endpoint with the supplied `(access, refresh)` pair. Prints
    /// the new pair so the operator can persist it. The production
    /// path consults the encrypted token store; this CLI surface
    /// is dev-friendly.
    Rotate {
        /// Provider: `gmail` or `slack`.
        provider: String,
        /// Token id to associate with the rotated tokens.
        #[arg(long)]
        token_id: String,
        /// Current access token (the soon-to-be-stale one).
        #[arg(long)]
        access_token: String,
        /// Current refresh token.
        #[arg(long)]
        refresh_token: String,
        /// OAuth2 client id.
        #[arg(long)]
        client_id: String,
        /// OAuth2 client secret.
        #[arg(long)]
        client_secret: String,
    },
}
