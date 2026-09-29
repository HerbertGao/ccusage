use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::{
    BucketKind, LoadedEntry, Result, SessionAccumulator, cli::AgentReportKind, cli::WeekDay,
    summarize_by_key, summarize_summaries_by_bucket, totals_json,
};

pub fn report_from_rows(
    rows: &[crate::UsageSummary],
    entries: &[LoadedEntry],
    kind: AgentReportKind,
    by_provider: bool,
) -> Value {
    let rows_json = rows
        .iter()
        .map(|row| ccusage_core::agent_summary_json(row, kind, kind == AgentReportKind::Session))
        .collect::<Vec<_>>();
    let mut report = json!({
        rows_key(kind): rows_json,
        "totals": totals_json(rows),
    });
    if by_provider {
        let providers = provider_rows(entries, kind);
        if !providers.is_empty() {
            report["providers"] = json!(providers);
        }
    }
    report
}

pub fn summarize_entries(
    entries: &[LoadedEntry],
    kind: AgentReportKind,
) -> Result<Vec<crate::UsageSummary>> {
    match kind {
        AgentReportKind::Daily => summarize_by_key(
            entries,
            |entry| entry.date.clone(),
            |date| (date.to_string(), None),
        ),
        AgentReportKind::Monthly => {
            let daily = summarize_by_key(
                entries,
                |entry| entry.date.clone(),
                |date| (date.to_string(), None),
            )?;
            Ok(summarize_summaries_by_bucket(
                &daily,
                BucketKind::Monthly,
                WeekDay::Sunday,
            ))
        }
        AgentReportKind::Session => {
            let mut groups = BTreeMap::<String, SessionAccumulator>::new();
            for entry in entries {
                groups
                    .entry(entry.session_id.to_string())
                    .or_default()
                    .add_entry(entry);
            }
            groups
                .into_values()
                .map(|group| group.into_summary())
                .collect()
        }
        AgentReportKind::Weekly => {
            let daily = summarize_entries(entries, AgentReportKind::Daily)?;
            Ok(summarize_summaries_by_bucket(
                &daily,
                BucketKind::Weekly,
                WeekDay::Sunday,
            ))
        }
    }
}

fn provider_rows(
    entries: &[LoadedEntry],
    kind: AgentReportKind,
) -> BTreeMap<String, serde_json::Value> {
    let mut groups = BTreeMap::<(String, String, String, String), Vec<LoadedEntry>>::new();
    for entry in entries {
        let Some(key) = entry.data.request_id.as_deref() else {
            continue;
        };
        let Some((provider, thinking)) = key.split_once("\u{1f}|") else {
            continue;
        };
        let period = match kind {
            AgentReportKind::Daily => entry.date.to_string(),
            AgentReportKind::Monthly => entry.date[..7].to_string(),
            AgentReportKind::Session => entry.session_id.to_string(),
            AgentReportKind::Weekly => entry.date.to_string(),
        };
        let model = entry
            .data
            .message
            .model
            .clone()
            .unwrap_or_else(|| "unknown".into());
        groups
            .entry((provider.to_string(), period, model, thinking.to_string()))
            .or_default()
            .push(entry.clone());
    }

    let mut result = BTreeMap::new();
    for ((provider, period, model, thinking), scoped) in groups {
        let rows = summarize_entries(&scoped, kind).unwrap_or_default();
        let provider_entry = result.entry(provider).or_insert_with(|| json!({}));
        let model_entry = provider_entry[rows_key(kind)][&period][&model].take();
        let mut model_value = if model_entry.is_null() {
            json!({"thinkingLevels": {}})
        } else {
            model_entry
        };
        model_value["thinkingLevels"][thinking] = json!({
            "totals": totals_json(&rows),
        });
        provider_entry[rows_key(kind)][&period][&model] = model_value;
    }
    result
}

fn rows_key(kind: AgentReportKind) -> &'static str {
    match kind {
        AgentReportKind::Daily => "daily",
        AgentReportKind::Weekly => "weekly",
        AgentReportKind::Monthly => "monthly",
        AgentReportKind::Session => "sessions",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_has_zero_totals_object() {
        let report = report_from_rows(&[], &[], AgentReportKind::Daily, false);

        assert_eq!(report["daily"], json!([]));
        assert!(report["totals"].is_object());
        assert!(report.get("providers").is_none());
        assert_eq!(report["totals"]["inputTokens"], json!(0));
        assert_eq!(report["totals"]["outputTokens"], json!(0));
        assert_eq!(report["totals"]["cacheCreationTokens"], json!(0));
        assert_eq!(report["totals"]["cacheReadTokens"], json!(0));
        assert_eq!(report["totals"]["totalTokens"], json!(0));
        assert_eq!(report["totals"]["totalCost"].as_f64(), Some(0.0));
    }
}
