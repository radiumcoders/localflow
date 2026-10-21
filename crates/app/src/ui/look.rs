//! The look of localflow's windows, shared with localflow.radiumcoders.com:
//! pure black and white, sky blue for what you can act on, Geist, dashed
//! hairlines for structure, raised keys for buttons, and springs for motion.

use std::borrow::Cow;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lyon::tessellation::{LineCap, LineJoin, StrokeOptions};

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";

const FONTS: [&[u8]; 5] = [
    include_bytes!("../../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../../assets/fonts/GeistMono-Regular.ttf"),
];

/// Load Geist; every window draws with it.
pub fn init(cx: &mut App) {
    if let Err(e) = cx.text_system().add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
        eprintln!("fonts: {e:#}");
    }
}

/// The site's springs: `SPRING` for things that move, `SOFT` for things that settle.
pub const SPRING: SpringConfig = SpringConfig::new(420., 34., 0.8);
pub const SOFT: SpringConfig = SpringConfig::new(260., 30., 1.);

pub fn spring(config: SpringConfig, to: f32) -> SpringAnimation<f32> {
    SpringAnimation::new(config).to(to)
}


fn alpha(c: u32, a: f32) -> Hsla {
    let mut h: Hsla = rgb(c).into();
    h.a = a;
    h
}

/// The colours, for the window's light or dark appearance.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub fg: Hsla,
    /// Secondary text.
    pub dim: Hsla,
    /// Text that's further back still (upcoming steps, placeholders).
    pub faint: Hsla,
    /// Tonal fills: inputs, code, hovered cells.
    pub muted: Hsla,
    pub hover: Hsla,
    /// The dashed hairlines.
    pub rail: Hsla,
    pub primary: Hsla,
    pub primary_hi: Hsla,
    pub primary_edge: Hsla,
    pub primary_fg: Hsla,
    /// The chosen cell's faint wash, and blue tints.
    pub tint: Hsla,
    pub tint_strong: Hsla,
    pub record: Hsla,
    /// Raised keys.
    pub key: Hsla,
    pub key_edge: Hsla,
    pub key_highlight: Hsla,
    pub key_drop: Hsla,
}

impl Palette {
    pub fn of(window: &Window) -> Palette {
        // A debug build can preview the other theme: LOCALFLOW_PREVIEW_THEME=light|dark.
        if cfg!(debug_assertions) {
            match std::env::var("LOCALFLOW_PREVIEW_THEME").as_deref() {
                Ok("light") => return Palette::LIGHT,
                Ok("dark") => return Palette::DARK,
                _ => {}
            }
        }
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Palette::DARK,
            _ => Palette::LIGHT,
        }
    }

    pub const DARK: Palette = Palette {
        bg: Hsla { h: 0., s: 0., l: 0., a: 1. },
        fg: Hsla { h: 0., s: 0., l: 1., a: 1. },
        dim: Hsla { h: 240. / 360., s: 0.05, l: 0.65, a: 1. },
        faint: Hsla { h: 240. / 360., s: 0.05, l: 0.65, a: 0.55 },
        muted: Hsla { h: 0., s: 0., l: 0.09, a: 1. },
        hover: Hsla { h: 0., s: 0., l: 1., a: 0.035 },
        rail: Hsla { h: 0., s: 0., l: 1., a: 0.18 },
        primary: Hsla { h: 198.4 / 360., s: 0.93, l: 0.6, a: 1. },
        primary_hi: Hsla { h: 198.4 / 360., s: 0.93, l: 0.67, a: 1. },
        primary_edge: Hsla { h: 198.4 / 360., s: 0.65, l: 0.47, a: 1. },
        primary_fg: Hsla { h: 0., s: 0., l: 0., a: 1. },
        tint: Hsla { h: 198.4 / 360., s: 0.93, l: 0.6, a: 0.05 },
        tint_strong: Hsla { h: 198.4 / 360., s: 0.93, l: 0.6, a: 0.14 },
        record: Hsla { h: 0., s: 0.91, l: 0.71, a: 1. },
        key: Hsla { h: 0., s: 0., l: 0.09, a: 1. },
        key_edge: Hsla { h: 0., s: 0., l: 0., a: 0.6 },
        key_highlight: Hsla { h: 0., s: 0., l: 1., a: 0.08 },
        key_drop: Hsla { h: 0., s: 0., l: 0., a: 0.5 },
    };

    pub const LIGHT: Palette = Palette {
        bg: Hsla { h: 0., s: 0., l: 1., a: 1. },
        fg: Hsla { h: 0., s: 0., l: 0., a: 1. },
        dim: Hsla { h: 240. / 360., s: 0.04, l: 0.46, a: 1. },
        faint: Hsla { h: 240. / 360., s: 0.04, l: 0.46, a: 0.55 },
        muted: Hsla { h: 240. / 360., s: 0.05, l: 0.96, a: 1. },
        hover: Hsla { h: 0., s: 0., l: 0., a: 0.025 },
        rail: Hsla { h: 0., s: 0., l: 0., a: 0.2 },
        primary: Hsla { h: 198.6 / 360., s: 0.89, l: 0.48, a: 1. },
        primary_hi: Hsla { h: 198.6 / 360., s: 0.89, l: 0.56, a: 1. },
        primary_edge: Hsla { h: 198.6 / 360., s: 0.89, l: 0.38, a: 1. },
        primary_fg: Hsla { h: 0., s: 0., l: 1., a: 1. },
        tint: Hsla { h: 198.6 / 360., s: 0.89, l: 0.48, a: 0.04 },
        tint_strong: Hsla { h: 198.6 / 360., s: 0.89, l: 0.48, a: 0.12 },
        record: Hsla { h: 0., s: 0.84, l: 0.6, a: 1. },
        key: Hsla { h: 0., s: 0., l: 1., a: 1. },
        key_edge: Hsla { h: 0., s: 0., l: 0., a: 0.12 },
        key_highlight: Hsla { h: 0., s: 0., l: 1., a: 0.9 },
        key_drop: Hsla { h: 0., s: 0., l: 0., a: 0.08 },
    };
}

