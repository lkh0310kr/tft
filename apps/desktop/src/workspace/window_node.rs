use crate::popout::PopoutWindow;
use gpui::{App, Bounds, Pixels, SharedString, px};
use serde_json::{Map, Value, json};
use tft_app::Layout;

/// One popout window, as `workspace.json`'s `floating` split records it.
/// A restored node has no GPUI window until it is opened.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WindowNode {
    pub(super) id: u64,
    /// The groups and how they are split. Window metadata lives in `node`.
    pub(super) tree: Layout,
    /// Window keys read from the file, including unknown keys.
    pub(super) node: Map<String, Value>,
    /// The GPUI window showing this node, while one is open.
    pub(super) handle: Option<gpui::WindowHandle<PopoutWindow>>,
    /// What that window's title bar says, set only when it changes.
    pub(super) title: SharedString,
    /// Inner size the window was opened for, accounting for fractional display scale.
    pub(super) opened_for: Option<(i32, i32)>,
}

impl WindowNode {
    /// A window TFT opened: not maximized, at zoom 0.
    pub(super) fn new(id: u64, tree: Layout, bounds: WindowBounds) -> WindowNode {
        let mut node = Map::new();
        node.insert("id".into(), json!(super::leaf_id()));
        node.insert("type".into(), json!("window"));
        node.insert("x".into(), json!(bounds.x));
        node.insert("y".into(), json!(bounds.y));
        node.insert("width".into(), json!(bounds.width));
        node.insert("height".into(), json!(bounds.height));
        node.insert("maximize".into(), json!(false));
        node.insert("zoom".into(), json!(0));
        WindowNode::from_node(id, tree, node)
    }

    /// A window as the file described it, not yet shown.
    pub(super) fn from_node(id: u64, tree: Layout, node: Map<String, Value>) -> WindowNode {
        WindowNode {
            id,
            tree,
            node,
            handle: None,
            title: SharedString::default(),
            opened_for: None,
        }
    }

    /// Records the current bounds and maximized state in the window metadata.
    pub(super) fn set_bounds(&mut self, bounds: WindowBounds, maximized: bool) {
        for (key, value) in [
            ("x", json!(bounds.x)),
            ("y", json!(bounds.y)),
            ("width", json!(bounds.width)),
            ("height", json!(bounds.height)),
            ("maximize", json!(maximized)),
        ] {
            self.node.insert(key.into(), value);
        }
    }

    /// Whether the GPUI window is still up.
    pub(super) fn is_open(&self, cx: &App) -> bool {
        self.handle.is_some_and(|handle| handle.read(cx).is_ok())
    }

    /// The recorded position and inner size, when all four are present.
    pub(super) fn bounds(&self) -> Option<WindowBounds> {
        let number = |key: &str| {
            self.node
                .get(key)
                .and_then(Value::as_f64)
                .map(|value| value.round() as i32)
        };
        Some(WindowBounds {
            x: number("x")?,
            y: number("y")?,
            width: number("width")?,
            height: number("height")?,
        })
    }
}

/// A popout's recorded position and inner size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowBounds {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

/// The surface size for a popout with the given inner size.
pub(super) fn popout_outer_size(width: f32, height: f32) -> gpui::Size<Pixels> {
    let margin = super::popout_margin() * 2.;
    gpui::size(px(width) + margin, px(height) + margin)
}

/// Opens at the recorded place, or centered when the display is no longer present.
pub(super) fn popout_placement(
    recorded: Option<(WindowBounds, bool)>,
    cx: &App,
) -> gpui::WindowBounds {
    let Some((recorded, maximized)) = recorded else {
        let (width, height) = crate::popout::INNER_SIZE;
        return gpui::WindowBounds::Windowed(Bounds::centered(
            None,
            popout_outer_size(width, height),
            cx,
        ));
    };
    let size = popout_outer_size(recorded.width as f32, recorded.height as f32);
    let origin = gpui::point(px(recorded.x as f32), px(recorded.y as f32));
    let on_screen = cx
        .displays()
        .iter()
        .any(|display| display.bounds().contains(&origin));
    let bounds = if on_screen {
        Bounds::new(origin, size)
    } else {
        Bounds::centered(None, size, cx)
    };
    if maximized {
        gpui::WindowBounds::Maximized(bounds)
    } else {
        gpui::WindowBounds::Windowed(bounds)
    }
}
