//! The native OpenDots view: Home (hand off a task, approvals, your dots, latest updates, team
//! chats), a dot's page (Chats, Computer, Setup), New dot, New team chat and OpenDots settings,
//! ported from Open Dot's React pages. Opened from the Dots sidebar's OpenDots row and from a dot
//! row's Setup and Computer actions. The GPUI web build compiles every file here but `host.rs`
//! (its own is `apps/gpui-web/src/app/web_host/open_dots.rs`).
//! SEE-ALSO: server/src/open_dots/ (`/api/openDotsRpc`), app/native_sidebar/dots.rs (the sidebar
//! entry points), app/native_sidebar/dot_face.rs (the dot's face).
mod computer;
mod forms;
mod host;
mod look_editor;
mod model;
mod pages;
mod style;
mod target;
mod view;

#[allow(unused_imports)]
pub(crate) use target::{OpenDotsDotTab, OpenDotsTarget};
pub(crate) use view::NativeOpenDotsView;
