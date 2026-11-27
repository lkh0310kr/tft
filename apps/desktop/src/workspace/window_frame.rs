use crate::assets::icon;
use crate::theme::{Theme, FONT_UI_SMALLER, HEADER_HEIGHT, RIBBON_WIDTH};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, rgba, transparent_black, App, BoxShadow, CursorStyle,
    Decorations, HitboxBehavior, Hsla, MouseButton, Pixels, ResizeEdge, Tiling, Window,
    WindowControlArea,
};

/// The margin around the content when TFT draws the frame: the shadow lives in it
/// and the pointer resizes from it. Obsidian's hidden frame measures 23 px on the
/// sides; gpui's inset is one value, so this is the side margin.
const FRAME_SHADOW: Pixels = px(24.);

/// The frame shadow's gaussian sigma (`blur_radius`). Obsidian fits sigma 13 with a
/// 12 px downward offset; this is as dark at the edge and fades before the margin,
/// since a tail cut off by the margin reads as a band.
const FRAME_SHADOW_BLUR: Pixels = px(11.);

/// How far the frame shadow sits below the content.
const FRAME_SHADOW_OFFSET: Pixels = px(6.);

/// The frame shadow's colour: black at 32 %, Obsidian's at the sides.
const FRAME_SHADOW_COLOR: u32 = 0x00000052;

/// The radius of a window corner TFT draws; Obsidian's hidden frame is about 10 px here.
pub(super) const WINDOW_RADIUS: Pixels = px(10.);

/// Obsidian's `--frame-left-space` on macOS: room for the traffic lights (19, 12)
/// past the ribbon in the window's top-left header.
pub(super) const MAC_FRAME_LEFT_SPACE: f32 = 80.;

/// Which window corners are rounded: each that meets no tiled or maximised edge,
/// when the frame is TFT's. gpui clips children to rectangles, so every element
/// reaching a corner rounds itself with [`WindowCorners::apply`].
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct WindowCorners {
    pub(super) tl: bool,
    pub(super) tr: bool,
    pub(super) bl: bool,
    pub(super) br: bool,
}

/// Per-window frame facts used while painting a workspace tree. The main window and
/// popouts share a Workspace, so these facts travel with the render instead of being
/// cached as mutable Workspace state.
#[derive(Clone, Copy)]
pub(super) struct WindowFrame {
    pub(super) corners: WindowCorners,
    pub(super) frameless: bool,
    fullscreen: bool,
}

impl WindowFrame {
    pub(super) fn of(window: &Window) -> Self {
        Self {
            corners: WindowCorners::of(window),
            frameless: frameless(window),
            fullscreen: window.is_fullscreen(),
        }
    }

    pub(super) fn mac_frame(self) -> bool {
        cfg!(target_os = "macos") && self.frameless
    }

    pub(super) fn left_space(self) -> f32 {
        if !self.mac_frame() || self.fullscreen {
            0.
        } else {
            MAC_FRAME_LEFT_SPACE
        }
    }
}

impl WindowCorners {
    pub(crate) fn of(window: &Window) -> WindowCorners {
        match window.window_decorations() {
            Decorations::Client { tiling } => WindowCorners {
                tl: !(tiling.top || tiling.left),
                tr: !(tiling.top || tiling.right),
                bl: !(tiling.bottom || tiling.left),
                br: !(tiling.bottom || tiling.right),
            },
            Decorations::Server => WindowCorners::default(),
        }
    }

    /// The left edge's corners only, for the ribbon or the left sidebar.
    pub(crate) fn left(self) -> WindowCorners {
        WindowCorners {
            tr: false,
            br: false,
            ..self
        }
    }

    /// The right edge's corners only, for the right sidebar.
    pub(crate) fn right(self) -> WindowCorners {
        WindowCorners {
            tl: false,
            bl: false,
            ..self
        }
    }

    /// The top edge's corners only, for a title row.
    pub(crate) fn top(self) -> WindowCorners {
        WindowCorners {
            bl: false,
            br: false,
            ..self
        }
    }

