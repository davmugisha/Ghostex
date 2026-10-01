//! Real mouse and keyboard events for the dot's page (`Input.dispatchMouseEvent`,
//! `Input.dispatchKeyEvent`, `Input.insertText`), used by the dot's own tools and by the user's
//! input while they drive from the Computer tab. Key names follow Open Dot's `KEY_MAP`.

use serde_json::{json, Value};

use super::cdp::Cdp;

const ALT: i64 = 1;
const CONTROL: i64 = 2;
const META: i64 = 4;
const SHIFT: i64 = 8;

/// Open Dot's key aliases (`ENTER`, `ESC`, `CMD`, ...) mapped to DOM key names.
fn normalize_key(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let mapped = match upper.as_str() {
        "ENTER" | "RETURN" => "Enter",
        "ESC" | "ESCAPE" => "Escape",
        "TAB" => "Tab",
        "SPACE" => " ",
        "BACKSPACE" => "Backspace",
        "DELETE" | "DEL" => "Delete",
        "UP" | "ARROWUP" => "ArrowUp",
        "DOWN" | "ARROWDOWN" => "ArrowDown",
        "LEFT" | "ARROWLEFT" => "ArrowLeft",
        "RIGHT" | "ARROWRIGHT" => "ArrowRight",
        "CTRL" | "CONTROL" => "Control",
        "CMD" | "COMMAND" | "META" | "SUPER" => "Meta",
        "ALT" | "OPTION" => "Alt",
        "SHIFT" => "Shift",
        "PAGEUP" => "PageUp",
        "PAGEDOWN" => "PageDown",
        "HOME" => "Home",
        "END" => "End",
        "INSERT" => "Insert",
        _ => "",
    };
    if !mapped.is_empty() {
        return mapped.to_string();
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(_), None) => name.to_string(),
        (Some(first), Some(_)) => {
            let rest: String = name.chars().skip(1).collect();
            format!(
                "{}{}",
                first.to_ascii_uppercase(),
                rest.to_ascii_lowercase()
            )
        }
        _ => String::new(),
    }
}

fn modifier_bit(key: &str) -> Option<i64> {
    match key {
        "Alt" => Some(ALT),
        "Control" => Some(CONTROL),
        "Meta" => Some(META),
        "Shift" => Some(SHIFT),
        _ => None,
    }
}

/// `{ key, code, windowsVirtualKeyCode, text }` for one DOM key name.
fn key_definition(key: &str) -> (String, String, i64, Option<String>) {
    let named = |code: &str, vk: i64, text: Option<&str>| {
        (
            key.to_string(),
            code.to_string(),
            vk,
            text.map(str::to_string),
        )
    };
    match key {
        "Enter" => return named("Enter", 13, Some("\r")),
        "Tab" => return named("Tab", 9, None),
        "Escape" => return named("Escape", 27, None),
        "Backspace" => return named("Backspace", 8, None),
        "Delete" => return named("Delete", 46, None),
        "Insert" => return named("Insert", 45, None),
        "ArrowUp" => return named("ArrowUp", 38, None),
        "ArrowDown" => return named("ArrowDown", 40, None),
        "ArrowLeft" => return named("ArrowLeft", 37, None),
        "ArrowRight" => return named("ArrowRight", 39, None),
        "PageUp" => return named("PageUp", 33, None),
        "PageDown" => return named("PageDown", 34, None),
        "Home" => return named("Home", 36, None),
        "End" => return named("End", 35, None),
        " " => return named("Space", 32, Some(" ")),
        "Control" => return named("ControlLeft", 17, None),
        "Shift" => return named("ShiftLeft", 16, None),
        "Alt" => return named("AltLeft", 18, None),
        "Meta" => return named("MetaLeft", 91, None),
        _ => {}
    }
    if let Some(number) = key.strip_prefix('F').and_then(|n| n.parse::<i64>().ok()) {
        if (1..=24).contains(&number) {
            return named(key, 111 + number, None);
        }
    }
    let mut chars = key.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return (key.to_string(), key.to_string(), 0, None);
    };
    let text = Some(ch.to_string());
    if ch.is_ascii_alphabetic() {
        let upper = ch.to_ascii_uppercase();
        return (key.to_string(), format!("Key{upper}"), upper as i64, text);
    }
    if ch.is_ascii_digit() {
        return (key.to_string(), format!("Digit{ch}"), ch as i64, text);
    }
    let (code, vk) = match ch {
        ';' => ("Semicolon", 186),
        '=' => ("Equal", 187),
        ',' => ("Comma", 188),
        '-' => ("Minus", 189),
        '.' => ("Period", 190),
        '/' => ("Slash", 191),
        '`' => ("Backquote", 192),
        '[' => ("BracketLeft", 219),
        '\\' => ("Backslash", 220),
        ']' => ("BracketRight", 221),
        '\'' => ("Quote", 222),
        _ => ("", 0),
    };
    (key.to_string(), code.to_string(), vk, text)
}

/// The editing command Chrome needs for a macOS shortcut (headless Chrome has no menu bar to
/// turn Cmd+A into "select all").
fn mac_command(modifiers: i64, key: &str) -> Option<&'static str> {
    if !cfg!(target_os = "macos") || modifiers & META == 0 {
        return None;
    }
    let shift = modifiers & SHIFT != 0;
    match key.to_ascii_lowercase().as_str() {
        "a" => Some("selectAll"),
        "c" => Some("copy"),
        "x" => Some("cut"),
        "v" => Some("paste"),
        "z" if shift => Some("redo"),
        "z" => Some("undo"),
        _ => None,
    }
}

