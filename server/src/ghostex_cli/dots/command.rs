use std::io::Read as _;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::ghostex_cli::args::{parse_args, Flags, ParsedArgs};
use crate::ghostex_cli::output::print_json;
use crate::ghostex_cli::rpc::{
    request_gxserver_envelope, resolve_gxserver_server_target, CliError, CliResult,
    GXSERVER_PROTOCOL_VERSION,
};

/// CDXC:OpenDots 2026-10-01 WHY:
/// One verb family for people and scripts, plus `rpc`, which the phone uses for every OpenDots method over SSH (the same shape as `ghostex session-chat-rpc`), so the phone needs no endpoint of its own.
pub(crate) fn run(args: &[String]) -> CliResult<()> {
    let wants_help = args.is_empty()
        || args[0] == "help"
        || (args[0] != "rpc" && args.iter().any(|arg| arg == "-h" || arg == "--help"));
    if wants_help {
        print!("{}", include_str!("help.txt"));
        return Ok(());
    }
    let command = args[0].as_str();
    let parsed = parse_args(&args[1..]);
    match command {
        "list" | "ls" => list(&parsed),
        "show" => show(&parsed),
        "create" => create(&parsed),
        "chat" => chat(&parsed),
        "pause" | "resume" | "stop" | "delete" => dot_action(command, &parsed),
        "approvals" => approvals(&parsed),
        "approve" => resolve(
            &parsed,
            if parsed.flags.truthy("always") {
                "always"
            } else {
                "approve"
            },
        ),
        "deny" => resolve(&parsed, "deny"),
        "answer" => resolve(&parsed, "answer"),
        "rpc" => rpc_verb(&parsed),
        "mcp" => super::mcp::run(&parsed),
        other => Err(CliError::Other(format!(
            "Unknown dots command: {other}. See ghostex dots --help."
        ))),
    }
}

fn flag_text(flags: &Flags, key: &str) -> Option<String> {
    flags
        .string_value(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Only the flags the RPC layer reads; command flags stay out of it.
fn server_flags(flags: &Flags) -> Flags {
    let mut result = Flags::default();
    if let Some(server) = flags.string_value("server") {
        result.insert_text("server", server);
    }
    if let Some(timeout) = flags.string_value("timeout") {
        result.insert_text("timeout", timeout);
    }
    result
}

/// The daemon's envelope for one method: `Ok(result)` or the refusal.
pub(super) fn call(method: &str, params: Value, flags: &Flags) -> CliResult<Value> {
    let flags = server_flags(flags);
    let target = resolve_gxserver_server_target(&flags, &json!({}))?;
    let body = json!({
        "params": { "method": method, "params": params },
        "protocolVersion": GXSERVER_PROTOCOL_VERSION,
    });
    let envelope = request_gxserver_envelope(&target, "/api/openDotsRpc", &body, &flags)?;
    Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
}

fn snapshot(flags: &Flags) -> CliResult<Value> {
    let snapshot = call("snapshot", json!({}), flags)?;
    if snapshot.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Err(CliError::Other(
            "OpenDots is off. Turn it on in Settings > Extensions.".to_string(),
        ));
    }
    Ok(snapshot)
}

fn list_of<'a>(snapshot: &'a Value, key: &str) -> Vec<&'a Value> {
    snapshot
        .get(key)
        .and_then(Value::as_array)
        .map(|list| list.iter().collect())
        .unwrap_or_default()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `<dot>`: an id, or a name in any case.
fn find_dot<'a>(snapshot: &'a Value, reference: &str) -> CliResult<&'a Value> {
    let reference = reference.trim();
    list_of(snapshot, "dots")
        .into_iter()
        .find(|dot| text(dot, "id") == reference)
        .or_else(|| {
            list_of(snapshot, "dots")
                .into_iter()
                .find(|dot| text(dot, "name").eq_ignore_ascii_case(reference))
        })
        .ok_or_else(|| {
            CliError::Other(format!(
                "No dot named \"{reference}\". See ghostex dots list."
            ))
        })
}

