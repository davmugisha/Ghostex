//! Where the OpenDots view opens: the page and, on a dot's page, the tab. The sidebar's
//! `openDotsView` host message names one (`OpenDotsTarget::from_message`).

use serde_json::Value;

/// A dot page's tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OpenDotsDotTab {
    #[default]
    Chats,
    Computer,
    Setup,
}

impl OpenDotsDotTab {
    pub(crate) const ALL: [OpenDotsDotTab; 3] = [
        OpenDotsDotTab::Chats,
        OpenDotsDotTab::Computer,
        OpenDotsDotTab::Setup,
    ];

    pub(crate) fn parse(value: &str) -> OpenDotsDotTab {
        match value {
            "computer" => OpenDotsDotTab::Computer,
            "setup" => OpenDotsDotTab::Setup,
            _ => OpenDotsDotTab::Chats,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            OpenDotsDotTab::Chats => "Chats",
            OpenDotsDotTab::Computer => "Computer",
            OpenDotsDotTab::Setup => "Setup",
        }
    }
}

/// Which OpenDots page to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum OpenDotsTarget {
    #[default]
    Home,
    Dot {
        dot_id: String,
        tab: OpenDotsDotTab,
    },
    NewDot,
    Settings,
    /// Open Dot's "New channel": a team of dots led by one of them.
    NewTeam,
}

impl OpenDotsTarget {
    /// The sidebar's `{"type":"openDotsView","page":"home|dot|new|settings|team","dotId"?,"tab"?}`.
    pub(crate) fn from_message(message: &Value) -> Option<OpenDotsTarget> {
        let text = |key: &str| message.get(key).and_then(Value::as_str).map(str::trim);
        Some(match text("page").unwrap_or("home") {
            "home" => OpenDotsTarget::Home,
            "new" => OpenDotsTarget::NewDot,
            "settings" => OpenDotsTarget::Settings,
            "team" => OpenDotsTarget::NewTeam,
            "dot" => OpenDotsTarget::Dot {
                dot_id: text("dotId").filter(|id| !id.is_empty())?.to_string(),
                tab: OpenDotsDotTab::parse(text("tab").unwrap_or("chats")),
            },
            _ => return None,
        })
    }
}
