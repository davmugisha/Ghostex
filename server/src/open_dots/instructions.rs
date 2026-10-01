//! The files in a dot's folder that make an ordinary agent session that dot: `AGENTS.md` (Open
//! Dot's system prompt, `agent/prompt.ts`, rebuilt from the dot's records), one Claude skill per
//! saved skill, and `.ghostex/mcp.json` for its tools. Rewritten on every change that reaches them.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! Open Dot rebuilt the system prompt for every model turn; an agent CLI reads its instructions once at launch, so the file says that memory, skills and routines are as of the start of the conversation and the tools return what changed since.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use rusqlite::Connection;

use super::launch::{agents_file, mcp_config, mcp_config_file};
use super::model::{Dot, Memory, Routine, Rule, Skill};
use super::{composio, cron, store};
use crate::paths::GxserverPaths;

/// Marks the Claude skill folders OpenDots writes, so a removed skill's folder can be cleaned up
/// without touching skills the user added by hand.
const SKILL_MARKER: &str = "<!-- Written by OpenDots";

fn decision_text(decision: &str) -> &'static str {
    match decision {
        "allow" => "do it without asking",
        "never" => "never do it",
        _ => "ask first",
    }
}

/// A skill's folder name: lowercase letters, digits and hyphens, as Claude skill names are.
pub(crate) fn skill_slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug
        .trim_end_matches('-')
        .chars()
        .take(64)
        .collect::<String>();
    if slug.is_empty() {
        "skill".to_string()
    } else {
        slug
    }
}

struct Context {
    dot: Dot,
    rules: Vec<Rule>,
    memories: Vec<Memory>,
    skills: Vec<Skill>,
    routines: Vec<Routine>,
    others: Vec<Dot>,
    sites: Vec<String>,
    apps: Option<Vec<String>>,
}

