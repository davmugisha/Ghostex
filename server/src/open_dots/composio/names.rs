//! App names, ported from Open Dot's `NAMES` / `prettyName` (composio.ts) and `TRIGGER_APPS`
//! (triggers.ts).

/// Composio toolkit slugs whose name is not just the slug capitalized.
const NAMES: &[(&str, &str)] = &[
    ("gmail", "Gmail"),
    ("googlecalendar", "Google Calendar"),
    ("googledrive", "Google Drive"),
    ("googlesheets", "Google Sheets"),
    ("googledocs", "Google Docs"),
    ("github", "GitHub"),
    ("linkedin", "LinkedIn"),
    ("metaads", "Meta Ads"),
    ("posthog", "PostHog"),
    ("serpapi", "SerpApi"),
    ("youtube", "YouTube"),
    ("google_search_console", "Search Console"),
    ("outlook", "Outlook"),
    ("notion", "Notion"),
    ("slack", "Slack"),
    ("linear", "Linear"),
    ("figma", "Figma"),
    ("reddit", "Reddit"),
    ("ahrefs", "Ahrefs"),
];

/// Apps offered for triggers, in Open Dot's order (plus anything already connected).
pub(crate) const TRIGGER_APPS: &[(&str, &str)] = &[
    ("gmail", "Gmail"),
    ("googlecalendar", "Google Calendar"),
    ("slack", "Slack"),
    ("github", "GitHub"),
    ("notion", "Notion"),
    ("outlook", "Outlook"),
    ("linear", "Linear"),
    ("jira", "Jira"),
    ("hubspot", "HubSpot"),
    ("discord", "Discord"),
    ("asana", "Asana"),
    ("googledrive", "Google Drive"),
];

/// `prettyName`: the known name, else `[_-]+` → space and every word capitalized.
pub(crate) fn pretty_name(slug: &str) -> String {
    if let Some((_, name)) = NAMES.iter().find(|(known, _)| *known == slug) {
        return (*name).to_string();
    }
    let mut spaced = String::with_capacity(slug.len());
    let mut in_separator = false;
    for c in slug.chars() {
        if c == '_' || c == '-' {
            if !in_separator {
                spaced.push(' ');
            }
            in_separator = true;
        } else {
            spaced.push(c);
            in_separator = false;
        }
    }
    let mut out = String::with_capacity(spaced.len());
    let mut previous_is_word = false;
    for c in spaced.chars() {
        let is_word = c.is_alphanumeric() || c == '_';
        if is_word && !previous_is_word {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        previous_is_word = is_word;
    }
    out
}

/// A trigger app's name: Open Dot's list, else the slug with its first letter capitalized.
pub(crate) fn trigger_app_name(slug: &str) -> String {
    if let Some((_, name)) = TRIGGER_APPS.iter().find(|(known, _)| *known == slug) {
        return (*name).to_string();
    }
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub(crate) fn logo_url(slug: &str) -> String {
    format!("https://logos.composio.dev/api/{slug}")
}
