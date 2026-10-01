//! A dot's face (OpenDots) as a GPUI image: gx-core draws the SVG (`dot_face_svg`), GPUI rasterizes
//! it. Shared by the sidebar's dot rows and the OpenDots view, and compiled into the web build.
//! SEE-ALSO: packages/gx-core/src/open_dots.rs (the face itself), apps/desktop/src/app/native_open_dots/.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ghostex_gx_core::open_dots::{DotFaceStatus, DotLook, dot_face_svg, dots_mark_svg};
use gpui::{Image, ImageFormat};
use serde_json::Value;

const CACHE_LIMIT: usize = 256;

#[derive(Default)]
struct FaceCache {
    images: HashMap<String, Arc<Image>>,
    order: VecDeque<String>,
}

thread_local! {
    static FACES: RefCell<FaceCache> = RefCell::default();
}

/// Rasterizing at 1:1 needs the SVG drawn at the pixel size the screen shows, so sizes snap up to
/// a few buckets (at 2x for Retina) and the element scales the picture down.
fn bucket(display_px: f32) -> u32 {
    let wanted = (display_px.max(1.0) * 2.0).ceil() as u32;
    [32, 48, 64, 96, 128, 192, 256, 384, 512]
        .into_iter()
        .find(|size| *size >= wanted)
        .unwrap_or(512)
}

fn cached(key: String, draw: impl FnOnce() -> String) -> Arc<Image> {
    FACES.with_borrow_mut(|cache| {
        if let Some(image) = cache.images.get(&key) {
            return image.clone();
        }
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, draw().into_bytes()));
        while cache.images.len() >= CACHE_LIMIT {
            let Some(oldest) = cache.order.pop_front() else {
                break;
            };
            cache.images.remove(&oldest);
        }
        cache.order.push_back(key.clone());
        cache.images.insert(key, image.clone());
        image
    })
}

/// The face for a wire look (`dotLook`, any shape; normalized like Open Dot), shown `display_px`
/// points wide.
pub(crate) fn dot_face_image(
    look: Option<&Value>,
    status: DotFaceStatus,
    display_px: f32,
) -> Arc<Image> {
    dot_face_image_for(&DotLook::from_value(look), status, display_px)
}

pub(crate) fn dot_face_image_for(
    look: &DotLook,
    status: DotFaceStatus,
    display_px: f32,
) -> Arc<Image> {
    let size = bucket(display_px);
    let key = format!(
        "face|{}|{}|{}|{}|{}|{}|{}|{:?}|{size}",
        look.shape,
        look.color,
        look.accent,
        look.eye_color,
        look.material,
        look.eyes,
        look.accessory,
        status
    );
    cached(key, || dot_face_svg(look, status, size))
}

/// The dots wordmark (three circles) in `color` (`#rrggbb`), for the Space-row slot and toggle.
pub(crate) fn dots_mark_image(color_hex: &str, display_px: f32) -> Arc<Image> {
    let size = bucket(display_px);
    cached(format!("mark|{color_hex}|{size}"), || {
        dots_mark_svg(size, color_hex)
    })
}