    pub(crate) fn apply<T: Styled>(self, mut el: T) -> T {
        if self.tl {
            el = el.rounded_tl(WINDOW_RADIUS);
        }
        if self.tr {
            el = el.rounded_tr(WINDOW_RADIUS);
        }
        if self.bl {
            el = el.rounded_bl(WINDOW_RADIUS);
        }
        if self.br {
            el = el.rounded_br(WINDOW_RADIUS);
        }
        el
    }
}

/// Makes `area` a handle on the window when the frame is TFT's: a press drags,
/// a double click maximises or restores, a right click opens the window menu.
/// Windows uses the hit test (`WindowControlArea::Drag`) instead, since asking for
/// both would stop the system dragging at all.
/// A press on a macOS drag area that has not moved yet; a window has one pointer.
static MAC_DRAG_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn window_handle(
    area: gpui::Stateful<gpui::Div>,
    frameless: bool,
) -> gpui::Stateful<gpui::Div> {
    if !frameless {
        return area;
    }
    if cfg!(target_os = "windows") {
        return area.window_control_area(WindowControlArea::Drag);
    }
    if cfg!(target_os = "macos") {
        // AppKit's drag session swallows the click it starts from, so the drag
        // begins on the first move and a double click still arrives as one; macOS has no window menu.
        return area
            .on_mouse_down(MouseButton::Left, |_, _, _| {
                MAC_DRAG_PENDING.store(true, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_up(MouseButton::Left, |_, _, _| {
                MAC_DRAG_PENDING.store(false, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_down_out(|_, _, _| {
                MAC_DRAG_PENDING.store(false, std::sync::atomic::Ordering::Relaxed)
            })
            .on_mouse_move(|_, window, _| {
                if MAC_DRAG_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    window.start_window_move();
                }
            })
            .on_click(|e, window, _| {
                if e.click_count() == 2 {
                    window.titlebar_double_click();
                }
            });
    }
    area.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .on_click(|e, window, _| {
            if e.click_count() == 2 {
                window.zoom_window();
            }
        })
        .on_mouse_down(MouseButton::Right, |e, window, _| {
            window.show_window_menu(e.position)
        })
}

/// Which edge or corner of the window `pos` is over, in the shadow margin of a non-tiled side.
fn resize_edge(
    pos: gpui::Point<Pixels>,
    size: gpui::Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let m = FRAME_SHADOW;
    let top = pos.y < m && !tiling.top;
    let bottom = pos.y > size.height - m && !tiling.bottom;
    let left = pos.x < m && !tiling.left;
    let right = pos.x > size.width - m && !tiling.right;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (true, _, _, _) => ResizeEdge::Top,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (_, true, _, _) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

/// Whether TFT draws the window's title row itself. On Linux this is whatever the
/// compositor granted (`Decorations::Client`); Windows and macOS answer from the
/// transparent-title-bar request. On macOS the row holds the system traffic lights.
pub(crate) fn frameless(window: &Window) -> bool {
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        crate::hidden_frame()
    } else {
        matches!(window.window_decorations(), Decorations::Client { .. })
    }
}

/// TFT's own frame for a desktop-bare window: a margin of [`FRAME_SHADOW`] holding
/// a soft shadow and resize handles, a one pixel border, and no margin on tiled or
/// maximised edges. With the desktop's frame the content passes through untouched.
pub(crate) fn client_frame(
    content: gpui::AnyElement,
    theme: &Theme,
    window: &mut Window,
) -> gpui::Stateful<gpui::Div> {
    let decorations = window.window_decorations();
    let tiling = match decorations {
        Decorations::Client { tiling } => tiling,
        Decorations::Server => Tiling::default(),
    };
    let client = matches!(decorations, Decorations::Client { .. });
    window.set_client_inset(if client { FRAME_SHADOW } else { px(0.) });
    let border = theme.tokens.background_modifier_border;
    let shadow: Hsla = rgba(FRAME_SHADOW_COLOR).into();
    let corners = WindowCorners::of(window);
    div()
        .id("window-backdrop")
        .size_full()
        .bg(transparent_black())
        .map(|d| corners.apply(d))
        .when(client, |d| {
            d.when(!tiling.top, |d| d.pt(FRAME_SHADOW))
                .when(!tiling.bottom, |d| d.pb(FRAME_SHADOW))
                .when(!tiling.left, |d| d.pl(FRAME_SHADOW))
                .when(!tiling.right, |d| d.pr(FRAME_SHADOW))
                .child(
                    canvas(
                        |_, window, _| {
                            window.insert_hitbox(
                                gpui::Bounds::new(
                                    point(px(0.), px(0.)),
                                    crate::zoom::window_size(window),
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_, hitbox, window, _| {
                            let size = crate::zoom::window_size(window);
                            let Some(edge) = resize_edge(window.mouse_position(), size, tiling)
                            else {
                                return;
                            };
                            let style = match edge {
                                ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                ResizeEdge::Left | ResizeEdge::Right => {
                                    CursorStyle::ResizeLeftRight
                                }
                                ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                    CursorStyle::ResizeUpLeftDownRight
                                }
                                ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                    CursorStyle::ResizeUpRightDownLeft
                                }
                            };
                            window.set_cursor_style(style, &hitbox);
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .on_mouse_move(|_, window, _| window.refresh())
                .on_mouse_down(MouseButton::Left, move |e, window, _| {
                    let size = crate::zoom::window_size(window);
                    if let Some(edge) = resize_edge(e.position, size, tiling) {
                        window.start_window_resize(edge);
                    }
                })
        })
        .child(
            div()
                .size_full()
                .cursor(CursorStyle::Arrow)
                .bg(theme.tokens.background_primary)
                .map(|d| corners.apply(d))
                .when(client, |d| {
                    d.border_color(border)
                        .when(!tiling.top, |d| d.border_t_1())
                        .when(!tiling.bottom, |d| d.border_b_1())
                        .when(!tiling.left, |d| d.border_l_1())
                        .when(!tiling.right, |d| d.border_r_1())
                        .when(!tiling.is_tiled(), |d| {
                            d.shadow(vec![BoxShadow {
                                color: shadow,
                                blur_radius: FRAME_SHADOW_BLUR,
                                spread_radius: px(0.),
                                offset: point(px(0.), FRAME_SHADOW_OFFSET),
                            }])
                        })
                })
                // The margin's own listeners see only what the content
                // lets through, so the content keeps the pointer's moves.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
}

/// The frame's margin a popout's surface carries on each side beyond its
/// inner size: with TFT's own frame on Linux the shadow is drawn inside
/// the window, so the window is asked for that much more than the inner
/// size `workspace.json` records, and the recorder takes the same margin
/// back off (see [`Workspace::render_popout`]). It follows what TFT
/// asked for, not the frame the compositor settled on: that is not known
/// until some frames in, and a window opened and recorded by the same
/// rule reopens at the size it closed at whichever frame it got.
pub(crate) fn popout_margin() -> Pixels {
    if crate::hidden_frame() && cfg!(target_os = "linux") {
        FRAME_SHADOW
    } else {
        px(0.)
    }
}

/// The frame-related window options every window of TFT's opens with,
/// the main one included in spirit: the desktop's frame when the frame
/// style is `native`, TFT's own otherwise, with the transparent margin
/// the shadow needs on Linux and the hidden caption on Windows and macOS.
/// A caller sets its bounds and minimum size over these.
pub(crate) fn frame_window_options(title: &'static str) -> gpui::WindowOptions {
    let hidden = crate::hidden_frame();
    let decorations = if hidden {
        gpui::WindowDecorations::Client
    } else {
        gpui::WindowDecorations::Server
    };
    gpui::WindowOptions {
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(title.into()),
            // What hides the caption on Windows and macOS, where
            // `window_decorations` is not read at all; see `main.rs`.
            appears_transparent: (cfg!(target_os = "windows") || cfg!(target_os = "macos"))
                && hidden,
            traffic_light_position: (cfg!(target_os = "macos") && hidden)
                .then(|| point(px(19.), px(12.))),
        }),
        // On macOS the hidden frame keeps the system's title bar, see-through,
        // over the top 28 px of the content, and a movable window is dragged
        // by any press there: the tab under it never saw its own drag start.
        // TFT moves the window itself from its drag areas
        // (`window_handle`), which a window that is not movable still obeys.
        is_movable: !(cfg!(target_os = "macos") && hidden),
        // KWin ties the activation token to the application by this id.
        app_id: Some("tft".into()),
        window_decorations: Some(decorations),
        // The shadow's margin has to be see-through, and only Linux draws
        // one: Windows keeps the system's shadow outside the window.
        window_background: match decorations {
            gpui::WindowDecorations::Client if !cfg!(target_os = "windows") => {
                gpui::WindowBackgroundAppearance::Transparent
            }
            _ => gpui::WindowBackgroundAppearance::Opaque,
        },
        ..Default::default()
    }
}

/// The three window buttons at the top right of a frameless window:
/// minimise, maximise or restore, and close, each `RIBBON_WIDTH` wide
/// and `HEADER_HEIGHT` tall, the close one red on hover and rounded with
/// the window's corner. `close` is what the close button does, since a
/// window with an open text field has a write to make on the way out.
/// On Windows the buttons name their areas instead and the system acts.
pub(crate) fn window_controls(
    theme: &Theme,
    window: &Window,
    corners: WindowCorners,
    close: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::Div {
    /// What a window button does when clicked.
    type Act = Box<dyn Fn(&mut Window, &mut App)>;
    let maximized = window.is_maximized();
    let system = cfg!(target_os = "windows");
    let button =
        |id: &'static str, name: &'static str, close: bool, area: WindowControlArea, act: Act| {
            let hover_bg: Hsla = if close {
                rgb(0xe93147).into()
            } else {
                theme.tokens.background_modifier_hover
            };
            let color = theme.tokens.text_muted;
            div()
                .id(id)
                .group(id)
                .w(px(RIBBON_WIDTH))
                .h(px(HEADER_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(close && corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
                .hover(move |s| s.bg(hover_bg))
                .when(system, |d| d.occlude().window_control_area(area))
                .when(!system, |d| {
                    d.on_click(move |_, window, cx| act(window, cx))
                })
                .child(
                    div()
                        .when(close, |d| {
                            d.group_hover(id, |s| s.text_color(gpui::white()))
                        })
                        .text_color(color)
                        .child(icon(name, 14., color)),
                )
        };
    div()
        .absolute()
        .top_0()
        .right_0()
        .flex()
        .flex_row()
        .child(button(
            "win-minimize",
            "minus",
            false,
            WindowControlArea::Min,
            Box::new(|w, _| w.minimize_window()),
        ))
        .child(button(
            "win-maximize",
            if maximized { "copy" } else { "square" },
            false,
            WindowControlArea::Max,
            Box::new(|w, _| w.zoom_window()),
        ))
        .child(button(
            "win-close",
            "x",
            true,
            WindowControlArea::Close,
            Box::new(close),
        ))
}

/// A title row for a frameless secondary window: `HEADER_HEIGHT` of `bg`
/// that drags the window, with `title` centred on it the way Obsidian's
/// `.titlebar-text` is, the window buttons at its right, or on macOS the
/// desktop's traffic lights at its left. `bg` is `--titlebar-background`
/// where the row is meant to read as a bar, and the window's own ground
/// where it is not. With the desktop's frame there is no row: the
/// desktop's title bar, with the same title, is above the window instead.
pub(crate) fn window_title_row(
    title: &'static str,
    bg: Hsla,
    theme: &Theme,
    window: &Window,
    close: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<gpui::Stateful<gpui::Div>> {
    if !frameless(window) {
        return None;
    }
    let corners = WindowCorners::of(window);
    let mac = cfg!(target_os = "macos");
    // The handle is a sibling of the buttons: a press inside a button would move
    // the window and swallow the click.
    let handle = window_handle(
        div()
            .id("window-title-drag")
            .flex_1()
            .h_full()
            .when(mac, |d| d.pl(px(MAC_FRAME_LEFT_SPACE))),
        true,
    );
    Some(
        div()
            .id("window-title-row")
            .relative()
            .flex_shrink_0()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .flex()
            .flex_row()
            .bg(bg)
            .map(|d| corners.top().apply(d))
            // The title is centred on the row and painted under the handle and
            // buttons, with no hitbox, so a press still drags the window.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.tokens.text_muted)
                    .child(title),
            )
            .child(handle)
            .when(!mac, |d| {
                d.child(
                    div()
                        .relative()
                        .flex_shrink_0()
                        .w(px(3. * RIBBON_WIDTH))
                        .h_full()
                        .child(window_controls(theme, window, corners, close)),
                )
            }),
    )
}
