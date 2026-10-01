//! OpenDots' own SQLite file, `<gxserver state>/open-dots.db`, with its own `user_version`
//! migrations, and the CRUD for every record in docs/2026-10-01/opendots/CONTRACT.md §1.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! OpenDots is an extension, so its tables live in their own file instead of taking a number in the shared `state.db` migration list other features append to; removing the extension later is deleting one file.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use super::model::*;
use crate::domain::DomainStateError;
use crate::paths::GxserverPaths;

/// Each entry upgrades the file from `user_version == index` to `index + 1`.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#,
    r#"
CREATE TABLE IF NOT EXISTS dots (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  slug TEXT NOT NULL,
  purpose TEXT NOT NULL DEFAULT '',
  instructions TEXT NOT NULL DEFAULT '',
  look TEXT NOT NULL DEFAULT '{}',
  agent_id TEXT NOT NULL,
  model TEXT,
  paused INTEGER NOT NULL DEFAULT 0,
  project_id TEXT NOT NULL DEFAULT '',
  folder TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS rules (
  id TEXT PRIMARY KEY,
  dot_id TEXT,
  action TEXT NOT NULL,
  decision TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memories (
  id TEXT PRIMARY KEY,
  dot_id TEXT NOT NULL,
  text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS skills (
  id TEXT PRIMARY KEY,
  dot_id TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  body TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS routines (
  id TEXT PRIMARY KEY,
  dot_id TEXT NOT NULL,
  name TEXT NOT NULL,
  instruction TEXT NOT NULL,
  schedule TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  last_run_at INTEGER,
  next_run_at INTEGER,
  session_id TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS triggers (
  id TEXT PRIMARY KEY,
  dot_id TEXT NOT NULL,
  composio_id TEXT NOT NULL,
  slug TEXT NOT NULL,
  toolkit TEXT NOT NULL,
  name TEXT NOT NULL,
  config TEXT NOT NULL DEFAULT '{}',
  instruction TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  last_fired_at INTEGER,
  last_error TEXT,
  session_id TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS passwords (
  id TEXT PRIMARY KEY,
  site TEXT NOT NULL,
  username TEXT NOT NULL,
  secret TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS approvals (
  id TEXT PRIMARY KEY,
  dot_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  session_id TEXT,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  title TEXT NOT NULL,
  detail TEXT,
  tool TEXT,
  options TEXT NOT NULL DEFAULT '[]',
  rule_action TEXT,
  toolkit TEXT,
  url TEXT,
  answer TEXT,
  created_at INTEGER NOT NULL,
  resolved_at INTEGER
);
CREATE INDEX IF NOT EXISTS approvals_status ON approvals (status, created_at);
CREATE TABLE IF NOT EXISTS updates (
  id TEXT PRIMARY KEY,
  dot_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  session_id TEXT,
  title TEXT,
  text TEXT NOT NULL,
  files TEXT NOT NULL DEFAULT '[]',
  read INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS handoffs (
  id TEXT PRIMARY KEY,
  from_dot_id TEXT NOT NULL,
  to_dot_id TEXT NOT NULL,
  message TEXT NOT NULL,
  status TEXT NOT NULL,
  reply TEXT,
  from_project_id TEXT,
  from_session_id TEXT,
  to_project_id TEXT,
  to_session_id TEXT,
  created_at INTEGER NOT NULL,
  replied_at INTEGER
);
CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  lead_id TEXT NOT NULL,
  member_ids TEXT NOT NULL DEFAULT '[]',
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS conversations (
  dot_id TEXT NOT NULL,
  ref TEXT NOT NULL,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (dot_id, ref)
);
"#,
    // A routine keeps why its last run could not start; two dots never share a folder slug or a
    // name (rows from before the indexes that collide are renamed first, so the indexes build).
    r#"
ALTER TABLE routines ADD COLUMN last_error TEXT;
UPDATE dots SET slug = slug || '-' || substr(id, 5, 6)
  WHERE rowid NOT IN (SELECT min(rowid) FROM dots GROUP BY slug);
UPDATE dots SET name = name || ' ' || substr(id, 5, 4)
  WHERE rowid NOT IN (SELECT min(rowid) FROM dots GROUP BY lower(name));
CREATE UNIQUE INDEX IF NOT EXISTS dots_slug ON dots (slug);
CREATE UNIQUE INDEX IF NOT EXISTS dots_name ON dots (lower(name));
"#,
];

pub(crate) fn open_dots_database_file(paths: &GxserverPaths) -> PathBuf {
    paths.root_dir.join("open-dots.db")
}

/// Opens the file (creating it), with WAL and a busy timeout, and runs any pending migration.
pub(crate) fn open_open_dots_database(paths: &GxserverPaths) -> rusqlite::Result<Connection> {
    open_at(&open_dots_database_file(paths))
}

/// The same file, found beside the gxserver `state.db` a connection has open (the launch path
/// only holds that connection).
pub(crate) fn open_beside_state_db(state_db: &Connection) -> Option<Connection> {
    let state_file = state_db.path().filter(|path| !path.is_empty())?;
    let root = Path::new(state_file).parent()?;
    open_at(&root.join("open-dots.db")).ok()
}

fn open_at(file: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let db = Connection::open(file)?;
    db.busy_timeout(std::time::Duration::from_millis(5_000))?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&db)?;
    Ok(db)
}

fn migrate(db: &Connection) -> rusqlite::Result<()> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

pub(crate) fn get_setting(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

/// `None` removes the key.
pub(crate) fn set_setting(db: &Connection, key: &str, value: Option<&str>) -> rusqlite::Result<()> {
    match value {
        Some(value) => db.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        ),
        None => db.execute("DELETE FROM settings WHERE key = ?1", params![key]),
    }
    .map(drop)
}

pub(crate) type StoreResult<T> = Result<T, DomainStateError>;

pub(crate) fn sql_error(error: rusqlite::Error) -> DomainStateError {
    DomainStateError {
        code: "internalError",
        message: format!("OpenDots storage error: {error}"),
    }
}

/// Opens the OpenDots file as a domain result.
pub(crate) fn open(paths: &GxserverPaths) -> StoreResult<Connection> {
    open_open_dots_database(paths).map_err(sql_error)
}

fn json_list(text: Option<String>) -> Vec<String> {
    text.and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

fn json_value(text: Option<String>) -> Value {
    text.and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| Value::Object(Default::default()))
}

// ------------------------------------------------------------------ dots

const DOT_COLUMNS: &str = "id, name, slug, purpose, instructions, look, agent_id, model, paused, project_id, folder, created_at, updated_at";

fn dot_from_row(row: &Row<'_>) -> rusqlite::Result<Dot> {
    let look: Option<String> = row.get(5)?;
    let paused = row.get::<_, i64>(8)? != 0;
    Ok(Dot {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        purpose: row.get(3)?,
        instructions: row.get(4)?,
        look: normalize_look(Some(&json_value(look))),
        agent_id: row.get(6)?,
        model: row.get(7)?,
        paused,
        // The rest of the status is derived from the dot's sessions (snapshot.rs).
        status: if paused { "paused" } else { "idle" }.to_string(),
        activity: None,
        project_id: row.get(9)?,
        folder: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub(crate) fn list_dots(db: &Connection) -> StoreResult<Vec<Dot>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {DOT_COLUMNS} FROM dots ORDER BY created_at"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], dot_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn get_dot(db: &Connection, dot_id: &str) -> StoreResult<Option<Dot>> {
    db.query_row(
        &format!("SELECT {DOT_COLUMNS} FROM dots WHERE id = ?1"),
        params![dot_id],
        dot_from_row,
    )
    .optional()
    .map_err(sql_error)
}

pub(crate) fn require_dot(db: &Connection, dot_id: &str) -> StoreResult<Dot> {
    get_dot(db, dot_id)?.ok_or_else(|| DomainStateError::not_found("That dot no longer exists."))
}

/// By id, or by name (case-insensitive).
pub(crate) fn find_dot(db: &Connection, reference: &str) -> StoreResult<Option<Dot>> {
    let reference = reference.trim();
    if let Some(dot) = get_dot(db, reference)? {
        return Ok(Some(dot));
    }
    Ok(list_dots(db)?
        .into_iter()
        .find(|dot| dot.name.trim().eq_ignore_ascii_case(reference)))
}

/// A write that broke the unique slug or name index (two creates or renames racing past the
/// name check) as the refusal the name check gives.
fn dot_write_error(error: rusqlite::Error, name: &str) -> DomainStateError {
    match &error {
        rusqlite::Error::SqliteFailure(failure, message)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            if message
                .as_deref()
                .is_some_and(|message| message.contains("dots_name"))
            {
                DomainStateError::bad_request(format!(
                    "You already have a dot named {name}. Dots find each other by name, so pick another."
                ))
            } else {
                DomainStateError::bad_request("Pick a different name for the dot.")
            }
        }
        _ => sql_error(error),
    }
}

pub(crate) fn insert_dot(db: &Connection, dot: &Dot) -> StoreResult<()> {
    db.execute(
        &format!("INSERT INTO dots ({DOT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
        params![
            dot.id,
            dot.name,
            dot.slug,
            dot.purpose,
            dot.instructions,
            dot.look.to_string(),
            dot.agent_id,
            dot.model,
            dot.paused as i64,
            dot.project_id,
            dot.folder,
            dot.created_at,
            dot.updated_at,
        ],
    )
    .map_err(|error| dot_write_error(error, &dot.name))?;
    Ok(())
}

/// Writes every editable column of `dot` back (name, purpose, instructions, look, agent, model,
/// paused, project) and stamps `updated_at`.
pub(crate) fn save_dot(db: &Connection, dot: &Dot) -> StoreResult<()> {
    db.execute(
        "UPDATE dots SET name = ?2, purpose = ?3, instructions = ?4, look = ?5, agent_id = ?6, model = ?7, paused = ?8, project_id = ?9, updated_at = ?10 WHERE id = ?1",
        params![
            dot.id,
            dot.name,
            dot.purpose,
            dot.instructions,
            dot.look.to_string(),
            dot.agent_id,
            dot.model,
            dot.paused as i64,
            dot.project_id,
            now_ms(),
        ],
    )
    .map_err(|error| dot_write_error(error, &dot.name))?;
    Ok(())
}

/// Removes the dot and every record that belongs only to it.
pub(crate) fn delete_dot_records(db: &Connection, dot_id: &str) -> StoreResult<()> {
    let tx = db.unchecked_transaction().map_err(sql_error)?;
    for table in [
        "rules",
        "memories",
        "skills",
        "routines",
        "triggers",
        "approvals",
        "updates",
        "conversations",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE dot_id = ?1"),
            params![dot_id],
        )
        .map_err(sql_error)?;
    }
    tx.execute(
        "DELETE FROM handoffs WHERE from_dot_id = ?1 OR to_dot_id = ?1",
        params![dot_id],
    )
    .map_err(sql_error)?;
    tx.execute("DELETE FROM teams WHERE lead_id = ?1", params![dot_id])
        .map_err(sql_error)?;
    tx.execute("DELETE FROM dots WHERE id = ?1", params![dot_id])
        .map_err(sql_error)?;
    tx.commit().map_err(sql_error)?;
    // A team the dot was only a member of keeps its other members.
    for mut team in list_teams(db)? {
        if team.member_ids.iter().any(|member| member == dot_id) {
            team.member_ids.retain(|member| member != dot_id);
            db.execute(
                "UPDATE teams SET member_ids = ?2 WHERE id = ?1",
                params![
                    team.id,
                    serde_json::to_string(&team.member_ids).unwrap_or_default()
                ],
            )
            .map_err(sql_error)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ rules

fn rule_from_row(row: &Row<'_>) -> rusqlite::Result<Rule> {
    Ok(Rule {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        action: row.get(2)?,
        decision: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(crate) fn list_rules(db: &Connection) -> StoreResult<Vec<Rule>> {
    let mut statement = db
        .prepare("SELECT id, dot_id, action, decision, created_at FROM rules ORDER BY created_at")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], rule_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

/// The rules one dot follows: its own and the ones for every dot.
pub(crate) fn rules_for(db: &Connection, dot_id: &str) -> StoreResult<Vec<Rule>> {
    Ok(list_rules(db)?
        .into_iter()
        .filter(|rule| rule.dot_id.as_deref().is_none_or(|owner| owner == dot_id))
        .collect())
}

pub(crate) fn get_rule(db: &Connection, rule_id: &str) -> StoreResult<Option<Rule>> {
    db.query_row(
        "SELECT id, dot_id, action, decision, created_at FROM rules WHERE id = ?1",
        params![rule_id],
        rule_from_row,
    )
    .optional()
    .map_err(sql_error)
}

pub(crate) fn add_rule(
    db: &Connection,
    dot_id: Option<&str>,
    action: &str,
    decision: &str,
) -> StoreResult<Rule> {
    let rule = Rule {
        id: new_id("rule"),
        dot_id: dot_id.map(str::to_string),
        action: action.to_string(),
        decision: decision.to_string(),
        created_at: now_ms(),
    };
    db.execute(
        "INSERT INTO rules (id, dot_id, action, decision, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            rule.id,
            rule.dot_id,
            rule.action,
            rule.decision,
            rule.created_at
        ],
    )
    .map_err(sql_error)?;
    Ok(rule)
}

/// Always allow on a card: the dot's own rules for this action (any case) become one allow rule,
/// instead of an allow rule beside an "ask first" one that would still win.
pub(crate) fn allow_rule_for_dot(db: &Connection, dot_id: &str, action: &str) -> StoreResult<Rule> {
    let key = action.trim().to_lowercase();
    let same: Vec<Rule> = list_rules(db)?
        .into_iter()
        .filter(|rule| {
            rule.dot_id.as_deref() == Some(dot_id) && rule.action.trim().to_lowercase() == key
        })
        .collect();
    let Some((first, rest)) = same.split_first() else {
        return add_rule(db, Some(dot_id), action, "allow");
    };
    db.execute(
        "UPDATE rules SET decision = 'allow' WHERE id = ?1",
        params![first.id],
    )
    .map_err(sql_error)?;
    for duplicate in rest {
        delete_rule(db, &duplicate.id)?;
    }
    Ok(Rule {
        decision: "allow".to_string(),
        ..first.clone()
    })
}

pub(crate) fn delete_rule(db: &Connection, rule_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM rules WHERE id = ?1", params![rule_id])
        .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ memories

fn memory_from_row(row: &Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        text: row.get(2)?,
        created_at: row.get(3)?,
    })
}

pub(crate) fn list_memories(db: &Connection, dot_id: Option<&str>) -> StoreResult<Vec<Memory>> {
    let mut statement = db
        .prepare(
            "SELECT id, dot_id, text, created_at FROM memories WHERE ?1 IS NULL OR dot_id = ?1 ORDER BY created_at",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![dot_id], memory_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn get_memory(db: &Connection, memory_id: &str) -> StoreResult<Option<Memory>> {
    db.query_row(
        "SELECT id, dot_id, text, created_at FROM memories WHERE id = ?1",
        params![memory_id],
        memory_from_row,
    )
    .optional()
    .map_err(sql_error)
}

pub(crate) fn add_memory(db: &Connection, dot_id: &str, text: &str) -> StoreResult<Memory> {
    let memory = Memory {
        id: new_id("mem"),
        dot_id: dot_id.to_string(),
        text: text.to_string(),
        created_at: now_ms(),
    };
    db.execute(
        "INSERT INTO memories (id, dot_id, text, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![memory.id, memory.dot_id, memory.text, memory.created_at],
    )
    .map_err(sql_error)?;
    Ok(memory)
}

pub(crate) fn delete_memory(db: &Connection, memory_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM memories WHERE id = ?1", params![memory_id])
        .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ skills

fn skill_from_row(row: &Row<'_>) -> rusqlite::Result<Skill> {
    Ok(Skill {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        body: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

const SKILL_COLUMNS: &str = "id, dot_id, name, description, body, created_at, updated_at";

pub(crate) fn list_skills(db: &Connection, dot_id: Option<&str>) -> StoreResult<Vec<Skill>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {SKILL_COLUMNS} FROM skills WHERE ?1 IS NULL OR dot_id = ?1 ORDER BY created_at"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![dot_id], skill_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn get_skill(db: &Connection, skill_id: &str) -> StoreResult<Option<Skill>> {
    db.query_row(
        &format!("SELECT {SKILL_COLUMNS} FROM skills WHERE id = ?1"),
        params![skill_id],
        skill_from_row,
    )
    .optional()
    .map_err(sql_error)
}

/// Open Dot's `upsertSkill`: a skill with the same name (case-insensitive) is updated.
pub(crate) fn upsert_skill(
    db: &Connection,
    dot_id: &str,
    name: &str,
    description: &str,
    body: &str,
) -> StoreResult<Skill> {
    let existing = list_skills(db, Some(dot_id))?
        .into_iter()
        .find(|skill| skill.name.eq_ignore_ascii_case(name.trim()));
    let now = now_ms();
    let id = match existing {
        Some(skill) => {
            db.execute(
                "UPDATE skills SET name = ?2, description = ?3, body = ?4, updated_at = ?5 WHERE id = ?1",
                params![skill.id, name.trim(), description, body, now],
            )
            .map_err(sql_error)?;
            skill.id
        }
        None => {
            let id = new_id("skill");
            db.execute(
                &format!(
                    "INSERT INTO skills ({SKILL_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)"
                ),
                params![id, dot_id, name.trim(), description, body, now],
            )
            .map_err(sql_error)?;
            id
        }
    };
    get_skill(db, &id)?.ok_or_else(|| DomainStateError::not_found("The skill was not saved."))
}

pub(crate) fn delete_skill(db: &Connection, skill_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM skills WHERE id = ?1", params![skill_id])
        .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ routines

const ROUTINE_COLUMNS: &str = "id, dot_id, name, instruction, schedule, enabled, last_run_at, next_run_at, session_id, created_at, last_error";

fn routine_from_row(row: &Row<'_>) -> rusqlite::Result<Routine> {
    Ok(Routine {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        name: row.get(2)?,
        instruction: row.get(3)?,
        schedule: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        last_run_at: row.get(6)?,
        next_run_at: row.get(7)?,
        session_id: row.get(8)?,
        created_at: row.get(9)?,
        last_error: row.get(10)?,
    })
}

pub(crate) fn list_routines(db: &Connection, dot_id: Option<&str>) -> StoreResult<Vec<Routine>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {ROUTINE_COLUMNS} FROM routines WHERE ?1 IS NULL OR dot_id = ?1 ORDER BY created_at"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![dot_id], routine_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn get_routine(db: &Connection, routine_id: &str) -> StoreResult<Option<Routine>> {
    db.query_row(
        &format!("SELECT {ROUTINE_COLUMNS} FROM routines WHERE id = ?1"),
        params![routine_id],
        routine_from_row,
    )
    .optional()
    .map_err(sql_error)
}

pub(crate) fn insert_routine(db: &Connection, routine: &Routine) -> StoreResult<()> {
    db.execute(
        &format!("INSERT INTO routines ({ROUTINE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
        params![
            routine.id,
            routine.dot_id,
            routine.name,
            routine.instruction,
            routine.schedule,
            routine.enabled as i64,
            routine.last_run_at,
            routine.next_run_at,
            routine.session_id,
            routine.created_at,
            routine.last_error,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

/// The user's edit (`updateRoutine`): what they can change and the next run it implies. What a run
/// records is written by `record_routine_run`, so an edit never undoes a run and a run never undoes
/// an edit.
pub(crate) fn save_routine(db: &Connection, routine: &Routine) -> StoreResult<()> {
    db.execute(
        "UPDATE routines SET name = ?2, instruction = ?3, schedule = ?4, enabled = ?5, next_run_at = ?6 WHERE id = ?1",
        params![
            routine.id,
            routine.name,
            routine.instruction,
            routine.schedule,
            routine.enabled as i64,
            routine.next_run_at,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

/// Moves a routine's next run from `expected` to `next` only while nobody changed it since it was
/// read (compare-and-set). `require_enabled` also refuses a routine the user turned off. Returns
/// whether this call moved it, so a due run is claimed exactly once.
pub(crate) fn move_routine_next_run(
    db: &Connection,
    routine_id: &str,
    expected: Option<i64>,
    next: Option<i64>,
    require_enabled: bool,
) -> StoreResult<bool> {
    let changed = db
        .execute(
            "UPDATE routines SET next_run_at = ?3 WHERE id = ?1 AND next_run_at IS ?2 AND (?4 = 0 OR enabled = 1)",
            params![routine_id, expected, next, require_enabled as i64],
        )
        .map_err(sql_error)?;
    Ok(changed == 1)
}

/// What a run leaves on the routine: when it ran, its conversation, and why it could not start.
pub(crate) fn record_routine_run(
    db: &Connection,
    routine_id: &str,
    ran_at: i64,
    session_id: Option<&str>,
    error: Option<&str>,
) -> StoreResult<()> {
    db.execute(
        "UPDATE routines SET last_run_at = ?2, session_id = COALESCE(?3, session_id), last_error = ?4 WHERE id = ?1",
        params![routine_id, ran_at, session_id, error],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn delete_routine(db: &Connection, routine_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM routines WHERE id = ?1", params![routine_id])
        .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ triggers

const TRIGGER_COLUMNS: &str = "id, dot_id, composio_id, slug, toolkit, name, config, instruction, enabled, last_fired_at, last_error, session_id, created_at";

fn trigger_from_row(row: &Row<'_>) -> rusqlite::Result<Trigger> {
    Ok(Trigger {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        composio_id: row.get(2)?,
        slug: row.get(3)?,
        toolkit: row.get(4)?,
        name: row.get(5)?,
        config: json_value(row.get(6)?),
        instruction: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        last_fired_at: row.get(9)?,
        last_error: row.get(10)?,
        session_id: row.get(11)?,
        created_at: row.get(12)?,
    })
}

pub(crate) fn list_triggers(db: &Connection, dot_id: Option<&str>) -> StoreResult<Vec<Trigger>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {TRIGGER_COLUMNS} FROM triggers WHERE ?1 IS NULL OR dot_id = ?1 ORDER BY created_at"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![dot_id], trigger_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn get_trigger(db: &Connection, trigger_id: &str) -> StoreResult<Option<Trigger>> {
    db.query_row(
        &format!("SELECT {TRIGGER_COLUMNS} FROM triggers WHERE id = ?1"),
        params![trigger_id],
        trigger_from_row,
    )
    .optional()
    .map_err(sql_error)
}

/// Every local trigger on one Composio trigger: Composio hands back the same trigger when two dots
/// add the same event with the same settings, so several dots can share one.
pub(crate) fn triggers_by_composio_id(
    db: &Connection,
    composio_id: &str,
) -> StoreResult<Vec<Trigger>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {TRIGGER_COLUMNS} FROM triggers WHERE composio_id = ?1 ORDER BY created_at"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![composio_id], trigger_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

/// What an event run leaves on a trigger (when it fired, its conversation, why it could not run),
/// without writing back fields the user may have changed meanwhile (its switch, its instruction).
pub(crate) fn record_trigger_fire(
    db: &Connection,
    trigger_id: &str,
    fired_at: i64,
    session_id: Option<&str>,
    error: Option<&str>,
) -> StoreResult<()> {
    db.execute(
        "UPDATE triggers SET last_fired_at = ?2, session_id = COALESCE(?3, session_id), last_error = ?4 WHERE id = ?1",
        params![trigger_id, fired_at, session_id, error],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn insert_trigger(db: &Connection, trigger: &Trigger) -> StoreResult<()> {
    db.execute(
        &format!("INSERT INTO triggers ({TRIGGER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
        params![
            trigger.id,
            trigger.dot_id,
            trigger.composio_id,
            trigger.slug,
            trigger.toolkit,
            trigger.name,
            trigger.config.to_string(),
            trigger.instruction,
            trigger.enabled as i64,
            trigger.last_fired_at,
            trigger.last_error,
            trigger.session_id,
            trigger.created_at,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn save_trigger(db: &Connection, trigger: &Trigger) -> StoreResult<()> {
    db.execute(
        "UPDATE triggers SET enabled = ?2, last_fired_at = ?3, last_error = ?4, session_id = ?5, instruction = ?6 WHERE id = ?1",
        params![
            trigger.id,
            trigger.enabled as i64,
            trigger.last_fired_at,
            trigger.last_error,
            trigger.session_id,
            trigger.instruction,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn delete_trigger(db: &Connection, trigger_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM triggers WHERE id = ?1", params![trigger_id])
        .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ passwords

fn password_from_row(row: &Row<'_>) -> rusqlite::Result<Password> {
    Ok(Password {
        id: row.get(0)?,
        site: row.get(1)?,
        username: row.get(2)?,
        created_at: row.get(3)?,
    })
}

pub(crate) fn list_passwords(db: &Connection) -> StoreResult<Vec<Password>> {
    let mut statement = db
        .prepare("SELECT id, site, username, created_at FROM passwords ORDER BY site")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], password_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn insert_password(
    db: &Connection,
    site: &str,
    username: &str,
    sealed: &str,
) -> StoreResult<Password> {
    let password = Password {
        id: new_id("pw"),
        site: site.to_string(),
        username: username.to_string(),
        created_at: now_ms(),
    };
    db.execute(
        "INSERT INTO passwords (id, site, username, secret, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            password.id,
            password.site,
            password.username,
            sealed,
            password.created_at
        ],
    )
    .map_err(sql_error)?;
    Ok(password)
}

pub(crate) fn delete_password(db: &Connection, password_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM passwords WHERE id = ?1", params![password_id])
        .map_err(sql_error)?;
    Ok(())
}

/// A site's host without `www.`, lowercased (Open Dot's `hostOf`).
pub(crate) fn host_of(site: &str) -> String {
    let site = site.trim();
    let with_scheme = if site.contains("://") {
        site.to_string()
    } else {
        format!("https://{site}")
    };
    url::Url::parse(&with_scheme)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
        .unwrap_or_else(|| site.to_ascii_lowercase())
}

/// A saved login, secret still sealed.
pub(crate) struct SavedLogin {
    pub(crate) site: String,
    /// `host_of(site)`: the host the page must be on (or under) for this login to be typed.
    pub(crate) host: String,
    pub(crate) username: String,
    pub(crate) sealed: String,
}

/// The saved login for the page the dot's browser shows, chosen by the page's own host: a login
/// saved for that host, else the one saved for the closest parent domain (`accounts.google.com`
/// uses a login saved for `google.com`). Never the other way round: a login saved for
/// `accounts.google.com` is not typed into `google.com`, and a bare suffix like `com` matches
/// nothing.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The login used to be chosen by the site the model named, and matched in both directions, so a page on another site could ask for any saved password ("site: github.com" on evil.com, or "site: com" for every .com login). The page's real host decides now, and the browser re-checks it inside the page before typing.
pub(crate) fn saved_login_for_host(
    db: &Connection,
    page_host: &str,
) -> StoreResult<Option<SavedLogin>> {
    let mut statement = db
        .prepare("SELECT site, username, secret FROM passwords ORDER BY created_at")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    let page_host = host_of(page_host);
    if page_host.is_empty() {
        return Ok(None);
    }
    let best = rows
        .into_iter()
        .map(|(site, username, sealed)| SavedLogin {
            host: host_of(&site),
            site,
            username,
            sealed,
        })
        .filter(|login| {
            login.host == page_host
                || (login.host.contains('.') && page_host.ends_with(&format!(".{}", login.host)))
        })
        // The exact host first, then the longest (closest) parent domain.
        .max_by_key(|login| (login.host == page_host, login.host.len()));
    Ok(best)
}

// ------------------------------------------------------------------ approvals

const APPROVAL_COLUMNS: &str = "id, dot_id, project_id, session_id, kind, status, title, detail, tool, options, rule_action, toolkit, url, answer, created_at, resolved_at";

fn approval_from_row(row: &Row<'_>) -> rusqlite::Result<Approval> {
    Ok(Approval {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        project_id: row.get(2)?,
        session_id: row.get(3)?,
        kind: row.get(4)?,
        status: row.get(5)?,
        title: row.get(6)?,
        detail: row.get(7)?,
        tool: row.get(8)?,
        options: json_list(row.get(9)?),
        rule_action: row.get(10)?,
        toolkit: row.get(11)?,
        url: row.get(12)?,
        answer: row.get(13)?,
        created_at: row.get(14)?,
        resolved_at: row.get(15)?,
    })
}

fn query_approvals(
    db: &Connection,
    where_sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> StoreResult<Vec<Approval>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals {where_sql}"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map(args, approval_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

/// Every pending approval, then the 50 latest resolved ones.
pub(crate) fn list_approvals_for_snapshot(db: &Connection) -> StoreResult<Vec<Approval>> {
    let mut rows = query_approvals(db, "WHERE status = 'pending' ORDER BY created_at", &[])?;
    rows.extend(query_approvals(
        db,
        "WHERE status <> 'pending' ORDER BY created_at DESC LIMIT 50",
        &[],
    )?);
    Ok(rows)
}

pub(crate) fn pending_approvals(db: &Connection) -> StoreResult<Vec<Approval>> {
    query_approvals(db, "WHERE status = 'pending' ORDER BY created_at", &[])
}

pub(crate) fn get_approval(db: &Connection, approval_id: &str) -> StoreResult<Option<Approval>> {
    Ok(query_approvals(db, "WHERE id = ?1", &[&approval_id])?
        .into_iter()
        .next())
}

pub(crate) fn insert_approval(db: &Connection, approval: &Approval) -> StoreResult<()> {
    db.execute(
        &format!("INSERT INTO approvals ({APPROVAL_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"),
        params![
            approval.id,
            approval.dot_id,
            approval.project_id,
            approval.session_id,
            approval.kind,
            approval.status,
            approval.title,
            approval.detail,
            approval.tool,
            serde_json::to_string(&approval.options).unwrap_or_else(|_| "[]".to_string()),
            approval.rule_action,
            approval.toolkit,
            approval.url,
            approval.answer,
            approval.created_at,
            approval.resolved_at,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

/// Moves a pending approval to its final status. Returns the stored row when it was still pending.
pub(crate) fn settle_approval(
    db: &Connection,
    approval_id: &str,
    status: &str,
    answer: Option<&str>,
) -> StoreResult<Option<Approval>> {
    let changed = db
        .execute(
            "UPDATE approvals SET status = ?2, answer = ?3, resolved_at = ?4 WHERE id = ?1 AND status = 'pending'",
            params![approval_id, status, answer, now_ms()],
        )
        .map_err(sql_error)?;
    if changed == 0 {
        return Ok(None);
    }
    get_approval(db, approval_id)
}

// ------------------------------------------------------------------ updates

const UPDATE_COLUMNS: &str =
    "id, dot_id, project_id, session_id, title, text, files, read, created_at";

fn update_from_row(row: &Row<'_>) -> rusqlite::Result<Update> {
    let files: Option<String> = row.get(6)?;
    Ok(Update {
        id: row.get(0)?,
        dot_id: row.get(1)?,
        project_id: row.get(2)?,
        session_id: row.get(3)?,
        title: row.get(4)?,
        text: row.get(5)?,
        files: files
            .and_then(|files| serde_json::from_str(&files).ok())
            .unwrap_or_default(),
        read: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
    })
}

pub(crate) fn list_updates(db: &Connection, limit: usize) -> StoreResult<Vec<Update>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {UPDATE_COLUMNS} FROM updates ORDER BY created_at DESC LIMIT ?1"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![limit as i64], update_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn insert_update(db: &Connection, update: &Update) -> StoreResult<()> {
    db.execute(
        &format!(
            "INSERT INTO updates ({UPDATE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
        ),
        params![
            update.id,
            update.dot_id,
            update.project_id,
            update.session_id,
            update.title,
            update.text,
            serde_json::to_string(&update.files).unwrap_or_else(|_| "[]".to_string()),
            update.read as i64,
            update.created_at,
        ],
    )
    .map_err(sql_error)?;
    // Keep the feed bounded: the snapshot shows the 100 latest.
    db.execute(
        "DELETE FROM updates WHERE id NOT IN (SELECT id FROM updates ORDER BY created_at DESC LIMIT 500)",
        [],
    )
    .map_err(sql_error)?;
    Ok(())
}

/// Marks updates read: the listed ids, else every update of one dot, else all.
pub(crate) fn mark_updates_read(
    db: &Connection,
    dot_id: Option<&str>,
    update_ids: Option<&[String]>,
) -> StoreResult<()> {
    match (update_ids, dot_id) {
        (Some(ids), _) => {
            for id in ids {
                db.execute("UPDATE updates SET read = 1 WHERE id = ?1", params![id])
                    .map_err(sql_error)?;
            }
        }
        (None, Some(dot_id)) => {
            db.execute(
                "UPDATE updates SET read = 1 WHERE dot_id = ?1",
                params![dot_id],
            )
            .map_err(sql_error)?;
        }
        (None, None) => {
            db.execute("UPDATE updates SET read = 1", [])
                .map_err(sql_error)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ handoffs

const HANDOFF_COLUMNS: &str = "id, from_dot_id, to_dot_id, message, status, reply, created_at, replied_at, from_project_id, from_session_id, to_project_id, to_session_id";

fn handoff_from_row(row: &Row<'_>) -> rusqlite::Result<Handoff> {
    Ok(Handoff {
        id: row.get(0)?,
        from_dot_id: row.get(1)?,
        to_dot_id: row.get(2)?,
        message: row.get(3)?,
        status: row.get(4)?,
        reply: row.get(5)?,
        created_at: row.get(6)?,
        replied_at: row.get(7)?,
        from_project_id: row.get(8)?,
        from_session_id: row.get(9)?,
        to_project_id: row.get(10)?,
        to_session_id: row.get(11)?,
    })
}

fn query_handoffs(db: &Connection, where_sql: &str) -> StoreResult<Vec<Handoff>> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {HANDOFF_COLUMNS} FROM handoffs {where_sql}"
        ))
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], handoff_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn list_handoffs(db: &Connection, limit: usize) -> StoreResult<Vec<Handoff>> {
    query_handoffs(db, &format!("ORDER BY created_at DESC LIMIT {limit}"))
}

/// Hand-offs still waiting on the other dot's reply.
pub(crate) fn open_handoffs(db: &Connection) -> StoreResult<Vec<Handoff>> {
    query_handoffs(
        db,
        "WHERE status IN ('sent', 'working') ORDER BY created_at",
    )
}

pub(crate) fn insert_handoff(db: &Connection, handoff: &Handoff) -> StoreResult<()> {
    db.execute(
        &format!("INSERT INTO handoffs ({HANDOFF_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"),
        params![
            handoff.id,
            handoff.from_dot_id,
            handoff.to_dot_id,
            handoff.message,
            handoff.status,
            handoff.reply,
            handoff.created_at,
            handoff.replied_at,
            handoff.from_project_id,
            handoff.from_session_id,
            handoff.to_project_id,
            handoff.to_session_id,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn save_handoff(db: &Connection, handoff: &Handoff) -> StoreResult<()> {
    db.execute(
        "UPDATE handoffs SET status = ?2, reply = ?3, replied_at = ?4, to_project_id = ?5, to_session_id = ?6 WHERE id = ?1",
        params![
            handoff.id,
            handoff.status,
            handoff.reply,
            handoff.replied_at,
            handoff.to_project_id,
            handoff.to_session_id,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ teams

fn team_from_row(row: &Row<'_>) -> rusqlite::Result<Team> {
    Ok(Team {
        id: row.get(0)?,
        name: row.get(1)?,
        lead_id: row.get(2)?,
        member_ids: json_list(row.get(3)?),
        project_id: row.get(4)?,
        session_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

pub(crate) fn list_teams(db: &Connection) -> StoreResult<Vec<Team>> {
    let mut statement = db
        .prepare("SELECT id, name, lead_id, member_ids, project_id, session_id, created_at FROM teams ORDER BY created_at")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], team_from_row)
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(rows)
}

pub(crate) fn insert_team(db: &Connection, team: &Team) -> StoreResult<()> {
    db.execute(
        "INSERT INTO teams (id, name, lead_id, member_ids, project_id, session_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            team.id,
            team.name,
            team.lead_id,
            serde_json::to_string(&team.member_ids).unwrap_or_else(|_| "[]".to_string()),
            team.project_id,
            team.session_id,
            team.created_at,
        ],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn delete_team(db: &Connection, team_id: &str) -> StoreResult<()> {
    db.execute("DELETE FROM teams WHERE id = ?1", params![team_id])
        .map_err(sql_error)?;
    Ok(())
}

// ------------------------------------------------------------------ conversations

/// The conversation a routine, trigger, hand-off sender or team keeps reusing
/// (`routine:<id>`, `trigger:<id>`, `from:<dotId>`, `team:<id>`).
pub(crate) fn conversation(
    db: &Connection,
    dot_id: &str,
    reference: &str,
) -> StoreResult<Option<(String, String)>> {
    db.query_row(
        "SELECT project_id, session_id FROM conversations WHERE dot_id = ?1 AND ref = ?2",
        params![dot_id, reference],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(sql_error)
}

pub(crate) fn set_conversation(
    db: &Connection,
    dot_id: &str,
    reference: &str,
    project_id: &str,
    session_id: &str,
) -> StoreResult<()> {
    db.execute(
        "INSERT INTO conversations (dot_id, ref, project_id, session_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(dot_id, ref) DO UPDATE SET project_id = excluded.project_id, session_id = excluded.session_id",
        params![dot_id, reference, project_id, session_id, now_ms()],
    )
    .map_err(sql_error)?;
    Ok(())
}
