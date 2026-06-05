    use super::*;
    use corvid_ast::File;
    use corvid_guarantees::lookup as lookup_guarantee;

    fn parse_source(source: &str) -> File {
        let tokens = lex(source).expect("lex");
        let (file, errors) = parse_file(&tokens);
        assert!(errors.is_empty(), "parse errors: {errors:?}");
        file
    }

    fn descriptor_with_claim_ids(ids: &[&str]) -> String {
        let claims = ids
            .iter()
            .map(|id| {
                let guarantee = lookup_guarantee(id).expect("registered guarantee");
                serde_json::json!({
                    "id": guarantee.id,
                    "kind": guarantee.kind.slug(),
                    "class": guarantee.class.slug(),
                    "phase": guarantee.phase.slug(),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "corvid_abi_version": corvid_abi::CORVID_ABI_VERSION,
            "compiler_version": "test",
            "source_path": "test.cor",
            "generated_at": "1970-01-01T00:00:00Z",
            "agents": [],
            "prompts": [],
            "tools": [],
            "types": [],
            "stores": [],
            "approval_sites": [],
            "claim_guarantees": claims,
        })
        .to_string()
    }

    #[test]
    fn signed_claim_coverage_accepts_registered_contracts() {
        let file = parse_source(
            r#"
effect transfer:
    cost: $0.01

tool issue_refund(id: String) -> String dangerous uses transfer

@budget($0.50)
@replayable
pub extern "c"
agent refund(id: String) -> String uses transfer:
    approve issue_refund(id)
    return issue_refund(id)
"#,
        );
        let descriptor =
            descriptor_with_claim_ids(corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS);
        validate_signed_claim_coverage(&file, &descriptor).expect("coverage accepted");
    }

    #[test]
    fn signed_claim_coverage_rejects_missing_declared_contract_id() {
        let file = parse_source(
            r#"
tool issue_refund(id: String) -> String dangerous

pub extern "c"
agent refund(id: String) -> String:
    approve issue_refund(id)
    return issue_refund(id)
"#,
        );
        let ids = corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS
            .iter()
            .copied()
            .filter(|id| *id != "approval.dangerous_call_requires_token")
            .collect::<Vec<_>>();
        let descriptor = descriptor_with_claim_ids(&ids);
        let err = validate_signed_claim_coverage(&file, &descriptor)
            .expect_err("missing approval claim must reject signing");
        assert!(
            err.to_string()
                .contains("approval.dangerous_call_requires_token"),
            "{err:#}"
        );
    }

    #[test]
    fn signed_claim_coverage_rejects_out_of_scope_contract_id() {
        let file = parse_source(
            r#"
pub extern "c"
agent answer(x: Int) -> Int:
    return x
"#,
        );
        let mut ids = corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS.to_vec();
        ids.push("platform.signing_key_compromise");
        let descriptor = descriptor_with_claim_ids(&ids);
        let err = validate_signed_claim_coverage(&file, &descriptor)
            .expect_err("out-of-scope claim must reject signing");
        assert!(
            err.to_string().contains("out_of_scope"),
            "{err:#}"
        );
    }

    /// Slice 33Q3 positive: an agent annotated `@trust(<level>)`
    /// declares a signable constraint that maps to the new
    /// `trust.constraint_enforcement` guarantee id. When the
    /// descriptor's `claim_guarantees` array carries that id,
    /// `validate_signed_claim_coverage` accepts the signing — closes
    /// the bug anonymous-2026-06-04 round-2 P2 reported, where
    /// `@trust(...)` and `corvid build --sign` were mutually
    /// exclusive because no `trust.*` row existed in the registry.
    ///
    /// The agent below mirrors the `mutation_budget_within_limit_is_ok`
    /// shape but pins the trust-claim path specifically; the
    /// `@budget` is included so the build accepts both claims as a
    /// realistic mixed-annotation use case.
    #[test]
    fn signed_claim_coverage_accepts_trust_constrained_agent() {
        let file = parse_source(
            r#"
effect transfer_money:
    cost: $0.50
    trust: human_required
    reversible: false

type Receipt:
    id: String

tool issue_refund(id: String, amount: Float) -> Receipt dangerous uses transfer_money

@budget($1.00)
@trust(human_required)
pub extern "c"
agent refund(id: String, amount: Float) -> Receipt uses transfer_money:
    approve IssueRefund(id, amount)
    return issue_refund(id, amount)
"#,
        );
        let descriptor =
            descriptor_with_claim_ids(corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS);
        validate_signed_claim_coverage(&file, &descriptor)
            .expect("trust-constrained agent must accept signing");
    }

    /// Slice 33Q3 adversarial: when the descriptor's
    /// `claim_guarantees` array does NOT include
    /// `trust.constraint_enforcement`, an agent declaring `@trust(...)`
    /// must reject signing with the missing-claim error. Confirms the
    /// new id is load-bearing — drop it from the descriptor and the
    /// build refuses, just like dropping `approval.dangerous_call_requires_token`
    /// rejects an `approve`-using agent.
    #[test]
    fn signed_claim_coverage_rejects_trust_when_id_missing_from_descriptor() {
        let file = parse_source(
            r#"
effect transfer_money:
    cost: $0.50
    trust: human_required
    reversible: false

type Receipt:
    id: String

tool issue_refund(id: String, amount: Float) -> Receipt dangerous uses transfer_money

@budget($1.00)
@trust(human_required)
pub extern "c"
agent refund(id: String, amount: Float) -> Receipt uses transfer_money:
    approve IssueRefund(id, amount)
    return issue_refund(id, amount)
"#,
        );
        let ids = corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS
            .iter()
            .copied()
            .filter(|id| *id != "trust.constraint_enforcement")
            .collect::<Vec<_>>();
        let descriptor = descriptor_with_claim_ids(&ids);
        let err = validate_signed_claim_coverage(&file, &descriptor)
            .expect_err("missing trust.constraint_enforcement must reject signing");
        assert!(
            err.to_string().contains("trust.constraint_enforcement"),
            "rejection must name the missing trust id; got: {err:#}"
        );
    }

    /// Slice 35-N positive: a `Decl::Schedule` raises a require for
    /// `jobs.cron_schedule_durable` and the gate accepts when the
    /// descriptor includes that id.
    #[test]
    fn signed_claim_coverage_walks_schedule_decl() {
        let file = parse_source(
            r#"
effect send_email:
    cost: $0.05

agent daily_brief(user_id: String) -> String uses send_email:
    return user_id

schedule "0 8 * * *" zone "America/New_York" -> daily_brief("u1") uses send_email
"#,
        );
        let descriptor =
            descriptor_with_claim_ids(corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS);
        validate_signed_claim_coverage(&file, &descriptor)
            .expect("schedule decl must be accepted when jobs.cron_schedule_durable is in claims");
    }

    /// Slice 35-N adversarial: a `Decl::Schedule` without the
    /// `jobs.cron_schedule_durable` claim id in the descriptor must
    /// be refused: a signed cdylib that ships a cron trigger must
    /// acknowledge that contract.
    #[test]
    fn signed_claim_coverage_rejects_schedule_without_jobs_coverage() {
        let file = parse_source(
            r#"
effect send_email:
    cost: $0.05

agent daily_brief(user_id: String) -> String uses send_email:
    return user_id

schedule "0 8 * * *" zone "America/New_York" -> daily_brief("u1") uses send_email
"#,
        );
        let ids = corvid_guarantees::SIGNED_CDYLIB_CLAIM_GUARANTEE_IDS
            .iter()
            .copied()
            .filter(|id| *id != "jobs.cron_schedule_durable")
            .collect::<Vec<_>>();
        let descriptor = descriptor_with_claim_ids(&ids);
        let err = validate_signed_claim_coverage(&file, &descriptor)
            .expect_err("schedule without cron_schedule_durable must reject signing");
        assert!(
            err.to_string().contains("jobs.cron_schedule_durable"),
            "{err:#}"
        );
    }