fn agents_markdown(context: &Context) -> String {
    let Context {
        dot,
        rules,
        memories,
        skills,
        routines,
        others,
        sites,
        apps,
    } = context;
    let mut out = String::new();
    let _ = writeln!(out, "# You are {}", dot.name);
    let _ = writeln!(
        out,
        "\nYou are {}, a \"dot\": a personal AI agent that works on its own on behalf of your user, inside Ghostex on their computer.",
        dot.name
    );
    if !dot.purpose.trim().is_empty() {
        let _ = writeln!(out, "\nYour job: {}", dot.purpose.trim());
    }
    if !dot.instructions.trim().is_empty() {
        let _ = writeln!(
            out,
            "\n## How the user wants you to work\n\n{}",
            dot.instructions.trim()
        );
    }
    let _ = writeln!(
        out,
        "\n## Your workspace\n\nYour folder is `{}`; keep your files there. You also have your own browser, which keeps its logins between conversations: operate websites with open_url, read_page, page_snapshot, click, type_text, press_key, scroll, screenshot and go_back. Use quick web search for facts and the browser when you need to operate a site.",
        dot.folder
    );
    out.push_str(
        "\n## Working style\n\n\
- Work autonomously until the task is done. Don't narrate every step; the user sees your activity.\n\
- Finish with a concise result: lead with the answer, then key details and sources or links.\n\
- For long work (research, multi-step tasks) you may post a brief progress note with send_update; deliver finished work with a clear title such as \"Your research is ready\".\n\
- When you make a file the user should have (a report, a spreadsheet, an image), write it in your folder and hand it over with share_file.\n\
- If you're blocked on something only the user can do (a login without a saved password, a captcha, a two-factor code), ask with ask_user and tell them they can take over your browser from your Computer tab.\n",
    );
    out.push_str(
        "\n## When to act and when to ask\n\n\
Take reversible, low-stakes actions yourself. Call request_approval BEFORE anything irreversible, public, costly, or that speaks for the user: sending emails, messages or posts, purchases or payments, deleting data, submitting forms, accepting invites, changing account or security settings. Describe exactly what will happen.\n",
    );
    if rules.is_empty() {
        out.push_str("The user has no custom rules yet.\n");
    } else {
        out.push_str("The user's rules (these override the defaults above):\n");
        for rule in rules {
            let _ = writeln!(
                out,
                "- When you want to {}: {}.",
                rule.action.trim(),
                decision_text(&rule.decision)
            );
        }
    }
    // CDXC:OpenDots 2026-10-01 WHY:
    // Seen with Claude Code: told a rule meant "ask first (request_approval)", the dot called request_approval itself and then the gated tool asked again, so the user answered two cards for one action (Open Dot's prompt had the same double ask).
    out.push_str("Your ghostex-dots tools (browser, sign_in, apps, routines) check these rules on their own and show the user an approval card when a rule says to ask, so call them directly and never call request_approval for an action one of them is about to take. Call request_approval only for something you do another way (a shell command, a file, a script) that a rule or the defaults above say needs the user's approval. When a tool reports that the user denied something, don't retry it or work around it.\n");

    out.push_str("\n## The user's apps (Composio)\n\n");
    match apps {
        Some(apps) => {
            let connected = if apps.is_empty() {
                "none yet".to_string()
            } else {
                apps.join(", ")
            };
            let _ = writeln!(
                out,
                "Connected: {connected}. For email, calendar, chat, docs, code, CRM and other apps, use COMPOSIO_SEARCH_TOOLS to find the right tools, then COMPOSIO_MULTI_EXECUTE_TOOL to run them, instead of the browser. Reading runs automatically; anything that sends, posts, creates, edits or deletes asks the user first on its own, so don't also call request_approval for it. If an app isn't connected, call app_connect."
            );
        }
        None => out.push_str("The user hasn't connected their apps yet. If a task needs their apps (email, calendar, chat, docs), tell them they can connect them in OpenDots Settings under Apps, or use your browser.\n"),
    }

    out.push_str("\n## Passwords\n\n");
    if sites.is_empty() {
        out.push_str("No saved logins yet.");
    } else {
        let _ = write!(
            out,
            "Saved logins exist for: {}. On the site's sign-in page, call sign_in: the password is typed for you and you never see it.",
            sites.join(", ")
        );
    }
    out.push_str(" Never ask the user to paste a password into chat; ask them to add it under Saved logins in OpenDots Settings instead.\n");

    out.push_str("\n## Memory\n\n");
    if memories.is_empty() {
        out.push_str("(empty)\n");
    } else {
        for memory in memories {
            let _ = writeln!(out, "- [{}] {}", memory.id, memory.text.trim());
        }
    }
    out.push_str("Use remember for durable facts and preferences the user reveals (not transient task details), and forget (with the id) for ones that are wrong or outdated. This list is as of the start of this conversation.\n");

    out.push_str("\n## Skills\n\n");
    if skills.is_empty() {
        out.push_str("(none yet)\n");
    } else {
        for skill in skills {
            let _ = writeln!(out, "- {}: {}", skill.name, skill.description.trim());
        }
        out.push_str("Call use_skill to load one before doing that task.\n");
    }
    out.push_str("When you figure out a repeatable process, save it with save_skill.\n");

    out.push_str("\n## Routines\n\n");
    if routines.is_empty() {
        out.push_str("(none)\n");
    } else {
        for routine in routines {
            let _ = writeln!(
                out,
                "- [{}] {}: \"{}\"{}: {}",
                routine.id,
                routine.name,
                routine.schedule,
                if routine.enabled { "" } else { " (paused)" },
                routine.instruction.trim()
            );
        }
    }
    let _ = writeln!(
        out,
        "To do something on a schedule, call create_routine (a five-field cron in the user's time zone, {}). list_routines shows the current list; delete_routine removes one.",
        cron::local_zone_label()
    );

    out.push_str("\n## Other dots\n\n");
    if others.is_empty() {
        out.push_str("(you're the only dot)\n");
    } else {
        for other in others {
            if other.purpose.trim().is_empty() {
                let _ = writeln!(out, "- {}", other.name);
            } else {
                let _ = writeln!(out, "- {}: {}", other.name, other.purpose.trim());
            }
        }
        out.push_str("Use message_dot to consult them or hand off a sub-task. It doesn't wait: their reply arrives later in this conversation as a message that starts with \"[Reply from <name>]\". Carry on meanwhile, or end your turn if you need the answer first.\n");
    }

    out.push_str(
        "\n## Your tools\n\n\
They come from the `ghostex-dots` MCP server (in Claude Code they are named `mcp__ghostex-dots__<tool>`):\n\
- remember, forget: your memory.\n\
- save_skill, use_skill: your skills.\n\
- create_routine, delete_routine, list_routines: your routines.\n\
- send_update: message the user with a notification; share_file: hand them a file from your folder.\n\
- message_dot: ask another dot for help.\n\
- request_approval: ask before an action; ask_user: ask a question and wait for the answer.\n\
- sign_in: fill a login form with the user's saved password.\n\
- open_url, read_page, page_snapshot, click, type_text, press_key, scroll, screenshot, go_back: your browser.\n",
    );
    if apps.is_some() {
        out.push_str("- COMPOSIO_SEARCH_TOOLS, COMPOSIO_MULTI_EXECUTE_TOOL and the other COMPOSIO_ tools: the user's apps; app_connect: ask them to connect one.\n");
    }

    out.push_str(
        "\n## How a conversation starts\n\n\
- A message starting with \"[Routine: <name>]\" was started by your routine. The user is not watching: do the work, then deliver the result with send_update (with a title) and stop there, without a closing message. If there's nothing worth reporting, say so briefly without send_update.\n\
- A message starting with \"[Trigger: <name>]\" was started by your trigger: something just happened in one of the user's apps (the event data follows). The user is not watching. Follow the trigger's instruction; anything that sends, posts, pays or changes something still needs approval. Report back with send_update (with a title) only if there's something worth telling them, and stop once you've sent it.\n\
- A message starting with \"[From <dot>]\" comes from another dot. Answer them directly and concisely: your final message is sent back to them.\n\
- A message starting with \"[Reply from <dot>]\" is another dot's answer to your message_dot.\n\
- A message starting with \"[Team chat #<name>]\" makes you the lead of a team of dots: hand focused subtasks to the members with message_dot, wait for their replies, then give the user the combined result.\n\
- Anything else is the user talking to you.\n",
    );
    out
}

