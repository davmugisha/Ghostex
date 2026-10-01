//! Triggers (Open Dot's `triggers.ts`): a dot runs an instruction when something happens in one of
//! the user's apps. They run through a Composio developer project, so they need its API key
//! (`COMPOSIO_API_KEY`, else the sealed `composio_api_key` setting); apps used by triggers are
//! connected in that project, separately from Composio For You. Events arrive over the live
//! subscription in `realtime.rs`, so gxserver needs no public URL.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::open_dots::{get_setting, open_open_dots_database, set_setting, vault};
use crate::paths::GxserverPaths;

use super::names::{trigger_app_name, TRIGGER_APPS};
use super::realtime::{self, Status, TriggerMessage};
use super::rest::{ComposioRest, RestError};
use super::{ComposioTriggerEvent, TriggerEventSink};

const KEY_SETTING: &str = "composio_api_key";
const USER_SETTING: &str = "composio_trigger_user";
const KEY_ENV: &str = "COMPOSIO_API_KEY";

pub(crate) const NO_KEY: &str = "Add a Composio API key in OpenDots settings to use triggers.";

fn env_key() -> Option<String> {
    std::env::var(KEY_ENV)
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn api_key(paths: &GxserverPaths) -> Option<String> {
    if let Some(key) = env_key() {
        return Some(key);
    }
    let db = open_open_dots_database(paths).ok()?;
    let sealed = get_setting(&db, KEY_SETTING).ok()??;
    vault::unseal(paths, &sealed).ok()
}

fn client(paths: &GxserverPaths) -> Result<ComposioRest, String> {
    api_key(paths)
        .map(|key| ComposioRest::new(&key))
        .ok_or_else(|| NO_KEY.to_string())
}

/// This install's user in the Composio project, stable so its connections and triggers stay
/// together.
fn user_id(paths: &GxserverPaths) -> Result<String, String> {
    let db = open_open_dots_database(paths).map_err(|error| error.to_string())?;
    if let Some(id) = get_setting(&db, USER_SETTING)
        .map_err(|error| error.to_string())?
        .filter(|id| !id.is_empty())
    {
        return Ok(id);
    }
    let id = format!("ghostex-dots-{}", uuid::Uuid::new_v4());
    set_setting(&db, USER_SETTING, Some(&id)).map_err(|error| error.to_string())?;
    Ok(id)
}

/// Where the Composio project API key came from: `"env"`, `"settings"` or none.
pub(crate) fn triggers_key_source(paths: &GxserverPaths) -> Option<&'static str> {
    if env_key().is_some() {
        return Some("env");
    }
    let db = open_open_dots_database(paths).ok()?;
    get_setting(&db, KEY_SETTING)
        .ok()
        .flatten()
        .map(|_| "settings")
}

/// Checks the key with Composio, then saves it sealed and restarts the live events. Empty removes
/// it. Returns an error message.
pub(crate) fn save_triggers_key(paths: &GxserverPaths, key: &str) -> Option<String> {
    if env_key().is_some() {
        return Some("The Composio API key is set by COMPOSIO_API_KEY.".to_string());
    }
    let key = key.trim();
    let db = match open_open_dots_database(paths) {
        Ok(db) => db,
        Err(error) => return Some(error.to_string()),
    };
    if key.is_empty() {
        stop_trigger_events_keep_wanted();
        return set_setting(&db, KEY_SETTING, None)
            .err()
            .map(|error| error.to_string());
    }
    if let Err(error) = ComposioRest::new(key).list_trigger_types(None, 1) {
        return Some(match error {
            RestError::BadKey(_) => "Composio didn't accept that API key.".to_string(),
            other => {
                let detail = other.detail();
                let lower = detail.to_ascii_lowercase();
                if ["401", "403", "unauthori", "invalid"]
                    .iter()
                    .any(|marker| lower.contains(marker))
                {
                    "Composio didn't accept that API key.".to_string()
                } else {
                    format!("Couldn't check the key with Composio: {detail}")
                }
            }
        });
    }
    let sealed = match vault::seal(paths, key) {
        Ok(sealed) => sealed,
        Err(error) => return Some(format!("Couldn't save the key: {error}")),
    };
    if let Err(error) = set_setting(&db, KEY_SETTING, Some(&sealed)) {
        return Some(error.to_string());
    }
    restart_trigger_events(paths);
    None
}

