//! The "Tachyon" mark in the toolbar, holographic on hover, and the About window it
//! opens, which credits the macOS original first.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, AppContext, Context, FontWeight, Hsla, Rgba, Window, div, prelude::*, px,
    size, svg,
};

use crate::app;
use crate::assets::Icon;
use crate::platform;
use crate::theme::Theme;
use crate::widgets::{Button, fit_window_height, icon, separator, title_bar};

const ORIGINAL_AUTHOR: &str = "sonatina-dream";
const ORIGINAL_URL: &str = "https://github.com/sonatina-dream/Tachyon-Download-Manager";
const WINDOWS_AUTHOR: &str = "thesaikodelik";
const WINDOWS_URL: &str = "https://github.com/thesaikodelik/Tachyon-Download-Manager";
const WORD: &str = "Tachyon";

/// Pearl foil colours: pink, violet, cyan, mint, gold. Deeper on light backgrounds
/// so the word stays readable, paler on dark ones.
const FOIL_LIGHT: [u32; 5] = [0xd0469b, 0x7b5cff, 0x0aa3c2, 0x19a974, 0xc98a00];
const FOIL_DARK: [u32; 5] = [0xff8ad8, 0xb3a1ff, 0x6fe3ff, 0x7dffc4, 0xffd86b];

fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xff) as f32 / 255., ((hex >> 8) & 0xff) as f32 / 255., (hex & 0xff) as f32 / 255.]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn to_hsla(c: [f32; 3], alpha: f32) -> Hsla {
    Rgba { r: c[0], g: c[1], b: c[2], a: alpha }.into()
}

/// The foil colour at `position` (any real number; the gradient repeats).
fn foil(position: f32, dark: bool) -> [f32; 3] {
    let stops = if dark { &FOIL_DARK } else { &FOIL_LIGHT };
    let scaled = position.rem_euclid(1.) * stops.len() as f32;
    let index = scaled.floor() as usize % stops.len();
    let next = (index + 1) % stops.len();
    mix(rgb(stops[index]), rgb(stops[next]), scaled.fract())
}

/// Colour of the glyph at `x` (0 = first letter, 1 = last) at time `t` of the loop:
/// the foil gradient drifting across the word, brightened where the sheen passes.
fn holo(x: f32, t: f32, dark: bool) -> [f32; 3] {
    // The gradient spans about two thirds of the palette across the word and drifts.
    let base = foil(x * 0.65 - t, dark);
    // A specular band sweeps left to right once per loop, off screen part of the time.
    let sweep = t * 1.8 - 0.4;
    let glow = (-((x - sweep) / 0.16).powi(2)).exp();
    let white = if dark { [1., 1., 1.] } else { [1., 0.98, 1.] };
    mix(base, white, glow * if dark { 0.75 } else { 0.55 })
}

#[derive(Default)]
pub struct Brand {
    hovered: bool,
}

impl Brand {
    #[cfg(feature = "dev-snapshots")]
    pub(crate) fn dev_set(&mut self, hovered: bool, _open: bool, cx: &mut Context<Self>) {
        self.hovered = hovered;
        cx.notify();
    }
}

/// One glyph of the holographic word: the letter over faint cyan and magenta copies,
/// a hair apart, like the colour fringes of real foil.
fn foil_glyph(letter: char, color: [f32; 3], dark: bool) -> impl IntoElement {
    let fringe = if dark { 0.45 } else { 0.35 };
    let ghost = |offset: f32, tint: [f32; 3]| {
        div()
            .absolute()
            .top_0()
            .left(px(offset))
            .text_color(to_hsla(tint, fringe))
            .child(letter.to_string())
    };
    div()
        .relative()
        .child(ghost(-0.6, [0.0, 0.85, 1.0]))
        .child(ghost(0.6, [1.0, 0.2, 0.8]))
        .child(div().relative().text_color(to_hsla(color, 1.)).child(letter.to_string()))
}

impl Render for Brand {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let dark = theme.dark;
        let letters: Vec<char> = WORD.chars().collect();
        let last = (letters.len() - 1) as f32;
        let bolt = svg().path(Icon::Bolt.path()).size(px(18.)).flex_none();
        let word = div().flex().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD);
        let base = div()
            .id("brand")
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(32.))
            .px(px(8.))
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_click(|_, _, cx| show_about(cx));

        if !self.hovered {
            return base
                .child(bolt.text_color(theme.icon))
                .child(word.text_color(theme.icon).child(WORD))
                .into_any_element();
        }
        if !platform::animations_enabled() {
            // Animations are off in Windows: the foil, standing still.
            return base
                .child(bolt.text_color(to_hsla(holo(-0.15, 0.6, dark), 1.)))
                .child(word.children(
                    letters.iter().enumerate().map(|(ix, l)| foil_glyph(*l, holo(ix as f32 / last, 0.6, dark), dark)),
                ))
                .into_any_element();
        }
        let cycle = Animation::new(Duration::from_millis(3600)).repeat();
        base.child(bolt.with_animation("holo-bolt", cycle.clone(), move |bolt, t| {
            bolt.text_color(to_hsla(holo(-0.15, t, dark), 1.))
        }))
        .child(word.children(letters.iter().enumerate().map(|(ix, letter)| {
            let letter = *letter;
            let x = ix as f32 / last;
            div()
                .child(foil_glyph(letter, holo(x, 0., dark), dark))
                .with_animation(("holo-letter", ix), cycle.clone(), move |_, t| {
                    div().child(foil_glyph(letter, holo(x, t, dark), dark))
                })
        })))
        .into_any_element()
    }
}

/// Opens the About window in the middle of the screen.
pub fn show_about(cx: &mut App) {
    let bounds = app::centered(size(px(340.), px(280.)), cx);
    app::open_styled(app::dialog_options("About Tachyon", bounds), cx, |_, cx| cx.new(|_| AboutDialog)).ok();
    cx.activate(true);
}

pub struct AboutDialog;

/// A person to credit; clicking opens their GitHub repository.
fn credit_row(id: &'static str, role: &'static str, name: &'static str, url: &'static str, theme: &Theme) -> impl IntoElement {
    let hover = theme.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(theme.radius))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(move |_, _, cx| cx.open_url(url))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(11.)).text_color(theme.muted).child(role))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(name)),
        )
        .child(icon(Icon::External, px(13.), theme.muted))
}

impl Render for AboutDialog {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        div()
            .size_full()
            .bg(theme.window)
            .text_color(theme.text)
            .text_size(px(13.))
            .child(fit_window_height(
                div()
                    .flex()
                    .flex_col()
                    .child(title_bar("About Tachyon", false, window, &theme))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(4.))
                            .pt(px(10.))
                            .pb(px(12.))
                            .child(icon(Icon::Bolt, px(44.), theme.icon))
                            .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child("Tachyon Download Manager"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted)
                                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                            ),
                    )
                    .child(div().px(px(16.)).child(separator(&theme)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(8.))
                            .py(px(8.))
                            .child(credit_row("credit-original", "Created by (macOS)", ORIGINAL_AUTHOR, ORIGINAL_URL, &theme))
                            .child(credit_row("credit-windows", "Windows edition by", WINDOWS_AUTHOR, WINDOWS_URL, &theme)),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .px(px(16.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(theme.border)
                            .child(Button::new("ok", "OK").primary().on_click(|_, window, _| window.remove_window())),
                    ),
            ))
    }
}