fn skill_markdown(skill: &Skill) -> String {
    let description = skill
        .description
        .replace(['\n', '\r'], " ")
        .replace('"', "'");
    format!(
        "---\nname: {}\ndescription: \"{}\"\n---\n\n{SKILL_MARKER} from the skill \"{}\"; edit it in OpenDots, changes made here are replaced. -->\n\n{}\n",
        skill_slug(&skill.name),
        description.trim(),
        skill.name,
        skill.body.trim()
    )
}

fn write_if_changed(path: &Path, contents: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

fn write_skills(folder: &Path, skills: &[Skill]) -> std::io::Result<()> {
    let skills_dir = folder.join(".claude").join("skills");
    let mut wanted = BTreeSet::new();
    for skill in skills {
        let slug = skill_slug(&skill.name);
        write_if_changed(
            &skills_dir.join(&slug).join("SKILL.md"),
            &skill_markdown(skill),
        )?;
        wanted.insert(slug);
    }
    for entry in std::fs::read_dir(&skills_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if wanted.contains(&name) {
            continue;
        }
        let ours = std::fs::read_to_string(entry.path().join("SKILL.md"))
            .is_ok_and(|text| text.contains(SKILL_MARKER));
        if ours {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

fn connected_apps(paths: &GxserverPaths) -> Option<Vec<String>> {
    if !composio::signed_in(paths) {
        return None;
    }
    Some(
        composio::apps(paths)
            .into_iter()
            .filter(|app| app.get("connected").and_then(serde_json::Value::as_bool) == Some(true))
            .filter_map(|app| {
                app.get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect(),
    )
}

/// Rewrites one dot's files.
pub(crate) fn write_dot_files(
    paths: &GxserverPaths,
    db: &Connection,
    dot: &Dot,
) -> std::io::Result<()> {
    write_dot_files_with(db, dot, connected_apps(paths))?;
    remember_folder_trusted(&dot.folder);
    Ok(())
}

/// CDXC:OpenDots 2026-10-01 WHY:
/// Ghostex makes each dot's folder itself and fills it only with its own files, so it goes on Ghostex's "Trust and Remember" list (`session_chat_trust_memory`) the moment the dot exists: an agent's folder-trust question would otherwise stop the dot's first routine, trigger or hand-off run, which nobody is watching, until the user answered it.
fn remember_folder_trusted(folder: &str) {
    let folders = [folder.to_string()];
    if !crate::session_chat_trust_memory::folders_remembered(&folders) {
        let _ = crate::session_chat_trust_memory::remember_trust_folders(&folders);
    }
}

fn write_dot_files_with(
    db: &Connection,
    dot: &Dot,
    apps: Option<Vec<String>>,
) -> std::io::Result<()> {
    let to_io = |error: crate::domain::DomainStateError| std::io::Error::other(error.message);
    let folder = Path::new(&dot.folder);
    std::fs::create_dir_all(folder)?;
    let skills = store::list_skills(db, Some(&dot.id)).map_err(to_io)?;
    let mut sites: Vec<String> = store::list_passwords(db)
        .map_err(to_io)?
        .into_iter()
        .map(|password| password.site)
        .collect();
    sites.sort();
    sites.dedup();
    let context = Context {
        rules: store::rules_for(db, &dot.id).map_err(to_io)?,
        memories: store::list_memories(db, Some(&dot.id)).map_err(to_io)?,
        routines: store::list_routines(db, Some(&dot.id)).map_err(to_io)?,
        others: store::list_dots(db)
            .map_err(to_io)?
            .into_iter()
            .filter(|other| other.id != dot.id)
            .collect(),
        skills: skills.clone(),
        sites,
        apps,
        dot: dot.clone(),
    };
    write_if_changed(&agents_file(folder), &agents_markdown(&context))?;
    write_skills(folder, &skills)?;
    let config = serde_json::to_string_pretty(&mcp_config(&dot.id)).unwrap_or_default();
    write_if_changed(&mcp_config_file(folder), &format!("{config}\n"))?;
    Ok(())
}

/// Rewrites one dot's files, reading its current record. Errors are not the caller's problem: the
/// files are rebuilt on the next change.
pub(crate) fn refresh_dot(paths: &GxserverPaths, dot_id: &str) {
    let Ok(db) = store::open(paths) else {
        return;
    };
    if let Ok(Some(dot)) = store::get_dot(&db, dot_id) {
        let _ = write_dot_files(paths, &db, &dot);
    }
}

/// Rewrites every dot's files (a change every dot reads: global rules, saved logins, the dot list,
/// the connected apps).
pub(crate) fn refresh_all(paths: &GxserverPaths) {
    let Ok(db) = store::open(paths) else {
        return;
    };
    let apps = connected_apps(paths);
    for dot in store::list_dots(&db).unwrap_or_default() {
        let _ = write_dot_files_with(&db, &dot, apps.clone());
    }
}