#[derive(Deserialize)]
struct AccountList {
    #[serde(default)]
    items: Vec<Value>,
}

/// `[{ slug, name, connected }]`: Open Dot's trigger apps plus any connected one, connected first.
pub(crate) fn trigger_apps(paths: &GxserverPaths) -> Result<Vec<Value>, String> {
    let rest = client(paths)?;
    let user = user_id(paths)?;
    let accounts = rest
        .list_active_accounts(&user, None)
        .map_err(|error| error.message())?;
    let connected = serde_json::from_value::<AccountList>(accounts)
        .map(|list| list.items)
        .unwrap_or_default()
        .iter()
        .filter_map(|account| account.pointer("/toolkit/slug").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut slugs = TRIGGER_APPS
        .iter()
        .map(|(slug, _)| (*slug).to_string())
        .collect::<Vec<_>>();
    for slug in &connected {
        if !slugs.contains(slug) {
            slugs.push(slug.clone());
        }
    }
    let mut apps = slugs
        .into_iter()
        .map(|slug| {
            let is_connected = connected.contains(&slug);
            (is_connected, slug)
        })
        .collect::<Vec<_>>();
    apps.sort_by_key(|(is_connected, _)| !*is_connected);
    Ok(apps
        .into_iter()
        .map(|(is_connected, slug)| {
            json!({ "slug": slug, "name": trigger_app_name(&slug), "connected": is_connected })
        })
        .collect())
}

/// The link that connects an app in the Composio project (opens in the user's browser): the app's
/// auth config, or Composio's managed sign-in when the project has none yet.
pub(crate) fn connect_trigger_app(paths: &GxserverPaths, toolkit: &str) -> Result<String, String> {
    let rest = client(paths)?;
    let toolkit = toolkit.trim().to_lowercase();
    let name = trigger_app_name(&toolkit);
    let configs = rest
        .list_auth_configs(&toolkit)
        .map_err(|error| error.message())?;
    let existing = configs
        .pointer("/items/0/id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let auth_config_id = match existing {
        Some(id) => id,
        None => rest
            .create_managed_auth_config(&toolkit, &format!("{toolkit} (Ghostex OpenDots)"))
            .map_err(|error| error.message())?,
    };
    let user = user_id(paths)?;
    let active = rest
        .list_active_accounts(&user, Some(&auth_config_id))
        .map_err(|error| error.message())?;
    if active
        .get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return Err(format!("{name} is already connected for triggers."));
    }
    let link = rest
        .create_link(&auth_config_id, &user)
        .map_err(|error| error.message())?;
    link.get("redirect_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{name} is already connected, or doesn't need a sign-in."))
}

/// A trigger type's config schema. `properties` keeps Composio's field order, which is the order
/// the form shows (serde_json's map would sort it).
#[derive(Deserialize, Default)]
struct TriggerSchema {
    #[serde(default, deserialize_with = "ordered_properties")]
    properties: Vec<(String, Value)>,
    #[serde(default)]
    required: Vec<String>,
}

fn ordered_properties<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(String, Value)>, D::Error> {
    struct Ordered;
    impl<'de> Visitor<'de> for Ordered {
        type Value = Vec<(String, Value)>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = map.next_entry::<String, Value>()? {
                entries.push(entry);
            }
            Ok(entries)
        }
        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }
    }
    deserializer.deserialize_any(Ordered)
}

#[derive(Deserialize)]
struct TriggerTypeItem {
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    config: Option<TriggerSchema>,
}

#[derive(Deserialize)]
struct TriggerTypeList {
    #[serde(default)]
    items: Vec<TriggerTypeItem>,
}

