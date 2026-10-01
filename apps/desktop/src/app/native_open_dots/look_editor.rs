//! The dot's look editor (Open Dot's `LookEditor.tsx`): body shape, body colour with its paired
//! feet, feet, eyes, eye colour, finish and an extra, plus Shuffle. Used by New dot and by a dot's
//! Setup, each editing its own form's look.

use super::model::now_ms;
use super::style::*;
use super::view::NativeOpenDotsView;
use ghostex_gx_core::open_dots::{
    DOT_ACCESSORIES, DOT_COLORS, DOT_EYE_COLORS, DOT_EYES, DOT_FEET_COLORS, DOT_MATERIALS,
    DOT_SHAPES, DotLook, dot_feet_for,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Context, IntoElement, ParentElement as _, Styled as _, div, px};
use gpui_component::{h_flex, v_flex};

/// Which form's look the editor changes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LookTarget {
    NewDot,
    Setup,
}

/// Open Dot's `randomLook`: never the toon finish or the wink, which read as glitches at random.
pub(crate) fn random_look() -> DotLook {
    let mut seed = (now_ms() as u64) ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = |count: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % count as u64) as usize
    };
    let color = DOT_COLORS[next(DOT_COLORS.len())];
    let materials: Vec<&str> = DOT_MATERIALS
        .iter()
        .copied()
        .filter(|m| *m != "toon")
        .collect();
    let eyes: Vec<&str> = DOT_EYES.iter().copied().filter(|e| *e != "wink").collect();
    DotLook {
        shape: DOT_SHAPES[next(DOT_SHAPES.len())].to_string(),
        color: color.to_string(),
        accent: dot_feet_for(color).to_string(),
        eye_color: DOT_EYE_COLORS[next(DOT_EYE_COLORS.len())].to_string(),
        material: materials[next(materials.len())].to_string(),
        eyes: eyes[next(eyes.len())].to_string(),
        accessory: DOT_ACCESSORIES[next(DOT_ACCESSORIES.len())].to_string(),
    }
}

fn capitalized(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

impl NativeOpenDotsView {
    pub(crate) fn look_mut(&mut self, target: LookTarget) -> Option<&mut DotLook> {
        match target {
            LookTarget::NewDot => self.forms.new_dot.as_mut().map(|form| &mut form.look),
            LookTarget::Setup => self.forms.setup.as_mut().map(|form| &mut form.look),
        }
    }

    fn edit_look(
        &mut self,
        target: LookTarget,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut DotLook),
    ) {
        if let Some(look) = self.look_mut(target) {
            edit(look);
            cx.notify();
        }
    }

    pub(crate) fn render_look_editor(
        &mut self,
        p: &DotsPalette,
        target: LookTarget,
        look: &DotLook,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let prefix = match target {
            LookTarget::NewDot => "dots-new-look",
            LookTarget::Setup => "dots-setup-look",
        };
        let row = |label: &'static str, control: AnyElement| {
            h_flex()
                .w_full()
                .min_w_0()
                .gap(px(12.0))
                .items_start()
                .py(px(10.0))
                .child(eyebrow(p, label).w(px(80.0)).flex_shrink_0().pt(px(7.0)))
                .child(div().flex_1().min_w_0().child(control))
                .into_any_element()
        };
        let chips = |field: &'static str,
                     options: &[&'static str],
                     selected: &str,
                     cx: &mut Context<Self>|
         -> AnyElement {
            h_flex()
                .flex_wrap()
                .gap(px(6.0))
                .children(options.iter().map(|option| {
                    let option = *option;
                    chip(
                        p,
                        gpui::SharedString::from(format!("{prefix}-{field}-{option}")),
                        capitalized(option),
                        None,
                        option == selected,
                        move |this: &mut Self, _, cx| {
                            this.edit_look(target, cx, |look| match field {
                                "shape" => look.shape = option.to_string(),
                                "eyes" => look.eyes = option.to_string(),
                                "material" => look.material = option.to_string(),
                                _ => look.accessory = option.to_string(),
                            })
                        },
                        cx,
                    )
                }))
                .into_any_element()
        };
        let swatches = |field: &'static str,
                        palette: &[&'static str],
                        selected: &str,
                        cx: &mut Context<Self>|
         -> AnyElement {
            h_flex()
                .flex_wrap()
                .gap(px(6.0))
                .pt(px(1.0))
                .children(palette.iter().map(|color| {
                    let color = *color;
                    swatch(
                        p,
                        gpui::SharedString::from(format!("{prefix}-{field}-{color}")),
                        color,
                        color.eq_ignore_ascii_case(selected),
                        move |this: &mut Self, _, cx| {
                            this.edit_look(target, cx, |look| match field {
                                // A palette colour comes with its matching feet.
                                "color" => {
                                    look.color = color.to_string();
                                    look.accent = dot_feet_for(color).to_string();
                                }
                                "accent" => look.accent = color.to_string(),
                                _ => look.eye_color = color.to_string(),
                            })
                        },
                        cx,
                    )
                }))
                .into_any_element()
        };
        let rows = vec![
            row("Body", chips("shape", &DOT_SHAPES, &look.shape, cx)),
            row("Color", swatches("color", &DOT_COLORS, &look.color, cx)),
            row(
                "Feet",
                swatches("accent", &DOT_FEET_COLORS, &look.accent, cx),
            ),
            row("Eyes", chips("eyes", &DOT_EYES, &look.eyes, cx)),
            row(
                "Eye color",
                swatches("eye", &DOT_EYE_COLORS, &look.eye_color, cx),
            ),
            row(
                "Finish",
                chips("material", &DOT_MATERIALS, &look.material, cx),
            ),
            row(
                "Extra",
                chips("accessory", &DOT_ACCESSORIES, &look.accessory, cx),
            ),
        ];
        let count = rows.len();
        v_flex()
            .w_full()
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .w_full()
                    .when(index + 1 < count, |this| {
                        this.border_b_1().border_color(p.border)
                    })
                    .child(row)
            }))
            .child(h_flex().pt(px(10.0)).child(button(
                p,
                gpui::SharedString::from(format!("{prefix}-shuffle")),
                Some("Shuffle look".into()),
                Some(ICON_SHUFFLE),
                Tone::Quiet,
                true,
                false,
                move |this: &mut Self, _, cx| {
                    let look = random_look();
                    this.edit_look(target, cx, |current| *current = look);
                },
                cx,
            )))
            .into_any_element()
    }
}
