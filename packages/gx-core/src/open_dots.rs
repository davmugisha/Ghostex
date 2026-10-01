//! OpenDots rules every renderer shares: a dot's look (the flat character Open Dot draws for each
//! personal agent), its palettes and starters, and the face itself as an SVG document, ported from
//! Open Dot's `DotOrb.tsx` and `lib/look.ts`. The desktop sidebar, the OpenDots view and the web
//! build draw `dot_face_svg`; the phone draws the same shapes with react-native-svg
//! (apps/mobile/app/src/dots/DotFace.tsx), so a change here lands there too.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! Open Dot draws a 3D (three.js) dot on its stages and a flat SVG one in lists; Ghostex has no WebGL surface in GPUI or the phone, so the flat face is the dot everywhere.
//! SEE-ALSO: apps/mobile/app/src/dots/DotFace.tsx, server/src/open_dots/ (validates the same values).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DOT_SHAPES: [&str; 3] = ["round", "chubby", "tall"];
pub const DOT_MATERIALS: [&str; 4] = ["soft", "glossy", "velvet", "toon"];
pub const DOT_EYES: [&str; 4] = ["classic", "happy", "wide", "wink"];
pub const DOT_ACCESSORIES: [&str; 7] = [
    "none",
    "bow",
    "cap",
    "antenna",
    "halo",
    "sprout",
    "headphones",
];
/// Body colors: soft pastels.
pub const DOT_COLORS: [&str; 12] = [
    "#ffa7c4", "#ffb8a1", "#ffd98a", "#c8ec8f", "#97e0b8", "#8fdcd6", "#9fcbff", "#b9b3ff",
    "#dcb4f5", "#f4efe6", "#c9c3bb", "#6e6a78",
];
/// Feet colors, paired by index with the body colors.
pub const DOT_FEET_COLORS: [&str; 12] = [
    "#d8195f", "#e0492d", "#d98a00", "#4f9a2f", "#1f8f5f", "#138a8a", "#2a6fdb", "#5b46d6",
    "#9b3cc4", "#3b3842", "#8a5a3c", "#f4f4f4",
];
pub const DOT_EYE_COLORS: [&str; 5] = ["#2b5fd9", "#1f8f5f", "#8a4bd6", "#c2410c", "#3b3842"];

/// A dot's look, exactly the wire shape (`{ shape, color, accent, eyeColor, material, eyes, accessory }`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DotLook {
    pub shape: String,
    pub color: String,
    pub accent: String,
    pub eye_color: String,
    pub material: String,
    pub eyes: String,
    pub accessory: String,
}

impl Default for DotLook {
    fn default() -> Self {
        DotLook {
            shape: "round".to_string(),
            color: "#ffa7c4".to_string(),
            accent: "#d8195f".to_string(),
            eye_color: "#2b5fd9".to_string(),
            material: "soft".to_string(),
            eyes: "classic".to_string(),
            accessory: "none".to_string(),
        }
    }
}

/// Feet color that goes with a body color (same index in the paired palettes).
pub fn dot_feet_for(body: &str) -> &'static str {
    DOT_COLORS
        .iter()
        .position(|color| color.eq_ignore_ascii_case(body))
        .map(|index| DOT_FEET_COLORS[index])
        .unwrap_or("#d8195f")
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

