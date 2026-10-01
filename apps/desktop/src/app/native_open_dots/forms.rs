//! The text fields and choices of every OpenDots form. Fields need a window to exist, so each is
//! made the first time its page draws (`ensure_*`), and a form a successful call should clear is
//! cleared on the next draw (`clear_after_render`).

use std::collections::HashMap;

use super::model::{Dot, TriggerApp, TriggerType};
use super::view::NativeOpenDotsView;
use ghostex_gx_core::open_dots::DotLook;
use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use gpui_component::input::{InputEvent, InputState, TextareaState};

/// A field a page has finished with, emptied on the next draw (setting a value needs a window).
pub(crate) enum PendingClear {
    Input(Entity<InputState>),
    Textarea(Entity<TextareaState>),
}

#[derive(Default)]
pub(crate) struct Forms {
    pub(crate) home_message: Option<Entity<TextareaState>>,
    pub(crate) picked_dot: Option<String>,
    /// The dot the Home composer's placeholder names.
    pub(crate) home_placeholder_dot: Option<String>,
    pub(crate) chat_message: Option<Entity<TextareaState>>,
    /// The dot the dot page's composer is set up for (placeholder and Enter).
    pub(crate) chat_placeholder_dot: Option<String>,
    pub(crate) answers: HashMap<String, Entity<InputState>>,
    pub(crate) setup: Option<SetupForm>,
    pub(crate) rules: HashMap<Option<String>, RuleForm>,
    pub(crate) routine: Option<RoutineForm>,
    pub(crate) memory: Option<Entity<InputState>>,
    pub(crate) new_dot: Option<NewDotForm>,
    pub(crate) password: Option<PasswordForm>,
    pub(crate) triggers_key: Option<TriggersKeyForm>,
    pub(crate) team: Option<TeamForm>,
    pub(crate) trigger_flow: Option<TriggerFlow>,
    pub(crate) url: Option<Entity<InputState>>,
    pub(crate) url_subscription: Option<Subscription>,
    pub(crate) pending_clears: Vec<PendingClear>,
    /// Enter-to-submit subscriptions, by field, so a field made again replaces its own.
    pub(crate) subscriptions: HashMap<String, Subscription>,
}

pub(crate) struct SetupForm {
    pub(crate) dot_id: String,
    pub(crate) name: Entity<InputState>,
    pub(crate) purpose: Entity<InputState>,
    pub(crate) instructions: Entity<TextareaState>,
    pub(crate) look: DotLook,
    /// What was last saved, to tell whether Save has anything to do.
    pub(crate) saved: (String, String, String, DotLook),
}

pub(crate) struct RuleForm {
    pub(crate) action: Entity<InputState>,
    pub(crate) decision: &'static str,
}

pub(crate) struct RoutineForm {
    pub(crate) dot_id: String,
    pub(crate) name: Entity<InputState>,
    pub(crate) instruction: Entity<InputState>,
    pub(crate) schedule: Entity<InputState>,
    pub(crate) error: Option<String>,
}

pub(crate) struct NewDotForm {
    pub(crate) name: Entity<InputState>,
    pub(crate) purpose: Entity<TextareaState>,
    pub(crate) look: DotLook,
    pub(crate) placeholder: &'static str,
    pub(crate) agent_id: Option<String>,
    pub(crate) model: Option<String>,
}

pub(crate) struct PasswordForm {
    pub(crate) site: Entity<InputState>,
    pub(crate) username: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
    pub(crate) error: Option<String>,
}

pub(crate) struct TriggersKeyForm {
    pub(crate) key: Entity<InputState>,
    pub(crate) editing: bool,
    pub(crate) error: Option<String>,
}

pub(crate) struct TeamForm {
    pub(crate) name: Entity<InputState>,
    pub(crate) message: Entity<TextareaState>,
    pub(crate) members: Vec<String>,
    pub(crate) lead: Option<String>,
}

/// Open Dot's add-trigger flow: app, then event, then the event's fields and the instruction.
pub(crate) struct TriggerFlow {
    pub(crate) dot_id: String,
    pub(crate) apps: Option<Result<Vec<TriggerApp>, String>>,
    pub(crate) app: Option<TriggerApp>,
    pub(crate) types: Option<Result<Vec<TriggerType>, String>>,
    pub(crate) trigger_type: Option<TriggerType>,
    pub(crate) values: Vec<(String, Entity<InputState>)>,
    pub(crate) flags: HashMap<String, bool>,
    pub(crate) instruction: Option<Entity<TextareaState>>,
    pub(crate) error: Option<String>,
}

pub(crate) fn input(
    window: &mut Window,
    cx: &mut Context<NativeOpenDotsView>,
    placeholder: &str,
) -> Entity<InputState> {
    let placeholder = placeholder.to_string();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

pub(crate) fn input_with(
    window: &mut Window,
    cx: &mut Context<NativeOpenDotsView>,
    placeholder: &str,
    value: &str,
) -> Entity<InputState> {
    let placeholder = placeholder.to_string();
    let value = value.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

pub(crate) fn textarea(
    window: &mut Window,
    cx: &mut Context<NativeOpenDotsView>,
    placeholder: &str,
    value: &str,
    min_rows: usize,
    max_rows: usize,
) -> Entity<TextareaState> {
    let placeholder = placeholder.to_string();
    let value = value.to_string();
    cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(min_rows, max_rows)
            .placeholder(placeholder)
            .default_value(value)
    })
}

impl NativeOpenDotsView {
    /// Empties the fields a finished call asked to clear.
    pub(crate) fn run_pending_clears(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for clear in std::mem::take(&mut self.forms.pending_clears) {
            match clear {
                PendingClear::Input(state) => {
                    state.update(cx, |state, cx| state.set_value("", window, cx))
                }
                PendingClear::Textarea(state) => {
                    state.update(cx, |state, cx| state.set_value("", window, cx))
                }
            }
        }
    }

    /// Runs `on_enter` when Enter is pressed in `state` (a single-line field, or a text area made
    /// with `submit_on_enter`).
    pub(crate) fn submit_on_enter<T: 'static>(
        &mut self,
        key: impl Into<String>,
        state: &Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_enter: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) where
        T: gpui::EventEmitter<InputEvent>,
    {
        let subscription = cx.subscribe_in(
            state,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    on_enter(this, window, cx);
                }
            },
        );
        self.forms.subscriptions.insert(key.into(), subscription);
    }

    pub(crate) fn ensure_setup_form(
        &mut self,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .forms
            .setup
            .as_ref()
            .is_some_and(|form| form.dot_id == dot.id)
        {
            return;
        }
        let look = DotLook::from_value(Some(&dot.look));
        self.forms.setup = Some(SetupForm {
            dot_id: dot.id.clone(),
            name: input_with(window, cx, "Name", &dot.name),
            purpose: input_with(window, cx, "What this dot is for", &dot.purpose),
            instructions: textarea(
                window,
                cx,
                "Tone, preferences, things to always or never do…",
                &dot.instructions,
                4,
                12,
            ),
            look: look.clone(),
            saved: (
                dot.name.clone(),
                dot.purpose.clone(),
                dot.instructions.clone(),
                look,
            ),
        });
        self.forms.routine = None;
        self.forms.memory = None;
        self.forms.rules.remove(&Some(dot.id.clone()));
    }
}