/// Presses a key or a combination such as `Enter`, `ctrl+a`, `Meta+Shift+z`. Returns the DOM
/// name of the combination it pressed.
pub(super) fn press(cdp: &mut Cdp, combo: &str) -> Result<String, String> {
    let combo = combo.trim();
    if combo.is_empty() {
        return Err("Name a key to press, like Enter or Tab.".to_string());
    }
    let mut parts: Vec<String> = if combo == "+" {
        vec!["+".to_string()]
    } else {
        let mut parts: Vec<&str> = combo.split('+').collect();
        if combo.ends_with("++") {
            parts.truncate(parts.len().saturating_sub(2));
            parts.push("+");
        }
        parts
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .map(|part| normalize_key(part.trim()))
            .collect()
    };
    let main = parts
        .pop()
        .ok_or_else(|| "Name a key to press.".to_string())?;
    let mut modifiers = 0;
    for part in &parts {
        let bit = modifier_bit(part).ok_or_else(|| {
            format!("{part} is not a modifier key (use Control, Shift, Alt or Meta).")
        })?;
        modifiers |= bit;
        key_event(cdp, "rawKeyDown", part, modifiers)?;
    }
    key_down_up(cdp, &main, modifiers)?;
    for part in parts.iter().rev() {
        modifiers &= !modifier_bit(part).unwrap_or(0);
        key_event(cdp, "keyUp", part, modifiers)?;
    }
    parts.push(main);
    Ok(parts.join("+"))
}

fn key_down_up(cdp: &mut Cdp, key: &str, modifiers: i64) -> Result<(), String> {
    // Only plain or shifted keys produce text; Control/Alt/Meta combinations are shortcuts.
    let types_text = modifiers & (CONTROL | ALT | META) == 0;
    let (_, _, _, text) = key_definition(key);
    let down = if types_text && text.is_some() {
        "keyDown"
    } else {
        "rawKeyDown"
    };
    key_event(cdp, down, key, modifiers)?;
    key_event(cdp, "keyUp", key, modifiers)
}

fn key_event(cdp: &mut Cdp, kind: &str, key: &str, modifiers: i64) -> Result<(), String> {
    let (key, code, vk, text) = key_definition(key);
    let mut params = json!({
        "type": kind,
        "key": key,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
        "modifiers": modifiers,
    });
    if kind == "keyDown" {
        if let Some(text) = text {
            params["text"] = Value::String(text.clone());
            params["unmodifiedText"] = Value::String(text);
        }
    }
    if kind != "keyUp" {
        if let Some(command) = mac_command(modifiers, &key) {
            params["commands"] = json!([command]);
        }
    }
    cdp.call("Input.dispatchKeyEvent", params).map(|_| ())
}

/// Types text the way a person would: one key event per character for a single character (so
/// sites that listen for keys see them), `Input.insertText` for longer text or characters with
/// no key.
pub(super) fn type_text(cdp: &mut Cdp, text: &str) -> Result<(), String> {
    let mut chars = text.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        let key = if ch == '\n' {
            "Enter".to_string()
        } else {
            ch.to_string()
        };
        let (_, code, _, _) = key_definition(&key);
        if !code.is_empty() || ch.is_ascii_graphic() {
            return key_down_up(cdp, &key, 0);
        }
    }
    insert_text(cdp, text)
}

pub(super) fn insert_text(cdp: &mut Cdp, text: &str) -> Result<(), String> {
    cdp.call("Input.insertText", json!({ "text": text }))
        .map(|_| ())
}

fn clamp_point(x: f64, y: f64) -> (f64, f64) {
    (
        x.clamp(0.0, f64::from(super::SCREEN_WIDTH) - 1.0),
        y.clamp(0.0, f64::from(super::SCREEN_HEIGHT) - 1.0),
    )
}

pub(super) fn mouse_move(cdp: &mut Cdp, x: f64, y: f64) -> Result<(), String> {
    let (x, y) = clamp_point(x, y);
    cdp.call(
        "Input.dispatchMouseEvent",
        json!({ "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0 }),
    )
    .map(|_| ())
}

/// A full click (`clicks` = 2 for a double click: a single click, then one with clickCount 2,
/// as a browser sees it).
pub(super) fn mouse_click(
    cdp: &mut Cdp,
    x: f64,
    y: f64,
    button: &str,
    clicks: u32,
) -> Result<(), String> {
    let (x, y) = clamp_point(x, y);
    let buttons = match button {
        "right" => 2,
        "middle" => 4,
        _ => 1,
    };
    mouse_move(cdp, x, y)?;
    for count in 1..=clicks.max(1) {
        cdp.call(
            "Input.dispatchMouseEvent",
            json!({ "type": "mousePressed", "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": count }),
        )?;
        cdp.call(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": x, "y": y, "button": button, "buttons": 0, "clickCount": count }),
        )?;
    }
    Ok(())
}

pub(super) fn mouse_wheel(
    cdp: &mut Cdp,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
) -> Result<(), String> {
    let (x, y) = clamp_point(x, y);
    cdp.call(
        "Input.dispatchMouseEvent",
        json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": delta_x, "deltaY": delta_y }),
    )
    .map(|_| ())
}
