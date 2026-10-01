//! The OpenDots snapshot as gxserver sends it (`/api/openDotsRpc` `snapshot`, docs CONTRACT §1),
//! read leniently so a field a newer daemon adds or an older one leaves out never blanks the page,
//! plus the small formatting rules Open Dot's pages share (`lib/status.ts`).

use chrono::{Local, TimeZone as _};
use ghostex_gx_core::open_dots::DotFaceStatus;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotsSnapshot {
    pub(crate) enabled: bool,
    pub(crate) dots: Vec<Dot>,
    pub(crate) rules: Vec<DotRule>,
    pub(crate) memories: Vec<DotMemory>,
    pub(crate) skills: Vec<DotSkill>,
    pub(crate) routines: Vec<DotRoutine>,
    pub(crate) triggers: Vec<DotTrigger>,
    pub(crate) passwords: Vec<DotPassword>,
    pub(crate) approvals: Vec<DotApproval>,
    pub(crate) updates: Vec<DotUpdate>,
    pub(crate) handoffs: Vec<DotHandoff>,
    pub(crate) teams: Vec<DotTeam>,
    pub(crate) settings: DotsSettings,
}

impl DotsSnapshot {
    pub(crate) fn dot(&self, dot_id: &str) -> Option<&Dot> {
        self.dots.iter().find(|dot| dot.id == dot_id)
    }

    pub(crate) fn pending_approvals(&self) -> impl Iterator<Item = &DotApproval> {
        self.approvals
            .iter()
            .filter(|approval| approval.status == "pending")
    }