impl DotLook {
    /// Open Dot's `normalizeLook`: any stored or wire value becomes a valid look; unknown values
    /// fall back to the defaults (and the feet follow the body color).
    pub fn from_value(value: Option<&Value>) -> DotLook {
        let defaults = DotLook::default();
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
            .unwrap_or(&defaults.color)
            .to_string();
        let accent = text("accent")
            .filter(|accent| DOT_FEET_COLORS.contains(accent))
            .map(str::to_string)
            .unwrap_or_else(|| dot_feet_for(&color).to_string());
        DotLook {
            shape: one_of("shape", &DOT_SHAPES, &defaults.shape),
            accent,
            eye_color: text("eyeColor")
                .filter(|color| is_hex_color(color))
                .unwrap_or(&defaults.eye_color)
                .to_string(),
            material: one_of("material", &DOT_MATERIALS, &defaults.material),
            eyes: one_of("eyes", &DOT_EYES, &defaults.eyes),
            accessory: one_of("accessory", &DOT_ACCESSORIES, &defaults.accessory),
            color,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// A starting point on the New dot page (Open Dot's STARTERS).
pub struct DotStarter {
    pub name: &'static str,
    pub purpose: &'static str,
    pub look: fn() -> DotLook,
}

pub const DOT_STARTERS: [DotStarter; 4] = [
    DotStarter {
        name: "Scout",
        purpose: "Research anything I ask and deliver clear, sourced briefs",
        look: || DotLook {
            color: "#9fcbff".into(),
            accent: "#2a6fdb".into(),
            accessory: "antenna".into(),
            ..DotLook::default()
        },
    },
    DotStarter {
        name: "Juniper",
        purpose: "Keep an eye on my inbox and draft replies in my voice",
        look: || DotLook {
            color: "#97e0b8".into(),
            accent: "#1f8f5f".into(),
            eye_color: "#1f8f5f".into(),
            accessory: "sprout".into(),
            shape: "chubby".into(),
            ..DotLook::default()
        },
    },
    DotStarter {
        name: "Atlas",
        purpose: "Plan trips and track prices for flights and hotels",
        look: || DotLook {
            color: "#ffd98a".into(),
            accent: "#e0492d".into(),
            eye_color: "#c2410c".into(),
            accessory: "cap".into(),
            eyes: "wide".into(),
            ..DotLook::default()
        },
    },
    DotStarter {
        name: "Bix",
        purpose: "Write and run code and scripts on its computer",
        look: || DotLook {
            color: "#b9b3ff".into(),
            accent: "#5b46d6".into(),
            eye_color: "#8a4bd6".into(),
            accessory: "headphones".into(),
            material: "glossy".into(),
            ..DotLook::default()
        },
    },
];

/// Name ideas for the New dot placeholder (Open Dot's NAME_IDEAS).
pub const DOT_NAME_IDEAS: [&str; 12] = [
    "Pixel", "Mochi", "Juniper", "Atlas", "Nova", "Pebble", "Scout", "Echo", "Clove", "Orbit",
    "Sprout", "Bix",
];

/// A dot's state as the face and the status dot draw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DotFaceStatus {
    #[default]
    Idle,
    Working,
    Waiting,
    Paused,
}

impl DotFaceStatus {
    pub fn parse(value: &str) -> DotFaceStatus {
        match value {
            "working" => DotFaceStatus::Working,
            "waiting" => DotFaceStatus::Waiting,
            "paused" => DotFaceStatus::Paused,
            _ => DotFaceStatus::Idle,
        }
    }

    /// Short status word (Open Dot's `statusLabel`).
    pub fn label(self) -> &'static str {
        match self {
            DotFaceStatus::Idle => "Ready",
            DotFaceStatus::Working => "Working",
            DotFaceStatus::Waiting => "Needs you",
            DotFaceStatus::Paused => "Paused",
        }
    }

    /// The small status dot's color (working blue, waiting amber, paused grey, idle green).
    pub fn color_rgb(self) -> u32 {
        match self {
            DotFaceStatus::Idle => 0x4ade80,
            DotFaceStatus::Working => 0x60a5fa,
            DotFaceStatus::Waiting => 0xf59e0b,
            DotFaceStatus::Paused => 0x64748b,
        }
    }
}

const INK: &str = "#1c1622";

fn parse_hex(hex: &str) -> Option<[f32; 3]> {
    if !is_hex_color(hex) {
        return None;
    }
    let channel = |start: usize| u8::from_str_radix(&hex[start..start + 2], 16).ok();
    Some([channel(1)? as f32, channel(3)? as f32, channel(5)? as f32])
}

/// Open Dot's `mix`: `hex` moved `t` of the way toward `other`, as `rgb(r,g,b)`.
fn mix(hex: &str, other: &str, t: f32) -> String {
    match (parse_hex(hex), parse_hex(other)) {
        (Some(a), Some(b)) => format!(
            "rgb({},{},{})",
            (a[0] + (b[0] - a[0]) * t).round() as u8,
            (a[1] + (b[1] - a[1]) * t).round() as u8,
            (a[2] + (b[2] - a[2]) * t).round() as u8
        ),
        _ => hex.to_string(),
    }
}

fn eye(x: f32, eye_y: f32, style: &str, eye_color: &str) -> String {
    match style {
        "happy" => format!(
            r#"<path d="M{} {} Q{} {} {} {}" fill="none" stroke="{INK}" stroke-width="2.8" stroke-linecap="round"/>"#,
            x - 4.5,
            eye_y + 2.0,
            x,
            eye_y - 6.0,
            x + 4.5,
            eye_y + 2.0
        ),
        "closed" => format!(
            r#"<path d="M{} {} Q{} {} {} {}" fill="none" stroke="{INK}" stroke-width="2.8" stroke-linecap="round"/>"#,
            x - 4.5,
            eye_y,
            x,
            eye_y + 4.5,
            x + 4.5,
            eye_y
        ),
        _ => {
            let w = if style == "wide" { 1.2 } else { 1.0 };
            format!(
                r##"<ellipse cx="{x}" cy="{eye_y}" rx="{}" ry="8.6" fill="{INK}"/><ellipse cx="{x}" cy="{}" rx="{}" ry="4.6" fill="{eye_color}"/><ellipse cx="{x}" cy="{}" rx="{}" ry="3.4" fill="#ffffff"/>"##,
                4.8 * w,
                eye_y + 3.4,
                3.7 * w,
                eye_y - 3.6,
                2.7 * w
            )
        }
    }
}

/// The face as a standalone SVG document `size` pixels square (Open Dot's DotOrb, viewBox
/// `-2 -5 104 104`). Paused dots close their eyes and fade toward grey; a waiting dot raises its
/// right arm. Renderers rasterize it (GPUI `ImageFormat::Svg`).
pub fn dot_face_svg(look: &DotLook, status: DotFaceStatus, size: u32) -> String {
    let (rx, ry) = match look.shape.as_str() {
        "chubby" => (39.5_f32, 33.5_f32),
        "tall" => (33.5, 39.0),
        _ => (36.0, 36.0),
    };
    let sleeping = status == DotFaceStatus::Paused;
    let waiting = status == DotFaceStatus::Waiting;
    let cy = 52.0_f32;
    let top = cy - ry;
    let eye_y = cy - 9.0;
    let trim = if look.accent.eq_ignore_ascii_case("#f4f4f4") {
        "#d8195f".to_string()
    } else {
        look.accent.clone()
    };
    let eye_style = if sleeping {
        "closed"
    } else {
        look.eyes.as_str()
    };
    let sw = 3.4_f32;
    let body = if sleeping {
        mix(&look.color, "#9a9aa2", 0.45)
    } else {
        look.color.clone()
    };
    let body_hex = parse_hex(&look.color)
        .map(|_| look.color.clone())
        .unwrap_or_else(|| "#ffa7c4".to_string());
    let light = mix(&body_hex, "#ffffff", 0.45);
    let dark = mix(&body_hex, "#000000", 0.12);
    let gloss = matches!(look.material.as_str(), "glossy");
    let mut svg = String::with_capacity(4096);
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -5 104 104" width="{size}" height="{size}">"#
    ));
    svg.push_str(&format!(
        r#"<defs><radialGradient id="b" cx="0.36" cy="0.3" r="0.8"><stop offset="0" stop-color="{}"/><stop offset="0.55" stop-color="{}"/><stop offset="1" stop-color="{}"/></radialGradient></defs>"#,
        if sleeping { body.clone() } else { light },
        body,
        if sleeping { body.clone() } else { dark }
    ));
    if look.accessory == "halo" {
        svg.push_str(&format!(
            r##"<ellipse cx="50" cy="{}" rx="20" ry="5" fill="none" stroke="#f5c518" stroke-width="3.5"/>"##,
            top - 8.0
        ));
    }
    // feet
    let feet_y = cy + ry - 3.0;
    svg.push_str(&format!(
        r#"<ellipse cx="33" cy="{feet_y}" rx="17" ry="10" transform="rotate(-10 33 {feet_y})" fill="{}" stroke="{INK}" stroke-width="{sw}"/>"#,
        look.accent
    ));
    svg.push_str(&format!(
        r#"<ellipse cx="67" cy="{feet_y}" rx="17" ry="10" transform="rotate(10 67 {feet_y})" fill="{}" stroke="{INK}" stroke-width="{sw}"/>"#,
        look.accent
    ));
    // arms
    let left_arm_x = 50.0 - rx + 1.0;
    let left_arm_y = cy + 4.0;
    svg.push_str(&format!(
        r#"<ellipse cx="{left_arm_x}" cy="{left_arm_y}" rx="9.5" ry="7.5" transform="rotate(-25 {left_arm_x} {left_arm_y})" fill="{body}" stroke="{INK}" stroke-width="{sw}"/>"#
    ));
    let right_arm_x = 50.0 + rx - 1.0;
    let right_arm_y = if waiting { cy - 16.0 } else { cy - 5.0 };
    let right_arm_rotation = if waiting { 45.0 } else { 35.0 };
    svg.push_str(&format!(
        r#"<ellipse cx="{right_arm_x}" cy="{right_arm_y}" rx="9.5" ry="7.5" transform="rotate({right_arm_rotation} {right_arm_x} {right_arm_y})" fill="{body}" stroke="{INK}" stroke-width="{sw}"/>"#
    ));
    // body
    svg.push_str(&format!(
        r#"<ellipse cx="50" cy="{cy}" rx="{rx}" ry="{ry}" fill="url(#b)" stroke="{INK}" stroke-width="{sw}"/>"#
    ));
    if gloss && !sleeping {
        svg.push_str(&format!(
            r##"<ellipse cx="{}" cy="{}" rx="7" ry="4.5" transform="rotate(-30 {} {})" fill="#ffffff" opacity="0.55"/>"##,
            50.0 - rx * 0.45,
            cy - ry * 0.55,
            50.0 - rx * 0.45,
            cy - ry * 0.55
        ));
    }
    // face
    let (left_style, right_style) = if eye_style == "wink" {
        ("classic", "closed")
    } else {
        (eye_style, eye_style)
    };
    svg.push_str(&eye(42.5, eye_y, left_style, &look.eye_color));
    svg.push_str(&eye(57.5, eye_y, right_style, &look.eye_color));
    let blush = mix(&body_hex, "#ff4f8b", 0.55);
    svg.push_str(&format!(
        r#"<ellipse cx="32" cy="{}" rx="6" ry="3.2" fill="{blush}" opacity="0.6"/><ellipse cx="68" cy="{}" rx="6" ry="3.2" fill="{blush}" opacity="0.6"/>"#,
        cy + 3.0,
        cy + 3.0
    ));
    svg.push_str(&format!(
        r#"<path d="M46.5 {} Q50 {} 53.5 {}" fill="none" stroke="{INK}" stroke-width="2.4" stroke-linecap="round"/>"#,
        cy + 2.5,
        cy + 6.5,
        cy + 2.5
    ));
    // accessories
    match look.accessory.as_str() {
        "bow" => svg.push_str(&format!(
            r#"<g transform="translate(66 {}) rotate(-20)"><ellipse cx="-7" cy="0" rx="8" ry="5.5" fill="{trim}" stroke="{INK}" stroke-width="2.6"/><ellipse cx="7" cy="0" rx="8" ry="5.5" fill="{trim}" stroke="{INK}" stroke-width="2.6"/><circle r="3.6" fill="{trim}" stroke="{INK}" stroke-width="2.4"/></g>"#,
            top + 8.0
        )),
        "cap" => {
            let cap_y = cy - ry * 0.58;
            let half = rx * (1.0 - ((cap_y - cy) / ry).powi(2)).sqrt() + 1.5;
            svg.push_str(&format!(
                r#"<path d="M{} {cap_y} A {half} {} 0 0 1 {} {cap_y} Z" fill="{trim}" stroke="{INK}" stroke-width="{sw}" stroke-linejoin="round"/>"#,
                50.0 - half,
                cap_y - top + 3.0,
                50.0 + half
            ));
            svg.push_str(&format!(
                r#"<path d="M50 {} Q 60 {} 62 {cap_y}" fill="none" stroke="{INK}" stroke-opacity="0.25" stroke-width="1.4"/>"#,
                top - 2.5,
                top + 4.0
            ));
            svg.push_str(&format!(
                r##"<circle cx="50" cy="{}" r="2.8" fill="{trim}" stroke="{INK}" stroke-width="2"/><circle cx="56" cy="{}" r="2.8" fill="#ffffff" stroke="{INK}" stroke-width="1.5"/>"##,
                top - 3.0,
                cap_y - 6.0
            ));
            svg.push_str(&format!(
                r#"<path d="M{} {} Q {} {} {} {} Q {} {} {} {} Z" fill="{}" stroke="{INK}" stroke-width="{sw}" stroke-linejoin="round"/>"#,
                50.0 + half * 0.35,
                cap_y + 1.0,
                50.0 + half + 14.0,
                cap_y - 3.0,
                50.0 + half + 20.0,
                cap_y + 3.0,
                50.0 + half + 8.0,
                cap_y + 7.5,
                50.0 + half * 0.35,
                cap_y + 5.0,
                mix(&trim, "#000000", 0.18)
            ));
        }
        "antenna" => svg.push_str(&format!(
            r#"<line x1="50" y1="{top}" x2="50" y2="{}" stroke="{INK}" stroke-width="2.6"/><circle cx="50" cy="{}" r="4.6" fill="{trim}" stroke="{INK}" stroke-width="2.4"/>"#,
            top - 11.0,
            top - 13.0
        )),
        "sprout" => svg.push_str(&format!(
            r##"<line x1="50" y1="{top}" x2="50" y2="{}" stroke="#3f8f4a" stroke-width="2.6"/><ellipse cx="44" cy="{}" rx="6.5" ry="3.4" transform="rotate(25 44 {})" fill="#62c46f" stroke="{INK}" stroke-width="2.2"/><ellipse cx="56" cy="{}" rx="6.5" ry="3.4" transform="rotate(-25 56 {})" fill="#62c46f" stroke="{INK}" stroke-width="2.2"/>"##,
            top - 7.0,
            top - 9.0,
            top - 9.0,
            top - 10.0,
            top - 10.0
        )),
        "headphones" => svg.push_str(&format!(
            r##"<path d="M{} {} A {} {} 0 0 1 {} {}" fill="none" stroke="#2a2530" stroke-width="4.5"/><rect x="{}" y="{}" width="10" height="16" rx="4" fill="{trim}" stroke="{INK}" stroke-width="2.6"/><rect x="{}" y="{}" width="10" height="16" rx="4" fill="{trim}" stroke="{INK}" stroke-width="2.6"/>"##,
            50.0 - rx + 2.0,
            cy - 10.0,
            rx - 2.0,
            ry + 2.0,
            50.0 + rx - 2.0,
            cy - 10.0,
            50.0 - rx - 4.0,
            cy - 18.0,
            50.0 + rx - 6.0,
            cy - 18.0
        )),
        _ => {}
    }
    svg.push_str("</svg>");
    svg
}

/// The dots wordmark (three overlapping circles, Open Dot's logo), drawn in `color` for the Space
/// row slot and the top-row toggle.
pub fn dots_mark_svg(size: u32, color: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="{size}" height="{size}"><circle cx="7" cy="12" r="4.2" fill="{color}"/><circle cx="12" cy="12" r="4.2" fill="{color}" fill-opacity="0.7"/><circle cx="17" cy="12" r="4.2" fill="{color}" fill-opacity="0.45"/></svg>"#
    )
}
