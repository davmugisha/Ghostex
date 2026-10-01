//! The desktop's OpenDots view (`apps/desktop/src/app/native_open_dots/`), compiled here file by
//! file. Only `host.rs` is the page's own: it is `web_host/open_dots.rs`, which opens the view in
//! the page's work area and answers what the view asks the app to do.
mod computer;
mod forms;
mod look_editor;
mod model;
mod pages;
mod style;
mod target;
mod view;

#[allow(unused_imports)]
pub(crate) use target::{OpenDotsDotTab, OpenDotsTarget};
pub(crate) use view::NativeOpenDotsView;