    pub(crate) fn agent(&self, agent_id: &str) -> Option<&DotAgent> {
        self.settings
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Dot {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) purpose: String,
    pub(crate) instructions: String,
    pub(crate) look: Value,
    pub(crate) agent_id: String,
    pub(crate) model: Option<String>,
    pub(crate) paused: bool,
    pub(crate) status: String,
    pub(crate) activity: Option<String>,
    pub(crate) project_id: String,
    pub(crate) folder: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

impl Dot {
    pub(crate) fn face_status(&self) -> DotFaceStatus {
        DotFaceStatus::parse(&self.status)
    }

    /// Open Dot's `statusLine`.
    pub(crate) fn status_line(&self) -> String {
        match self.face_status() {
            DotFaceStatus::Paused => "Paused".to_string(),
            DotFaceStatus::Waiting => "Waiting for you".to_string(),
            DotFaceStatus::Working => self
                .activity
                .clone()
                .filter(|activity| !activity.trim().is_empty())
                .unwrap_or_else(|| "Working".to_string()),
            DotFaceStatus::Idle => {
                if self.purpose.trim().is_empty() {
                    "Ready".to_string()
                } else {
                    self.purpose.clone()
                }
            }
        }
    }

    /// The short status a card prints under the dot's job: the activity while working, else
    /// Open Dot's `statusLabel`.
    pub(crate) fn status_word(&self) -> String {
        match self.face_status() {
            DotFaceStatus::Working => self
                .activity
                .clone()
                .filter(|activity| !activity.trim().is_empty())
                .unwrap_or_else(|| "Working".to_string()),
            status => status.label().to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotRule {
    pub(crate) id: String,
    pub(crate) dot_id: Option<String>,
    pub(crate) action: String,
    pub(crate) decision: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotMemory {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) text: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotSkill {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) body: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotRoutine {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) name: String,
    pub(crate) instruction: String,
    pub(crate) schedule: String,
    pub(crate) enabled: bool,
    pub(crate) last_run_at: Option<i64>,
    pub(crate) next_run_at: Option<i64>,
    pub(crate) session_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotTrigger {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) slug: String,
    pub(crate) toolkit: String,
    pub(crate) name: String,
    pub(crate) instruction: String,
    pub(crate) enabled: bool,
    pub(crate) last_fired_at: Option<i64>,
    pub(crate) last_error: Option<String>,
    pub(crate) session_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotPassword {
    pub(crate) id: String,
    pub(crate) site: String,
    pub(crate) username: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotApproval {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) options: Vec<String>,
    pub(crate) rule_action: Option<String>,
    pub(crate) toolkit: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) answer: Option<String>,
    pub(crate) created_at: i64,
}

impl DotApproval {
    pub(crate) fn heading(&self) -> &'static str {
        match self.kind.as_str() {
            "question" => "Question for you",
            "connect" => "Connect an app",
            _ => "Needs your approval",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotUpdateFile {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotUpdate {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) text: String,
    pub(crate) files: Vec<DotUpdateFile>,
    pub(crate) read: bool,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotHandoff {
    pub(crate) id: String,
    pub(crate) from_dot_id: String,
    pub(crate) to_dot_id: String,
    pub(crate) message: String,
    pub(crate) status: String,
    pub(crate) reply: Option<String>,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotTeam {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) lead_id: String,
    pub(crate) member_ids: Vec<String>,
    pub(crate) project_id: String,
    pub(crate) session_id: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotsSettings {
    pub(crate) composio: ComposioInfo,
    pub(crate) triggers_key: Option<String>,
    pub(crate) vault: String,
    pub(crate) chrome: Option<String>,
    pub(crate) agents: Vec<DotAgent>,
    pub(crate) default_agent_id: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ComposioInfo {
    pub(crate) signed_in: bool,
    pub(crate) apps: Vec<ComposioApp>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ComposioApp {
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) connected: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotAgent {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) models: Vec<DotModel>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotModel {
    pub(crate) id: String,
    pub(crate) label: String,
}

/// `browserScreenshot`'s answer.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotScreen {
    pub(crate) image: Option<String>,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) live: bool,
    pub(crate) taken_over: bool,
}

/// One `listFiles` entry.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DotFileEntry {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) is_dir: bool,
}

/// One `triggerApps` entry.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct TriggerApp {
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) connected: bool,
}

/// One `triggerTypes` entry.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct TriggerType {
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) fields: Vec<TriggerField>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct TriggerField {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) field_type: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) required: bool,
    #[serde(rename = "enum")]
    pub(crate) choices: Option<Vec<String>>,
    pub(crate) default: Option<Value>,
}

/// A conversation in a dot's project, from `/api/listSessions` (the dot page's Chats tab).
#[derive(Clone, Debug, Default)]
pub(crate) struct DotChat {
    pub(crate) session_id: String,
    pub(crate) title: String,
    pub(crate) sleeping: bool,
    pub(crate) updated: String,
}

impl DotChat {
    pub(crate) fn from_session_row(row: &Value) -> Option<DotChat> {
        let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::trim);
        let session_id = text("sessionId").filter(|id| !id.is_empty())?.to_string();
        if text("lifecycleState") == Some("stopped") || text("kind") == Some("terminal") {
            return None;
        }
        let title = ["title", "displayTitle", "agentTitle", "name"]
            .iter()
            .find_map(|key| text(key).filter(|title| !title.is_empty()))
            .unwrap_or("New chat")
            .to_string();
        Some(DotChat {
            session_id,
            title,
            sleeping: text("lifecycleState") == Some("sleeping"),
            updated: text("updatedAt")
                .or_else(|| text("lastActiveAt"))
                .unwrap_or_default()
                .to_string(),
        })
    }
}

pub(crate) fn now_ms() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Open Dot's `timeAgo`.
pub(crate) fn time_ago(at: i64, now: i64) -> String {
    let seconds = ((now - at).max(0) as f64 / 1000.0).round() as i64;
    if seconds < 45 {
        return "just now".to_string();
    }
    let minutes = (seconds as f64 / 60.0).round() as i64;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%b %-d").to_string())
        .unwrap_or_default()
}

/// A routine's or trigger's time, as Open Dot prints it (`Mon 8:00 AM`).
pub(crate) fn weekday_time(at: i64) -> String {
    Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%a %-I:%M %p").to_string())
        .unwrap_or_default()
}

/// An ISO time from a session row, as `time_ago`.
pub(crate) fn iso_time_ago(iso: &str, now: i64) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|time| time_ago(time.timestamp_millis(), now))
        .unwrap_or_default()
}

/// Open Dot's file `size`.
pub(crate) fn file_size(bytes: u64) -> String {
    if bytes > 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes > 1024 {
        format!("{} KB", (bytes as f64 / 1024.0).round() as u64)
    } else {
        format!("{bytes} B")
    }
}

/// An update's text as one line for the "Latest" list (Open Dot strips Markdown marks).
pub(crate) fn one_line(text: &str, limit: usize) -> String {
    let flat: String = text
        .chars()
        .filter(|character| !matches!(character, '#' | '*' | '_' | '`' | '>' | '|'))
        .map(|character| {
            if character.is_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let mut flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > limit {
        flat = flat.chars().take(limit).collect::<String>() + "…";
    }
    flat
}
