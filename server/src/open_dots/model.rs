//! The OpenDots records exactly as every client reads them (docs/2026-10-01/opendots/CONTRACT.md
//! §1): camelCase JSON, ids `<prefix>_<16 hex>`, times in ms since the epoch.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const DOT_SHAPES: [&str; 3] = ["round", "chubby", "tall"];
pub(crate) const DOT_MATERIALS: [&str; 4] = ["soft", "glossy", "velvet", "toon"];
pub(crate) const DOT_EYES: [&str; 4] = ["classic", "happy", "wide", "wink"];
pub(crate) const DOT_ACCESSORIES: [&str; 7] = [
    "none",
    "bow",
    "cap",
    "antenna",
    "halo",
    "sprout",
    "headphones",
];
const DOT_COLORS: [&str; 12] = [
    "#ffa7c4", "#ffb8a1", "#ffd98a", "#c8ec8f", "#97e0b8", "#8fdcd6", "#9fcbff", "#b9b3ff",
    "#dcb4f5", "#f4efe6", "#c9c3bb", "#6e6a78",
];
const DOT_FEET_COLORS: [&str; 12] = [
    "#d8195f", "#e0492d", "#d98a00", "#4f9a2f", "#1f8f5f", "#138a8a", "#2a6fdb", "#5b46d6",
    "#9b3cc4", "#3b3842", "#8a5a3c", "#f4f4f4",
];

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

/// Open Dot's `normalizeLook` (the same rules as `packages/gx-core/src/open_dots.rs`
/// `DotLook::from_value`): any stored or wire value becomes a valid look.
pub(crate) fn normalize_look(value: Option<&Value>) -> Value {
    let text = |key: &str| {
        value
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
    };
    let one_of = |key: &str, allowed: &[&str], fallback: &str| -> String {
        text(key)
            .filter(|candidate| allowed.contains(candidate))
            .unwrap_or(fallback)
            .to_string()
    };
    let color = text("color")
        .filter(|color| is_hex_color(color))
        .unwrap_or("#ffa7c4")
        .to_string();
    let accent = text("accent")
        .filter(|accent| DOT_FEET_COLORS.contains(accent))
        .map(str::to_string)
        .unwrap_or_else(|| {
            DOT_COLORS
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(&color))
                .map(|index| DOT_FEET_COLORS[index])
                .unwrap_or("#d8195f")
                .to_string()
        });
    json!({
        "shape": one_of("shape", &DOT_SHAPES, "round"),
        "color": color,
        "accent": accent,
        "eyeColor": text("eyeColor").filter(|color| is_hex_color(color)).unwrap_or("#2b5fd9"),
        "material": one_of("material", &DOT_MATERIALS, "soft"),
        "eyes": one_of("eyes", &DOT_EYES, "classic"),
        "accessory": one_of("accessory", &DOT_ACCESSORIES, "none"),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Dot {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) purpose: String,
    pub(crate) instructions: String,
    pub(crate) look: Value,
    pub(crate) agent_id: String,
    pub(crate) model: Option<String>,
    pub(crate) paused: bool,
    /// Derived: `paused > waiting > working > idle` (status.rs).
    pub(crate) status: String,
    pub(crate) activity: Option<String>,
    pub(crate) project_id: String,
    pub(crate) folder: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    #[serde(skip)]
    pub(crate) slug: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rule {
    pub(crate) id: String,
    pub(crate) dot_id: Option<String>,
    pub(crate) action: String,
    pub(crate) decision: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Memory {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) text: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Skill {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) body: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Routine {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) name: String,
    pub(crate) instruction: String,
    pub(crate) schedule: String,
    pub(crate) enabled: bool,
    pub(crate) last_run_at: Option<i64>,
    pub(crate) next_run_at: Option<i64>,
    pub(crate) session_id: Option<String>,
    pub(crate) created_at: i64,
    /// Why the last run could not start (its agent did not launch), `None` after a good run.
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Trigger {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) composio_id: String,
    pub(crate) slug: String,
    pub(crate) toolkit: String,
    pub(crate) name: String,
    pub(crate) config: Value,
    pub(crate) instruction: String,
    pub(crate) enabled: bool,
    pub(crate) last_fired_at: Option<i64>,
    pub(crate) last_error: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) created_at: i64,
}

/// The secret never leaves gxserver: this is all a client sees.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Password {
    pub(crate) id: String,
    pub(crate) site: String,
    pub(crate) username: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Approval {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: Option<String>,
    /// `approval|question|connect`
    pub(crate) kind: String,
    /// `pending|approved|denied|answered|expired`
    pub(crate) status: String,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) options: Vec<String>,
    pub(crate) rule_action: Option<String>,
    pub(crate) toolkit: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) answer: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) resolved_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateFile {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Update {
    pub(crate) id: String,
    pub(crate) dot_id: String,
    pub(crate) project_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) text: String,
    pub(crate) files: Vec<UpdateFile>,
    pub(crate) read: bool,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Handoff {
    pub(crate) id: String,
    pub(crate) from_dot_id: String,
    pub(crate) to_dot_id: String,
    pub(crate) message: String,
    /// `sent|working|replied|failed`
    pub(crate) status: String,
    pub(crate) reply: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) replied_at: Option<i64>,
    /// The asking conversation the reply goes back to (`None` when the asker had no session).
    #[serde(skip)]
    pub(crate) from_project_id: Option<String>,
    #[serde(skip)]
    pub(crate) from_session_id: Option<String>,
    /// The other dot's conversation that works on it.
    #[serde(skip)]
    pub(crate) to_project_id: Option<String>,
    #[serde(skip)]
    pub(crate) to_session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Team {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) lead_id: String,
    pub(crate) member_ids: Vec<String>,
    pub(crate) project_id: String,
    pub(crate) session_id: String,
    pub(crate) created_at: i64,
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `<prefix>_<16 hex>`.
pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}_{:016x}", rand::random::<u64>())
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