/// One form field from a JSON-schema property (Open Dot's `triggerTypes` mapping).
fn trigger_field(name: &str, property: &Value, required: bool) -> Value {
    let kind = property
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("string");
    let kind = if kind == "array" {
        format!(
            "array:{}",
            property
                .pointer("/items/type")
                .and_then(Value::as_str)
                .unwrap_or("string")
        )
    } else {
        kind.to_string()
    };
    let mut field = json!({
        "name": name,
        "type": kind,
        "title": property
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| name.replace('_', " ")),
        "description": property.get("description").and_then(Value::as_str).unwrap_or_default(),
        "required": required,
    });
    if let Some(options) = property.get("enum").filter(|options| options.is_array()) {
        field["enum"] = options.clone();
    }
    if let Some(default) = property.get("default") {
        field["default"] = default.clone();
    }
    field
}

/// `[{ slug, name, description, fields: [{ name, type, title, description, required, enum?, default? }] }]`.
pub(crate) fn trigger_types(paths: &GxserverPaths, toolkit: &str) -> Result<Vec<Value>, String> {
    let rest = client(paths)?;
    let value = rest
        .list_trigger_types(Some(&toolkit.trim().to_lowercase()), 50)
        .map_err(|error| error.message())?;
    let list = serde_json::from_value::<TriggerTypeList>(value)
        .map_err(|_| "Composio sent an unreadable list of trigger events.".to_string())?;
    Ok(list
        .items
        .into_iter()
        .map(|item| {
            let schema = item.config.unwrap_or_default();
            let fields = schema
                .properties
                .iter()
                .map(|(name, property)| {
                    trigger_field(name, property, schema.required.contains(name))
                })
                .collect::<Vec<_>>();
            json!({
                "slug": item.slug,
                "name": item.name,
                "description": item.description,
                "fields": fields,
            })
        })
        .collect())
}

/// Creates the trigger in Composio and returns its Composio id. Starts the live events if they
/// were wanted and not running.
pub(crate) fn create_trigger(
    paths: &GxserverPaths,
    slug: &str,
    config: &Value,
) -> Result<String, String> {
    let rest = client(paths)?;
    match rest.get_trigger_type(slug) {
        Ok(_) => {}
        Err(RestError::Status(400 | 404, _)) => {
            return Err(format!("Composio has no trigger called {slug}."))
        }
        Err(error) => return Err(error.message()),
    }
    let user = user_id(paths)?;
    let config = if config.is_object() {
        config.clone()
    } else {
        json!({})
    };
    let id = rest
        .upsert_trigger(slug, &user, &config)
        .map_err(|error| error.message())?;
    ensure_trigger_events(paths);
    Ok(id)
}

pub(crate) fn set_trigger_enabled(
    paths: &GxserverPaths,
    composio_id: &str,
    enabled: bool,
) -> Result<(), String> {
    client(paths)?
        .set_trigger_status(composio_id, enabled)
        .map_err(|error| error.message())
}

/// Deletes the trigger in Composio. Already gone there counts as done (Open Dot ignored every
/// failure here; callers should still delete their own record on an error).
pub(crate) fn delete_trigger(paths: &GxserverPaths, composio_id: &str) -> Result<(), String> {
    match client(paths)?.delete_trigger(composio_id) {
        Ok(()) | Err(RestError::Status(404, _)) => Ok(()),
        Err(error) => Err(error.message()),
    }
}

// ---------------------------------------------------------------- live events

struct Events {
    sink: Option<TriggerEventSink>,
    paths: Option<GxserverPaths>,
    /// `start_trigger_events` was called and `stop_trigger_events` wasn't since.
    wanted: bool,
    /// The key the running subscription uses.
    running_key: Option<String>,
    stop: Option<Arc<AtomicBool>>,
}

static EVENTS: Mutex<Events> = Mutex::new(Events {
    sink: None,
    paths: None,
    wanted: false,
    running_key: None,
    stop: None,
});

