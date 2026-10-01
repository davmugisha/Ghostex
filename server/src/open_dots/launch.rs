//! How a session in a dot project reaches the dot's tools and instructions: the `ghostex dots mcp`
//! server and the generated `AGENTS.md`, passed as launch flags (CONTRACT §4). The flags live in the
//! session's saved base command, so resume, fork and account wrapping keep them, like a
//! coordinator's role.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::{dot_id_of_project, store, OPEN_DOTS_MCP_SERVER_NAME};

/// The `ghostex` CLI shipped beside this gxserver (the binary every session's prompt-editor wrapper
/// is pinned to), else `ghostex` on PATH.
pub(crate) fn dot_cli_executable() -> String {
    let sibling = std::env::current_exe().ok().map(|gxserver| {
        let gxserver = std::fs::canonicalize(&gxserver).unwrap_or(gxserver);
        gxserver.with_file_name(if cfg!(windows) {
            "ghostex.exe"
        } else {
            "ghostex"
        })
    });
    sibling
        .filter(|cli| crate::ghostex_cli::launchers::is_executable_file_sync(cli))
        .map(|cli| cli.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ghostex".to_string())
}

pub(crate) fn mcp_config_file(folder: &Path) -> PathBuf {
    folder.join(".ghostex").join("mcp.json")
}

pub(crate) fn agents_file(folder: &Path) -> PathBuf {
    folder.join("AGENTS.md")
}

/// `{ "mcpServers": { "ghostex-dots": { command, args } } }`.
pub(crate) fn mcp_config(dot_id: &str) -> Value {
    json!({
        "mcpServers": {
            OPEN_DOTS_MCP_SERVER_NAME: {
                "command": dot_cli_executable(),
                "args": ["dots", "mcp", "--dot", dot_id],
            }
        }
    })
}

/// What a create into a dot project needs from the dot: its id, folder, agent and model.
pub(crate) struct DotLaunch {
    pub(crate) dot_id: String,
    pub(crate) folder: PathBuf,
    /// The agent the dot runs on (`claude`, `codex`, a custom launcher id).
    pub(crate) agent_id: String,
    pub(crate) model: Option<String>,
}

/// The dot a project stands for, read from OpenDots' file beside `state.db`. `None` for every
/// other project, and for a dot project whose dot is gone.
pub(crate) fn dot_launch_for_project(state_db: &Connection, project: &Value) -> Option<DotLaunch> {
    let dot_id = dot_id_of_project(project)?;
    let db = store::open_beside_state_db(state_db)?;
    let dot = store::get_dot(&db, dot_id).ok().flatten()?;
    let folder = project
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(&dot.folder));
    Some(DotLaunch {
        dot_id: dot.id,
        folder,
        agent_id: dot.agent_id,
        model: dot.model.filter(|model| !model.trim().is_empty()),
    })
}

/// The create params with the dot's model filled in when the create names none, for the agent
/// families that take a launch model. `family` is the session's agent family and `dot_family` the
/// family of the agent the dot runs on.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The dot's model belongs to the dot's agent: a Codex chat started in a Claude dot launched with `--model claude-…` and failed, so the model is only applied when the session runs the dot's own agent family.
pub(crate) fn with_dot_model(
    params: &Map<String, Value>,
    family: Option<&str>,
    dot_family: Option<&str>,
    launch: &DotLaunch,
) -> Option<Map<String, Value>> {
    let model = launch.model.as_ref()?;
    if !matches!(family, Some("claude" | "codex")) || family != dot_family {
        return None;
    }
    if params
        .get("agentModel")
        .is_some_and(|value| !value.is_null())
    {
        return None;
    }
    let mut params = params.clone();
    params.insert("agentModel".to_string(), Value::String(model.clone()));
    Some(params)
}

/// The variables the `ghostex dots mcp` process reads to learn its session and reach gxserver.
const MCP_SESSION_ENV: [&str; 4] = [
    "GHOSTEX_SESSION_ID",
    "GHOSTEX_GLOBAL_SESSION_REF",
    "GHOSTEX_GXSERVER_BASE_URL",
    "GHOSTEX_GXSERVER_AUTH_TOKEN_FILE",
];

/// Appends the dot's MCP server and instructions to a Claude or Codex command; other agents read
/// `AGENTS.md` from the folder and get nothing extra. `with_instructions` is false when the session
/// already carries a coordinator's role: Claude keeps only the last `--append-system-prompt-file`,
/// so the coordinator's role stays and the dot's AGENTS.md (still in the folder) is not passed.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Claude's `--mcp-config` and `--allowedTools` both take several values, so each is followed by another flag and never by a positional argument. Codex stops an MCP tool call after 60 seconds by default, and an approval can wait up to 30 minutes, so its server gets a longer `tool_timeout_sec`; Codex also starts MCP servers with a short environment allowlist, so the session variables are passed through with `env_vars` or the dot's tools never learn which conversation they serve.
pub(crate) fn with_dot_flags(
    command: &str,
    family: &str,
    launch: &DotLaunch,
    with_instructions: bool,
) -> String {
    let command = command.trim();
    let quote = |path: &Path| crate::agents::quote_shell_arg(&path.to_string_lossy());
    match family {
        "claude" => {
            let mut flags = format!(
                "{command} --mcp-config {} --allowedTools mcp__{OPEN_DOTS_MCP_SERVER_NAME}",
                quote(&mcp_config_file(&launch.folder)),
            );
            if with_instructions {
                flags.push_str(&format!(
                    " --append-system-prompt-file {}",
                    quote(&agents_file(&launch.folder))
                ));
            }
            flags
        }
        "codex" => {
            let cli = serde_json::to_string(&dot_cli_executable()).unwrap_or_default();
            let args = serde_json::to_string(&["dots", "mcp", "--dot", launch.dot_id.as_str()])
                .unwrap_or_default();
            let setting = |key: &str, value: &str| {
                crate::agents::quote_shell_arg(&format!(
                    "mcp_servers.{OPEN_DOTS_MCP_SERVER_NAME}.{key}={value}"
                ))
            };
            let env_vars = serde_json::to_string(&MCP_SESSION_ENV).unwrap_or_default();
            format!(
                "{command} -c {} -c {} -c {} -c {}",
                setting("command", &cli),
                setting("args", &args),
                setting("env_vars", &env_vars),
                setting("tool_timeout_sec", "1900"),
            )
        }
        _ => command.to_string(),
    }
}