/// A dashed hairline (dashes of 10, gaps of 6), as SVG would draw it, so the
/// dashes keep their length at any size.
pub fn dashes(vertical: bool, color: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let mut path = PathBuilder::stroke(px(1.)).dash_array(&[px(10.), px(6.)]);
            if vertical {
                path.move_to(point(b.origin.x + px(0.5), b.origin.y));
                path.line_to(point(b.origin.x + px(0.5), b.bottom()));
            } else {
                path.move_to(point(b.origin.x, b.origin.y + px(0.5)));
                path.line_to(point(b.right(), b.origin.y + px(0.5)));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .when(vertical, |c| c.w(px(1.)).h_full())
    .when(!vertical, |c| c.h(px(1.)).w_full())
}

/// A faint dot grid that fades out toward the edges, behind a stage.
pub fn dot_grid(color: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
            let step = 18.;
            let mut y = step / 2.;
            while y < h {
                let mut x = (w % step) / 2.;
                while x < w {
                    let dx = (x - w / 2.) / (w / 2.);
                    let dy = (y - h / 2.) / (h / 2.);
                    let fade = (1. - (dx * dx + dy * dy).sqrt() / 0.9).clamp(0., 1.);
                    if fade > 0. {
                        let mut c = color;
                        c.a *= fade * 0.6;
                        window.paint_quad(fill(
                            Bounds::new(point(b.origin.x + px(x), b.origin.y + px(y)), size(px(1.), px(1.))),
                            c,
                        ));
                    }
                    x += step;
                }
                y += step;
            }
        },
    )
    .absolute()
    .inset_0()
}

