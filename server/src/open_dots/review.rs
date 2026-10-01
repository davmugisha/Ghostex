//! Rule review (Open Dot's `agent/review.ts`): "your dot knows when to take action, and when to ask
//! for approval". The user writes rules as "When your dot wants to <action>, it should
//! allow / ask / never"; a small model decides which rules apply to a concrete pending action.
//! "Never" beats "ask" beats "allow". With no reviewer, anything not explicitly safe asks.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! The reviewer is a one-shot agent CLI call (Claude Haiku at low effort, else Codex), the same way session titles are generated, because the dots run on the user's agent subscriptions and have no API key of their own.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::model::{Dot, Rule};
use super::store;
use crate::paths::GxserverPaths;

const REVIEW_TIMEOUT: Duration = Duration::from_secs(45);
const CACHE_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug)]
pub(crate) struct Verdict {
    /// `allow|ask|never`
    pub(crate) decision: String,
    pub(crate) rule: Option<Rule>,
}

static CACHE: Mutex<Option<HashMap<String, (Instant, Verdict)>>> = Mutex::new(None);

fn cache_key(rules: &[Rule], action: &str, fallback: &str) -> String {
    let fingerprint = rules
        .iter()
        .map(|rule| format!("{}:{}:{}", rule.id, rule.decision, rule.action))
        .collect::<Vec<_>>()
        .join("|");
    format!("{fingerprint}\n{fallback}\n{action}")
}

fn cached(key: &str) -> Option<Verdict> {
    let mut guard = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
    cache.get(key).map(|(_, verdict)| verdict.clone())
}

fn remember(key: String, verdict: &Verdict) {
    let mut guard = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    guard
        .get_or_insert_with(HashMap::new)
        .insert(key, (Instant::now(), verdict.clone()));
}

fn rule_line(index: usize, rule: &Rule) -> String {
    let decision = match rule.decision.as_str() {
        "allow" => "allow automatically",
        "never" => "never allow",
        _ => "ask first",
    };
    format!(
        "{}. When the dot wants to {} -> {decision}",
        index + 1,
        rule.action.trim()
    )
}

fn review_prompt(rules: &[Rule], action: &str) -> String {
    let list = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| rule_line(index, rule))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You gate actions of a personal AI agent. Decide which of the user's rules (if any) apply to the pending action. A rule applies only if the action clearly falls under it. Do not use any tools.\n\nRules:\n{list}\n\nPending action: the dot wants to {action}\n\nReply with ONLY a JSON object, no other text, in exactly this form: {{\"applying_rules\": [<numbers of every applying rule>]}}. Use an empty list if none apply."
    )
}

/// The numbers inside the first `{...}` that parses as `{ "applying_rules": [..] }`.
fn parse_applying(output: &str) -> Option<Vec<usize>> {
    let mut search = output;
    while let Some(start) = search.find('{') {
        let candidate = &search[start..];
        if let Some(end) = candidate.find('}') {
            if let Ok(value) = serde_json::from_str::<Value>(&candidate[..=end]) {
                if let Some(list) = value.get("applying_rules").and_then(Value::as_array) {
                    return Some(
                        list.iter()
                            .filter_map(Value::as_u64)
                            .map(|number| number as usize)
                            .collect(),
                    );
                }
            }
        }
        search = &candidate[1..];
    }
    None
}

/// Which CLI reviews: the dot's own family when it is Claude or Codex, else whichever is installed.
fn reviewer_agent(dot: &Dot) -> Option<&'static str> {
    let installed = |binary: &str| crate::platform::live_path::find(binary, &[]).is_some();
    let preferred = if dot.agent_id.to_ascii_lowercase().contains("codex") {
        ["codex", "claude"]
    } else {
        ["claude", "codex"]
    };
    preferred.into_iter().find(|binary| installed(binary))
}