fn dot_reference(parsed: &ParsedArgs) -> CliResult<String> {
    parsed
        .rest
        .first()
        .cloned()
        .or_else(|| flag_text(&parsed.flags, "dot"))
        .ok_or_else(|| CliError::Other("Name the dot (its name or id).".to_string()))
}

fn pending_for<'a>(snapshot: &'a Value, dot_id: &str) -> Vec<&'a Value> {
    list_of(snapshot, "approvals")
        .into_iter()
        .filter(|approval| text(approval, "status") == "pending")
        .filter(|approval| dot_id.is_empty() || text(approval, "dotId") == dot_id)
        .collect()
}

fn list(parsed: &ParsedArgs) -> CliResult<()> {
    let snapshot = snapshot(&parsed.flags)?;
    if parsed.flags.truthy("json") {
        print_json(
            &json!({ "ok": true, "dots": snapshot["dots"], "approvals": pending_for(&snapshot, "") }),
        );
        return Ok(());
    }
    let dots = list_of(&snapshot, "dots");
    if dots.is_empty() {
        println!("No dots yet. Make one with: ghostex dots create --name <name> --job \"<what it does>\"");
        return Ok(());
    }
    for dot in dots {
        let waiting = pending_for(&snapshot, text(dot, "id")).len();
        let mut line = format!("{}\t{}", text(dot, "name"), text(dot, "status"));
        if let Some(activity) = dot.get("activity").and_then(Value::as_str) {
            line.push_str(&format!("\t{activity}"));
        }
        if waiting > 0 {
            line.push_str(&format!("\t({waiting} need you)"));
        }
        println!("{line}");
    }
    Ok(())
}

fn show(parsed: &ParsedArgs) -> CliResult<()> {
    let snapshot = snapshot(&parsed.flags)?;
    let dot = find_dot(&snapshot, &dot_reference(parsed)?)?;
    let dot_id = text(dot, "id");
    let owned = |key: &str| -> Vec<&Value> {
        list_of(&snapshot, key)
            .into_iter()
            .filter(|row| text(row, "dotId") == dot_id)
            .collect()
    };
    let rules: Vec<&Value> = list_of(&snapshot, "rules")
        .into_iter()
        .filter(|rule| {
            rule.get("dotId").is_none_or(Value::is_null) || text(rule, "dotId") == dot_id
        })
        .collect();
    if parsed.flags.truthy("json") {
        print_json(&json!({
            "ok": true,
            "dot": dot,
            "rules": rules,
            "memories": owned("memories"),
            "skills": owned("skills"),
            "routines": owned("routines"),
            "triggers": owned("triggers"),
            "approvals": pending_for(&snapshot, dot_id),
        }));
        return Ok(());
    }
    println!("{} ({})", text(dot, "name"), dot_id);
    if !text(dot, "purpose").is_empty() {
        println!("Job: {}", text(dot, "purpose"));
    }
    let model = dot
        .get("model")
        .and_then(Value::as_str)
        .map(|model| format!(", model {model}"))
        .unwrap_or_default();
    println!("Agent: {}{model}", text(dot, "agentId"));
    println!(
        "Status: {}{}",
        text(dot, "status"),
        dot.get("activity")
            .and_then(Value::as_str)
            .map(|activity| format!(" ({activity})"))
            .unwrap_or_default()
    );
    println!("Folder: {}", text(dot, "folder"));
    let section = |title: &str, rows: Vec<String>| {
        if rows.is_empty() {
            return;
        }
        println!("\n{title}:");
        for row in rows {
            println!("  {row}");
        }
    };
    section(
        "Rules",
        rules
            .iter()
            .map(|rule| {
                format!(
                    "When it wants to {}: {}{}",
                    text(rule, "action"),
                    text(rule, "decision"),
                    if rule.get("dotId").is_none_or(Value::is_null) {
                        " (all dots)"
                    } else {
                        ""
                    }
                )
            })
            .collect(),
    );
    section(
        "Memory",
        owned("memories")
            .iter()
            .map(|memory| format!("[{}] {}", text(memory, "id"), text(memory, "text")))
            .collect(),
    );
    section(
        "Skills",
        owned("skills")
            .iter()
            .map(|skill| format!("{}: {}", text(skill, "name"), text(skill, "description")))
            .collect(),
    );
    section(
        "Routines",
        owned("routines")
            .iter()
            .map(|routine| {
                format!(
                    "[{}] {} \"{}\"{}",
                    text(routine, "id"),
                    text(routine, "name"),
                    text(routine, "schedule"),
                    if routine.get("enabled").and_then(Value::as_bool) == Some(false) {
                        " (paused)"
                    } else {
                        ""
                    }
                )
            })
            .collect(),
    );
    section(
        "Triggers",
        owned("triggers")
            .iter()
            .map(|trigger| format!("{} ({})", text(trigger, "name"), text(trigger, "toolkit")))
            .collect(),
    );
    section(
        "Waiting on you",
        pending_for(&snapshot, dot_id)
            .iter()
            .map(|approval| format!("{}  {}", text(approval, "id"), text(approval, "title")))
            .collect(),
    );
    Ok(())
}