fn key_shadow(p: &Palette) -> Vec<BoxShadow> {
    vec![
        BoxShadow { color: p.key_highlight, offset: point(px(0.), px(1.)), blur_radius: px(0.), spread_radius: px(0.), inset: true },
        BoxShadow { color: p.key_edge, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
        BoxShadow { color: p.key_drop, offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false },
        BoxShadow { color: p.key_drop, offset: point(px(0.), px(2.)), blur_radius: px(6.), spread_radius: px(-1.), inset: false },
    ]
}

/// A paper key with a hairline edge: neutral buttons, chips, keycaps.
pub fn raised<E: Styled>(e: E, p: &Palette) -> E {
    e.bg(p.key).shadow(key_shadow(p))
}

/// The sky blue button, lit from above, with a soft glow underneath.
pub fn primary_button(id: impl Into<ElementId>, label: impl Into<SharedString>, enabled: bool, p: &Palette) -> Stateful<Div> {
    let mut glow = p.primary;
    glow.a = 0.5;
    div()
        .id(id)
        .h(px(36.))
        .min_w(px(112.))
        .px(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(p.primary_edge)
        .bg(linear_gradient(180., linear_color_stop(p.primary_hi, 0.), linear_color_stop(p.primary, 0.6)))
        .shadow(vec![
            BoxShadow { color: alpha(0xffffff, 0.32), offset: point(px(0.), px(1.)), blur_radius: px(0.), spread_radius: px(0.), inset: true },
            BoxShadow { color: alpha(0x000000, 0.12), offset: point(px(0.), px(-1.)), blur_radius: px(0.), spread_radius: px(0.), inset: true },
            BoxShadow { color: alpha(0x000000, 0.18), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false },
            BoxShadow { color: glow, offset: point(px(0.), px(4.)), blur_radius: px(12.), spread_radius: px(-3.), inset: false },
        ])
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.primary_fg)
        .whitespace_nowrap()
        .child(label.into())
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(p.primary_hi)))
        .when(!enabled, |d| d.opacity(0.45))
}

/// A raised neutral button.
pub fn secondary_button(id: impl Into<ElementId>, label: impl Into<SharedString>, enabled: bool, p: &Palette) -> Stateful<Div> {
    let hover = p.muted;
    raised(div().id(id), p)
        .h(px(32.))
        .px(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.fg)
        .whitespace_nowrap()
        .child(label.into())
        .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
        .when(!enabled, |d| d.opacity(0.5))
}

/// A quiet text button.
pub fn ghost_button(id: impl Into<ElementId>, label: impl Into<SharedString>, p: &Palette) -> Stateful<Div> {
    let (fg, hover) = (p.fg, p.muted);
    div()
        .id(id)
        .h(px(36.))
        .px(px(16.))
        .flex()
        .items_center()
        .rounded_full()
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.dim)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(label.into())
}

/// The blue line along the top of a chosen cell: it springs out from the left.
pub fn chosen_line(id: impl Into<ElementId>, on: bool, p: &Palette) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .h(px(1.))
        .bg(p.primary)
        .with_spring(id, spring(SOFT, if on { 1. } else { 0. }), |el, v| el.w(relative(v.max(0.))))
}

/// Measure a line of text, for things that must know their width up front.
pub fn text_width(text: &str, size: f32, weight: FontWeight, window: &Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: Font { weight, ..font(SANS) },
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(window.text_system().layout_line(text, px(size), &[run], None).width)
}

/// Small pills with a raised key that slides under the chosen one.
pub fn chips<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, SharedString)],
    value: T,
    on_pick: impl Fn(T, &mut Window, &mut App) + 'static,
    p: &Palette,
    window: &Window,
) -> AnyElement {
    const PAD: f32 = 12.;
    const GAP: f32 = 2.;
    let widths: Vec<f32> = options.iter().map(|(_, l)| text_width(l, 13., FontWeight::MEDIUM, window) + PAD * 2.).collect();
    let found = options.iter().position(|(v, _)| *v == value);
    let chosen = found.unwrap_or(0);
    let left: f32 = widths[..chosen].iter().map(|w| w + GAP).sum();
    let total: f32 = widths.iter().sum::<f32>() + GAP * (widths.len().saturating_sub(1)) as f32;
    let on_pick = std::rc::Rc::new(on_pick);
    let (fg, dim) = (p.fg, p.dim);
    let indicator = raised(div(), p).h(px(28.)).rounded_full().when(found.is_none(), |d| d.opacity(0.)).with_spring((id, 0usize), spring(SPRING, widths[chosen]), |el, w| el.w(px(w)));
    div()
        .relative()
        .flex_none()
        .w(px(total))
        .h(px(28.))
        .child(div().absolute().top_0().h(px(28.)).with_spring((id, 1usize), spring(SPRING, left), |el, x| el.left(px(x))).child(indicator))
        .child(div().relative().flex().gap(px(GAP)).children(options.iter().enumerate().map(|(i, (v, label))| {
            let v = *v;
            let on_pick = on_pick.clone();
            let selected = found == Some(i);
            div()
                .id((id, i + 2))
                .w(px(widths[i]))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap()
                .text_color(if selected { fg } else { dim })
                .cursor_pointer()
                .hover(move |s| s.text_color(fg))
                .on_click(move |_, window, cx| on_pick(v, window, cx))
                .child(label.clone())
        })))
        .into_any_element()
}