fn lock_events() -> MutexGuard<'static, Events> {
    EVENTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn stop_running(events: &mut Events) {
    if let Some(stop) = events.stop.take() {
        stop.store(true, Ordering::SeqCst);
    }
    events.running_key = None;
}

/// Turns a subscription message into the event the sink takes, dropping events for another user
/// of the same Composio project (the SDK's `userId` filter).
fn deliver(message: TriggerMessage, user: &str, sink: &TriggerEventSink) {
    if !message.user_id.is_empty() && message.user_id != user {
        return;
    }
    if message.ids.is_empty() {
        return;
    }
    let payload = if message.payload.is_null() {
        json!({})
    } else {
        message.payload
    };
    let sink = sink.clone();
    let event = ComposioTriggerEvent {
        composio_ids: message.ids,
        payload,
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event))).is_err() {
        eprintln!("[open-dots] a trigger event handler panicked");
    }
}

/// Starts the subscription for the current key unless one already runs for it (Open Dot's
/// idempotent `startEvents`).
fn ensure_running(events: &mut Events) {
    let (Some(paths), Some(sink)) = (events.paths.clone(), events.sink.clone()) else {
        return;
    };
    if !events.wanted {
        return;
    }
    let Some(key) = api_key(&paths) else {
        stop_running(events);
        return;
    };
    if events.running_key.as_deref() == Some(key.as_str()) {
        return;
    }
    stop_running(events);
    let user = match user_id(&paths) {
        Ok(user) => user,
        Err(error) => {
            eprintln!("[open-dots] couldn't listen for triggers: {error}");
            return;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    events.stop = Some(stop.clone());
    events.running_key = Some(key.clone());
    let spawned = std::thread::Builder::new()
        .name("open-dots-triggers".to_string())
        .spawn(move || {
            let rest = ComposioRest::new(&key);
            let last_status: Mutex<Option<bool>> = Mutex::new(None);
            let on_status = |status: Status| {
                let mut last = last_status.lock().unwrap_or_else(|p| p.into_inner());
                match status {
                    Status::Subscribed => {
                        if *last != Some(true) {
                            eprintln!("[open-dots] listening for Composio trigger events");
                        }
                        *last = Some(true);
                    }
                    Status::Lost(reason) => {
                        if *last != Some(false) {
                            eprintln!("[open-dots] trigger events interrupted: {reason}");
                        }
                        *last = Some(false);
                    }
                }
            };
            let on_message = |message: TriggerMessage| deliver(message, &user, &sink);
            realtime::run(&rest, &stop, &on_message, &on_status);
        });
    if let Err(error) = spawned {
        eprintln!("[open-dots] couldn't listen for triggers: {error}");
        stop_running(events);
    }
}

/// Starts (or restarts after a key change) the live trigger subscription; idempotent. Events go to
/// `sink` on a background thread; keep the sink quick.
pub(crate) fn start_trigger_events(paths: &GxserverPaths, sink: TriggerEventSink) {
    let mut events = lock_events();
    events.paths = Some(paths.clone());
    events.sink = Some(sink);
    events.wanted = true;
    ensure_running(&mut events);
}

pub(crate) fn stop_trigger_events() {
    let mut events = lock_events();
    events.wanted = false;
    stop_running(&mut events);
}

/// The key was removed: stop listening, but start again when a key is saved.
fn stop_trigger_events_keep_wanted() {
    stop_running(&mut lock_events());
}

/// The key changed: restart with it when events are wanted.
fn restart_trigger_events(paths: &GxserverPaths) {
    let mut events = lock_events();
    if events.paths.is_none() {
        events.paths = Some(paths.clone());
    }
    stop_running(&mut events);
    ensure_running(&mut events);
}

/// A trigger was created: make sure events are flowing.
fn ensure_trigger_events(paths: &GxserverPaths) {
    let mut events = lock_events();
    if events.paths.is_none() {
        events.paths = Some(paths.clone());
    }
    ensure_running(&mut events);
}