async fn run_reviewer(paths: &GxserverPaths, agent: &str, prompt: &str) -> Result<String, String> {
    let delimiter = format!(
        "ghostex_OPENDOTS_REVIEW_{}",
        chrono::Utc::now().timestamp_millis()
    );
    let shell_command =
        crate::server::build_title_generation_command(agent, agent, &delimiter, prompt)?;
    let shell = crate::platform::shell::command_shell();
    let mut child = tokio::process::Command::new(&shell.executable);
    child.args(shell.interactive_script_args(&shell_command));
    child.current_dir(&paths.home_dir);
    child.envs(crate::server::internal_prompt_generation_environment(
        &paths.home_dir,
    ));
    child.stdin(std::process::Stdio::null());
    child.stdout(std::process::Stdio::piped());
    child.stderr(std::process::Stdio::piped());
    child.kill_on_drop(true);
    let output = tokio::time::timeout(REVIEW_TIMEOUT, child.output())
        .await
        .map_err(|_| "the rule review timed out".to_string())?
        .map_err(|error| format!("the rule review failed: {error}"))?;
    if !output.status.success() {
        return Err(format!("the rule review exited {:?}", output.status.code()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Checks a pending action against the dot's rules. `fallback` is the tool's default decision.
pub(crate) async fn review(
    paths: &GxserverPaths,
    dot: &Dot,
    action: &str,
    fallback: &str,
) -> Verdict {
    let rules = store::open(paths)
        .and_then(|db| store::rules_for(&db, &dot.id))
        .unwrap_or_default();
    if rules.is_empty() {
        return Verdict {
            decision: fallback.to_string(),
            rule: None,
        };
    }
    let key = cache_key(&rules, action, fallback);
    if let Some(verdict) = cached(&key) {
        return verdict;
    }
    // CDXC:OpenDots 2026-10-01 WHY:
    // When the reviewer cannot run (timed out, offline, no CLI, unreadable reply) nobody knows which rule applies, so any "ask first" or "never" rule this dot follows makes it ask; only with nothing but allow rules does a tool that runs freely by default go ahead. Falling back to the tool's default let a "never" rule through whenever the reviewer failed.
    let restrictive = rules
        .iter()
        .any(|rule| matches!(rule.decision.as_str(), "ask" | "never"));
    let conservative = Verdict {
        decision: if fallback == "allow" && !restrictive {
            "allow"
        } else {
            "ask"
        }
        .to_string(),
        rule: None,
    };
    let Some(agent) = reviewer_agent(dot) else {
        return conservative;
    };
    let output = match run_reviewer(paths, agent, &review_prompt(&rules, action)).await {
        Ok(output) => output,
        Err(_) => return conservative,
    };
    let Some(applying) = parse_applying(&output) else {
        return conservative;
    };
    let matched: Vec<&Rule> = applying
        .into_iter()
        .filter_map(|number| number.checked_sub(1).and_then(|index| rules.get(index)))
        .collect();
    // CDXC:OpenDots 2026-10-01 WHY:
    // A dot's own rules refine the rules for every dot, so they decide first (a global "never" stays absolute); within one scope "ask first" wins a conflict as in Open Dot. With one flat list, Always allow on a card raised by a rule for every dot could never take effect for that dot.
    let in_scope = |own: bool| -> Vec<&Rule> {
        matched
            .iter()
            .copied()
            .filter(|rule| rule.dot_id.is_some() == own)
            .collect()
    };
    let pick = |rules: &[&Rule], decision: &str| {
        rules
            .iter()
            .find(|rule| rule.decision == decision)
            .map(|rule| (*rule).clone())
    };
    let own = in_scope(true);
    let global = in_scope(false);
    let decide = |rules: &[&Rule]| {
        pick(rules, "never")
            .or_else(|| pick(rules, "ask"))
            .or_else(|| pick(rules, "allow"))
    };
    let verdict = match pick(&global, "never")
        .or_else(|| decide(&own))
        .or_else(|| decide(&global))
    {
        Some(rule) => Verdict {
            decision: rule.decision.clone(),
            rule: Some(rule),
        },
        None => Verdict {
            decision: fallback.to_string(),
            rule: None,
        },
    };
    remember(key, &verdict);
    verdict
}
