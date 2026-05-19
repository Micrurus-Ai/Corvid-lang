//! `corvid review-queue list` — read `ReviewQueueRecord` JSONL,
//! optionally rank by the `cost_of_being_wrong` policy, render.
//!
//! Proves `review_queue.cost_of_being_wrong_ranking` end-to-end:
//! the runtime envelopes (`crates/corvid-runtime/src/review_queue.rs`)
//! carry `cost_of_being_wrong: f64`; this CLI surface lets a
//! backend operator capture the queue, sort it by the highest-cost
//! pending review, and feed the result into a triage workflow.

use anyhow::{Context, Result};
use corvid_runtime::review_queue::{ReviewQueueRecord, ReviewStatus};
use std::io::{self, Read};
use std::path::Path;

const RANK_COST_OF_BEING_WRONG: &str = "cost-of-being-wrong";

pub fn run_list(
    records_path: &Path,
    rank: Option<&str>,
    status_filter: Option<&str>,
    json: bool,
) -> Result<u8> {
    let raw = read_records_input(records_path)?;
    let mut records = parse_jsonl(&raw)?;

    if let Some(filter) = status_filter {
        let parsed = parse_status_filter(filter)?;
        records.retain(|record| record.status == parsed);
    }

    if let Some(rank_value) = rank {
        apply_ranking(&mut records, rank_value)?;
    }

    if json {
        let out = serde_json::to_string_pretty(&records)
            .context("serialise review queue records to JSON")?;
        println!("{out}");
    } else {
        print_table(&records);
    }
    Ok(0)
}

fn read_records_input(path: &Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("read review queue records from stdin")?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("read review queue records from {}", path.display()))
    }
}

fn parse_jsonl(raw: &str) -> Result<Vec<ReviewQueueRecord>> {
    let mut out = Vec::new();
    for (line_no, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: ReviewQueueRecord = serde_json::from_str(trimmed)
            .with_context(|| format!("parse review queue record on line {}", line_no + 1))?;
        out.push(record);
    }
    Ok(out)
}

fn parse_status_filter(filter: &str) -> Result<ReviewStatus> {
    match filter {
        "pending" => Ok(ReviewStatus::Pending),
        "approved" => Ok(ReviewStatus::Approved),
        "rejected" => Ok(ReviewStatus::Rejected),
        "escalated" => Ok(ReviewStatus::Escalated),
        other => anyhow::bail!(
            "unknown --status `{other}`; expected one of \
             pending|approved|rejected|escalated"
        ),
    }
}

fn apply_ranking(records: &mut [ReviewQueueRecord], rank: &str) -> Result<()> {
    if rank != RANK_COST_OF_BEING_WRONG {
        anyhow::bail!(
            "unknown --rank `{rank}`; expected `{RANK_COST_OF_BEING_WRONG}`"
        );
    }
    records.sort_by(|a, b| {
        b.cost_of_being_wrong
            .partial_cmp(&a.cost_of_being_wrong)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.review_id.cmp(&b.review_id))
    });
    Ok(())
}

fn print_table(records: &[ReviewQueueRecord]) {
    if records.is_empty() {
        println!("(no review-queue records)");
        return;
    }
    println!(
        "{:<24}  {:>14}  {:<18}  {:<10}  {:<16}  {}",
        "review_id", "cost_of_wrong", "reason", "status", "tenant", "trace_id"
    );
    for record in records {
        println!(
            "{:<24}  {:>14.2}  {:<18}  {:<10}  {:<16}  {}",
            truncate(&record.review_id, 24),
            record.cost_of_being_wrong,
            format!("{:?}", record.reason).to_lowercase(),
            format!("{:?}", record.status).to_lowercase(),
            truncate(&record.tenant_id, 16),
            record.trace_id,
        );
    }
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_runtime::review_queue::ReviewReason;

    fn sample(review_id: &str, cost: f64) -> ReviewQueueRecord {
        ReviewQueueRecord {
            review_id: review_id.to_string(),
            trace_id: format!("trace-{review_id}"),
            span_id: format!("span-{review_id}"),
            tenant_id: "tenant-1".to_string(),
            actor_id: "user-1".to_string(),
            reason: ReviewReason::HighRisk,
            status: ReviewStatus::Pending,
            cost_of_being_wrong: cost,
            source_prompt_hash: String::new(),
            model_fingerprint: String::new(),
            approval_id: String::new(),
            replay_key: String::new(),
            guarantee_id: String::new(),
            audit_event_id: String::new(),
            reviewer_actor_id: String::new(),
            decision_note: String::new(),
            created_ms: 0,
            resolved_ms: 0,
        }
    }

    fn jsonl(records: &[ReviewQueueRecord]) -> String {
        records
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rank_cost_of_being_wrong_sorts_highest_first() {
        let mut records = vec![
            sample("review-low", 5.0),
            sample("review-high", 100.0),
            sample("review-mid", 42.0),
        ];
        apply_ranking(&mut records, RANK_COST_OF_BEING_WRONG).unwrap();
        let order: Vec<&str> = records
            .iter()
            .map(|r| r.review_id.as_str())
            .collect();
        assert_eq!(order, vec!["review-high", "review-mid", "review-low"]);
    }

    #[test]
    fn rank_unknown_policy_refused() {
        let mut records = vec![sample("review-a", 1.0)];
        let err = apply_ranking(&mut records, "by-creation-time").unwrap_err();
        assert!(err.to_string().contains("unknown --rank"));
    }

    #[test]
    fn jsonl_round_trip_preserves_records() {
        let originals = vec![sample("review-a", 1.0), sample("review-b", 2.0)];
        let parsed = parse_jsonl(&jsonl(&originals)).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].review_id, "review-a");
        assert_eq!(parsed[1].review_id, "review-b");
    }

    #[test]
    fn parse_jsonl_skips_blank_lines() {
        let raw = format!(
            "\n{}\n\n{}\n",
            serde_json::to_string(&sample("review-a", 1.0)).unwrap(),
            serde_json::to_string(&sample("review-b", 2.0)).unwrap(),
        );
        let parsed = parse_jsonl(&raw).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn parse_status_filter_accepts_each_known_value() {
        assert_eq!(parse_status_filter("pending").unwrap(), ReviewStatus::Pending);
        assert_eq!(
            parse_status_filter("approved").unwrap(),
            ReviewStatus::Approved
        );
        assert_eq!(
            parse_status_filter("rejected").unwrap(),
            ReviewStatus::Rejected
        );
        assert_eq!(
            parse_status_filter("escalated").unwrap(),
            ReviewStatus::Escalated
        );
        assert!(parse_status_filter("queued").is_err());
    }
}