fn create(parsed: &ParsedArgs) -> CliResult<()> {
    let flags = &parsed.flags;
    let name = flag_text(flags, "name")
        .or_else(|| parsed.rest.first().cloned())
        .ok_or_else(|| CliError::Other("Give the dot a name with --name <name>.".to_string()))?;
    let mut params = json!({ "name": name });
    for (flag, key) in [
        ("job", "purpose"),
        ("purpose", "purpose"),
        ("instructions", "instructions"),
        ("agent", "agentId"),
        ("model", "model"),
    ] {
        if let Some(value) = flag_text(flags, flag) {
            params[key] = json!(value);
        }
    }
    let created = call("createDot", params, flags)?;
    if flags.truthy("json") {
        print_json(&json!({ "ok": true, "dot": created["dot"] }));
    } else {
        println!(
            "Made {} ({}). Talk to it with: ghostex dots chat \"{}\" --message \"<what to do>\"",
            text(&created["dot"], "name"),
            text(&created["dot"], "id"),
            text(&created["dot"], "name"),
        );
    }
    Ok(())
}

fn chat(parsed: &ParsedArgs) -> CliResult<()> {
    let snapshot = snapshot(&parsed.flags)?;
    let dot = find_dot(&snapshot, &dot_reference(parsed)?)?;
    let mut params = json!({ "dotId": text(dot, "id") });
    if let Some(message) = flag_text(&parsed.flags, "message")
        .or_else(|| (parsed.rest.len() > 1).then(|| parsed.rest[1..].join(" ")))
    {
        params["message"] = json!(message);
    }
    if let Some(title) = flag_text(&parsed.flags, "title") {
        params["title"] = json!(title);
    }
    let started = call("startChat", params, &parsed.flags)?;
    if parsed.flags.truthy("json") {
        print_json(
            &json!({ "ok": true, "projectId": started["projectId"], "sessionId": started["sessionId"] }),
        );
    } else {
        println!(
            "Started a conversation with {} (session {}). Open it in Ghostex, under Dots.",
            text(dot, "name"),
            text(&started, "sessionId")
        );
    }
    Ok(())
}

fn dot_action(command: &str, parsed: &ParsedArgs) -> CliResult<()> {
    let snapshot = snapshot(&parsed.flags)?;
    let dot = find_dot(&snapshot, &dot_reference(parsed)?)?;
    let method = match command {
        "pause" => "pauseDot",
        "resume" => "resumeDot",
        "stop" => "stopDot",
        _ => "deleteDot",
    };
    let result = call(method, json!({ "dotId": text(dot, "id") }), &parsed.flags)?;
    if parsed.flags.truthy("json") {
        let mut output = json!({ "ok": true });
        if let Some(updated) = result.get("dot") {
            output["dot"] = updated.clone();
        }
        print_json(&output);
        return Ok(());
    }
    let name = text(dot, "name");
    match command {
        "pause" => println!(
            "Paused {name}. Its routines, triggers and hand-offs wait until you resume it."
        ),
        "resume" => println!("Resumed {name}."),
        "stop" => println!("Stopped what {name} was doing."),
        _ => println!(
            "Deleted {name}. Its folder {} was kept.",
            text(dot, "folder")
        ),
    }
    Ok(())
}