/// A row of cells between dashed lines, split by dashed lines.
pub fn band(cells: Vec<AnyElement>, p: &Palette) -> Div {
    let rail = p.rail;
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(dashes(false, rail))
        .child(div().flex().w_full().children(cells.into_iter().enumerate().map(|(i, cell)| {
            div()
                .relative()
                .flex()
                .flex_1()
                .flex_basis(relative(0.))
                .min_w_0()
                .when(i > 0, |d| d.child(div().absolute().top_0().bottom_0().left_0().child(dashes(true, rail))))
                .child(cell)
        })))
        .child(dashes(false, rail))
}

/// The logo, drawn in as `t` runs from 0 to 1: the viewfinder corners, the
/// capsule, its cradle and stand, then the grille, over dashed rails that
/// stop behind the capsule.
pub fn logo(px_size: f32, t: f32, p: &Palette) -> impl IntoElement {
    let (blue, fg, rail) = (p.primary, p.fg, p.rail);
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let s = px_size / 344.;
            let at = |x: f32, y: f32| point(b.origin.x + px((x - 84.) * s), b.origin.y + px((y - 84.) * s));
            let ease = |x: f32| if x < 0.5 { 4. * x * x * x } else { 1. - (-2. * x + 2.).powi(3) / 2. };
            // Time in seconds into the 1.1 s draw, and each stroke's share of it.
            let now = t * 1.1;
            let reveal = |delay: f32| ease(((now - delay) / 0.6).clamp(0., 1.));

            let rails_on = (now / 0.5).clamp(0., 1.);
            if rails_on > 0. {
                let mut color = rail;
                color.a *= rails_on;
                let mut path = PathBuilder::stroke(px(4. * s)).dash_array(&[px(20. * s), px(12. * s)]);
                for (a, z) in [((256., 84.), (256., 110.)), ((256., 290.), (256., 428.)), ((84., 210.), (204., 210.)), ((308., 210.), (428., 210.))] {
                    path.move_to(at(a.0, a.1));
                    path.line_to(at(z.0, z.1));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }

            let stroke = |width: f32, length: f32, shown: f32| {
                let options = StrokeOptions::default()
                    .with_line_width(width * s)
                    .with_line_cap(LineCap::Round)
                    .with_line_join(LineJoin::Round);
                let builder = PathBuilder::stroke(px(width * s)).with_style(PathStyle::Stroke(options));
                if shown < 1. { builder.dash_array(&[px(length * s * shown), px(length * s + 1.)]) } else { builder }
            };
            let mut draw = |delay: f32, width: f32, length: f32, color: Hsla, trace: &dyn Fn(&mut PathBuilder)| {
                let shown = reveal(delay);
                if shown <= 0.01 {
                    return;
                }
                let mut path = stroke(width, length, shown);
                trace(&mut path);
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };

            let corners = [
                [(100., 156.), (100., 100.), (156., 100.)],
                [(356., 100.), (412., 100.), (412., 156.)],
                [(412., 356.), (412., 412.), (356., 412.)],
                [(156., 412.), (100., 412.), (100., 356.)],
            ];
            for (i, c) in corners.iter().enumerate() {
                draw(0.05 + i as f32 * 0.05, 18., 112., blue, &|path| {
                    path.move_to(at(c[0].0, c[0].1));
                    path.line_to(at(c[1].0, c[1].1));
                    path.line_to(at(c[2].0, c[2].1));
                });
            }
            let r = px(52. * s);
            draw(0.2, 18., 479., blue, &|path| {
                path.move_to(at(204., 162.));
                path.line_to(at(204., 238.));
                path.arc_to(point(r, r), px(0.), false, false, at(308., 238.));
                path.line_to(at(308., 162.));
                path.arc_to(point(r, r), px(0.), false, false, at(204., 162.));
            });
            let r = px(94. * s);
            draw(0.3, 18., 327., blue, &|path| {
                path.move_to(at(162., 216.));
                path.line_to(at(162., 232.));
                path.arc_to(point(r, r), px(0.), false, false, at(350., 232.));
                path.line_to(at(350., 216.));
            });
            draw(0.45, 18., 56., blue, &|path| {
                path.move_to(at(256., 326.));
                path.line_to(at(256., 382.));
            });
            draw(0.5, 18., 104., blue, &|path| {
                path.move_to(at(204., 386.));
                path.line_to(at(308., 386.));
            });
            for (i, y) in [166., 196., 226.].into_iter().enumerate() {
                draw(0.45 + i as f32 * 0.06, 11., 36., fg, &|path| {
                    path.move_to(at(238., y));
                    path.line_to(at(274., y));
                });
            }
        },
    )
    .size(px(px_size))
}


/// Give gpui-kit's own components (inputs, selects, switches, popovers) the
/// site's colours, for the current light or dark mode. Run it again after
/// the mode changes: the kit reloads its theme then.
pub fn apply_theme(cx: &mut App) {
    use gpui_kit::component::theme::Theme;
    let theme = Theme::global_mut(cx);
    let dark = theme.mode.is_dark();
    let p = if dark { Palette::DARK } else { Palette::LIGHT };
    let solid = |l: f32| Hsla { h: 0., s: 0., l, a: 1. };
    let border = if dark { solid(0.16) } else { solid(0.89) };
    let popover = if dark { solid(0.04) } else { solid(1.) };
    let mut selection = p.primary;
    selection.a = 0.3;
    theme.font_family = SANS.into();
    theme.mono_font_family = MONO.into();
    theme.radius = px(10.);
    let c = &mut theme.colors;
    c.background = p.bg;
    c.foreground = p.fg;
    c.muted = p.muted;
    c.muted_foreground = p.dim;
    c.border = border;
    c.input = border;
    c.ring = p.primary;
    c.caret = p.primary;
    c.selection = selection;
    c.primary = p.primary;
    c.primary_hover = p.primary_hi;
    c.primary_active = p.primary_edge;
    c.primary_foreground = p.primary_fg;
    c.secondary = p.muted;
    c.secondary_hover = p.hover;
    c.secondary_foreground = p.fg;
    c.accent = p.muted;
    c.accent_foreground = p.fg;
    c.popover = popover;
    c.popover_foreground = p.fg;
    c.list_hover = p.muted;
    c.list_active = p.tint_strong;
    c.list_active_border = p.primary;
    c.switch = if dark { solid(0.22) } else { solid(0.84) };
    c.switch_thumb = solid(1.);
    c.success = p.primary;
    c.danger = p.record;
    c.link = p.primary;
    c.progress_bar = p.primary;
    c.window_border = border;
    // Components read the tokens; make them the colors just set.
    theme.tokens = (&theme.colors).into();
    Theme::sync_base(cx);
    cx.refresh_windows();
}

/// Keycaps, as (icon, label, wide).
pub type Keys = &'static [(&'static str, &'static str, bool)];

/// A keycap: an icon (or nothing) at the top, the key's name below. Held,
/// it goes down and lights up.
pub fn keycap(glyph: &str, label: &str, wide: bool, held: bool, p: &Palette) -> impl IntoElement {
    let ink = if held { p.primary } else { p.fg.opacity(0.8) };
    let key = div()
        .relative()
        .h(px(48.))
        .min_w(px(48.))
        .when(wide, |d| d.w(px(144.)))
        .px(px(8.))
        .py(px(6.))
        .flex()
        .flex_col()
        .justify_between()
        .rounded(px(8.))
        .child(div().h(px(14.)).when(!glyph.is_empty(), |d| d.child(svg().path(SharedString::from(format!("icons/{glyph}.svg"))).size(px(13.)).flex_none().text_color(ink))))
        .child(
            div()
                .when(wide, |d| d.mx_auto())
                .text_size(px(if glyph.is_empty() { 13. } else { 11. }))
                .line_height(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(label.to_string()),
        );
    let key = if held {
        key.bg(p.tint_strong).border_1().border_color(p.primary.opacity(0.4)).text_color(p.primary)
    } else {
        raised(key, p).text_color(p.fg.opacity(0.8))
    };
    key.with_spring(SharedString::from(format!("key-{label}")), spring(SPRING, if held { 2. } else { 0. }), |el, y| el.top(px(y)))
}