fn approvals(parsed: &ParsedArgs) -> CliResult<()> {
    let snapshot = snapshot(&parsed.flags)?;
    let pending = pending_for(&snapshot, "");
    if parsed.flags.truthy("json") {
        print_json(&json!({ "ok": true, "approvals": pending }));
        return Ok(());
    }
    if pending.is_empty() {
        println!("Nothing is waiting on you.");
        return Ok(());
    }
    for approval in pending {
        let dot_name = list_of(&snapshot, "dots")
            .into_iter()
            .find(|dot| text(dot, "id") == text(approval, "dotId"))
            .map(|dot| text(dot, "name"))
            .unwrap_or_default();
        println!(
            "{}\t{}\t{}\t{}",
            text(approval, "id"),
            dot_name,
            text(approval, "kind"),
            text(approval, "title")
        );
        if let Some(detail) = approval.get("detail").and_then(Value::as_str) {
            for line in detail.lines() {
                println!("\t\t{line}");
            }
        }
        if let Some(options) = approval.get("options").and_then(Value::as_array) {
            let options: Vec<&str> = options.iter().filter_map(Value::as_str).collect();
            if !options.is_empty() {
                println!("\t\tOptions: {}", options.join(" | "));
            }
        }
    }
    Ok(())
}

fn resolve(parsed: &ParsedArgs, choice: &str) -> CliResult<()> {
    let approval_id = parsed.rest.first().cloned().ok_or_else(|| {
        CliError::Other("Give the approval id (see ghostex dots approvals).".to_string())
    })?;
    let mut params = json!({ "approvalId": approval_id, "choice": choice });
    let answer = flag_text(&parsed.flags, "answer")
        .or_else(|| (parsed.rest.len() > 1).then(|| parsed.rest[1..].join(" ")));
    if let Some(answer) = answer {
        params["answer"] = json!(answer);
    }
    let result = call("resolveApproval", params, &parsed.flags)?;
    if parsed.flags.truthy("json") {
        print_json(&json!({ "ok": true, "approval": result["approval"] }));
    } else {
        println!(
            "{}: {}",
            text(&result["approval"], "title"),
            text(&result["approval"], "status")
        );
    }
    Ok(())
}

/// `ghostex dots rpc <method> --params-base64 <b64>` (or `--params '<json>'`, or `--params-stdin`
/// with the JSON on standard input): prints the daemon's envelope, `{ ok, result }` or
/// `{ ok: false, code, message }`.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// `--params-stdin` keeps secrets (a saved login's password, the Composio triggers key) off the command line, where every user of the computer can read them in the process list while the call runs; the phone sends those methods this way.
fn rpc_verb(parsed: &ParsedArgs) -> CliResult<()> {
    let method = parsed
        .rest
        .first()
        .cloned()
        .ok_or_else(|| CliError::Other("Give the method name.".to_string()))?;
    let params = if parsed.flags.truthy("paramsStdin") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| CliError::Other(format!("Could not read the params: {error}")))?;
        if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Value>(&text).map_err(|_| {
                CliError::Other("The params on standard input are not JSON.".to_string())
            })?
        }
    } else if let Some(encoded) = flag_text(&parsed.flags, "paramsBase64") {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded.as_bytes()))
            .map_err(|_| CliError::Other("--params-base64 is not base64.".to_string()))?;
        serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| CliError::Other("--params-base64 is not JSON.".to_string()))?
    } else if let Some(raw) = flag_text(&parsed.flags, "params") {
        serde_json::from_str::<Value>(&raw)
            .map_err(|_| CliError::Other("--params is not JSON.".to_string()))?
    } else {
        json!({})
    };
    match call(&method, params, &parsed.flags) {
        Ok(result) => print_json(&json!({ "ok": true, "result": result })),
        Err(CliError::Rpc { message, response }) => {
            print_json(&json!({
                "ok": false,
                "code": response.get("error").cloned().unwrap_or(json!("error")),
                "message": message,
            }));
            crate::ghostex_cli::set_exit_code(1);
        }
        Err(error) => {
            print_json(&json!({ "ok": false, "code": "connection", "message": error.to_string() }));
            crate::ghostex_cli::set_exit_code(1);
        }
    }
    Ok(())
}
