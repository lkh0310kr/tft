//! Main terminal view component for GPUI.
//!
//! This module provides [`TerminalView`], the primary component for embedding terminals
//! in GPUI applications. It manages:
//!
//! - **I/O Streams**: Accepts arbitrary [`Read`]/[`Write`]
//!   streams, allowing integration with any PTY implementation
//! - **Event Handling**: Keyboard and mouse input, with configurable callbacks
//! - **Rendering**: Efficient canvas-based rendering via [`TerminalRenderer`]
//! - **Configuration**: Font, colors, dimensions, and padding via [`TerminalConfig`]
//!
//! # Architecture
//!
//! The terminal uses a push-based async I/O architecture:
//!
//! 1. A background thread reads bytes from the PTY stdout in 4KB chunks
//! 2. Bytes are sent through a [flume](https://docs.rs/flume) channel to an async task
//! 3. The async task processes bytes through the VTE parser and calls `cx.notify()`
//! 4. GPUI repaints the terminal with the updated grid
//!
//! This approach ensures the terminal only wakes when data arrives, avoiding polling.
//!
//! # Thread Safety
//!
//! - [`TerminalView`] itself is not `Send` (it contains GPUI handles)
//! - The stdin writer is wrapped in `Arc<parking_lot::Mutex<>>` for thread-safe writes
//! - Callbacks ([`ResizeCallback`], [`KeyHandler`]) must be `Send + Sync`
//!
//! # Example
//!
//! ```ignore
//! use gpui::{Context, Edges, px};
//! use gpui_terminal::{ColorPalette, TerminalConfig, TerminalView};
//!
//! // In a GPUI window context:
//! let terminal = cx.new(|cx| {
//!     TerminalView::new(pty_writer, pty_reader, TerminalConfig::default(), cx)
//!         .with_resize_callback(move |cols, rows| {
//!             // Notify PTY of new dimensions
//!         })
//!         .with_exit_callback(|_, cx| {
//!             cx.quit();
//!         })
//! });
//!
//! // Focus the terminal to receive keyboard input
//! terminal.read(cx).focus_handle().focus(window);
//! ```

actions!(terminal_view, [Find, FindPrevious, Clear]);

use crate::colors::ColorPalette;
use crate::event::{GpuiEventProxy, TerminalEvent};
use crate::input::{KeyEventKind, keystroke_to_bytes_for_event};
use crate::render::{SearchMatch, TerminalRenderer};
use crate::terminal::TerminalState;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::NamedColor;
use gpui::Focusable;
use gpui::prelude::*;
use gpui::{Edges, *};
use std::cell::Cell;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use tft_app::text_input::{InputEvent, TextInput};
use tft_app::theme::Theme;

/// Configuration for terminal creation and runtime updates.
///
/// This struct defines the terminal's appearance and behavior, including
/// grid dimensions, font settings, scrollback buffer, and color scheme.
///
/// # Default Values
///
/// | Field | Default |
/// |-------|---------|
/// | `cols` | 80 |
/// | `rows` | 24 |
/// | `font_family` | "monospace" |
/// | `font_size` | 14px |
/// | `scrollback` | 10000 |
/// | `line_height_multiplier` | 1.2 |
/// | `padding` | 0px all sides |
/// | `colors` | Default palette |
///
/// # Example
///
/// ```ignore
/// use gpui::{Edges, px};
/// use gpui_terminal::{ColorPalette, TerminalConfig};
///
/// let config = TerminalConfig {
///     cols: 120,
///     rows: 40,
///     font_family: "JetBrains Mono".into(),
///     font_size: px(13.0),
///     scrollback: 50000,
///     line_height_multiplier: 1.1,
///     padding: Edges::all(px(10.0)),
///     colors: ColorPalette::builder()
///         .background(0x1a, 0x1a, 0x1a)
///         .foreground(0xe0, 0xe0, 0xe0)
///         .build(),
/// };
/// ```
///
/// # Runtime Updates
///
/// Configuration can be updated at runtime via [`TerminalView::update_config`].
/// This is useful for implementing features like dynamic font sizing:
///
/// ```ignore
/// terminal.update(cx, |terminal, cx| {
///     let mut config = terminal.config().clone();
///     config.font_size += px(1.0);
///     terminal.update_config(config, cx);
/// });
/// ```
#[derive(Clone, Debug)]
pub struct TerminalConfig {
    /// Number of columns (character width) in the terminal
    pub cols: usize,

    /// Number of rows (lines) in the terminal
    pub rows: usize,

    /// Font family name (e.g., "Fira Code", "JetBrains Mono")
    pub font_family: String,

    /// Font size in pixels
    pub font_size: Pixels,

    /// Maximum number of scrollback lines to keep in history
    pub scrollback: usize,

    /// Multiplier for line height to accommodate tall glyphs (e.g., nerd fonts)
    /// Default is 1.2 (20% extra height)
    pub line_height_multiplier: f32,

    /// Padding around the terminal content (top, right, bottom, left)
    /// The padding area renders with the terminal's background color
    pub padding: Edges<Pixels>,

    /// Color palette for terminal colors (16 ANSI colors, 256 extended colors,
    /// foreground, background, and cursor colors)
    pub colors: ColorPalette,

    /// The shell to start; `None` uses `$SHELL`.
    pub shell: Option<String>,

    /// Extra environment variables for the shell, in addition to the
    /// inherited environment and the terminal's own (`TERM`, …).
    pub env: Vec<(String, String)>,

    /// Start the shell as a login shell (`-l`), so it reads the profile a
    /// desktop session would.
    pub login_shell: bool,

    /// Blink the cursor while it is on screen.
    pub cursor_blink: bool,

    /// Copy a selection as soon as the mouse releases it, as Zed can.
    pub copy_on_select: bool,

    /// Option is Meta for the shell (word motion); when false, Option types
    /// the alternate character the layout produces.
    pub option_as_meta: bool,

    /// Ring the terminal bell visibly when an application sends BEL.
    pub bell: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            font_family: "monospace".into(),
            font_size: px(14.0),
            scrollback: 10000,
            line_height_multiplier: 1.2,
            padding: Edges::all(px(0.0)),
            colors: ColorPalette::default(),
            shell: None,
            env: Vec::new(),
            login_shell: false,
            cursor_blink: true,
            copy_on_select: false,
            option_as_meta: true,
            bell: true,
        }
    }
}

/// Callback type for PTY resize notifications.
///
/// This callback is invoked when the terminal grid dimensions change,
/// typically due to window resizing. The callback receives the new
/// column and row counts.
///
/// # Arguments
///
/// * `cols` - New number of columns (characters wide)
/// * `rows` - New number of rows (lines tall)
///
/// # Thread Safety
///
/// This callback must be `Send + Sync` as it may be called from the render thread.
///
/// # Example
///
/// ```ignore
/// use portable_pty::PtySize;
///
/// let pty = Arc::new(Mutex::new(pty_master));
/// let pty_clone = pty.clone();
///
/// terminal.with_resize_callback(move |cols, rows| {
///     pty_clone.lock().resize(PtySize {
///         cols: cols as u16,
///         rows: rows as u16,
///         pixel_width: 0,
///         pixel_height: 0,
///     }).ok();
/// });
/// ```
pub type ResizeCallback = Box<dyn Fn(usize, usize) + Send + Sync>;

/// Callback type for key event interception.
///
/// This callback is invoked before the terminal processes a key event,
/// allowing you to intercept and handle specific key combinations.
///
/// # Arguments
///
/// * `event` - The key down event from GPUI
///
/// # Returns
///
/// * `true` - Consume the event (terminal will not process it)
/// * `false` - Let the terminal handle the event normally
///
/// # Thread Safety
///
/// This callback must be `Send + Sync`.
///
/// # Example
///
/// ```ignore
/// terminal.with_key_handler(|event| {
///     let keystroke = &event.keystroke;
///
///     // Intercept Ctrl++ for font size increase
///     if keystroke.modifiers.control && (keystroke.key == "+" || keystroke.key == "=") {
///         // Handle font size increase
///         return true; // Consume the event
///     }
///
///     // Intercept Ctrl+- for font size decrease
///     if keystroke.modifiers.control && keystroke.key == "-" {
///         // Handle font size decrease
///         return true;
///     }
///
///     false // Let terminal handle all other keys
/// });
/// ```
pub type KeyHandler = Box<dyn Fn(&KeyDownEvent) -> bool + Send + Sync>;

/// Callback for terminal bell events.
///
/// This callback is invoked when the terminal bell is triggered (BEL character,
/// ASCII 0x07), allowing you to play a sound or show a visual indicator.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
///
/// # Example
///
/// ```ignore
/// terminal.with_bell_callback(|window, cx| {
///     // Option 1: Visual bell (flash the window or show an indicator)
///     // Option 2: Play a sound
///     // Option 3: Notify the user via system notification
/// });
/// ```
pub type BellCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>)>;

/// Callback for terminal title changes.
///
/// This callback is invoked when the terminal title changes via escape sequences
/// (OSC 0, OSC 2), allowing you to update the window or tab title.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
/// * `title` - The new title string
///
/// # Example
///
/// ```ignore
/// terminal.with_title_callback(|window, cx, title| {
///     // Update the window title
///     // Or update a tab label in a tabbed interface
///     println!("Terminal title changed to: {}", title);
/// });
/// ```
pub type TitleCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>, &str)>;

/// Emitted when the running application sets or resets its title (OSC 0/2),
/// so a host can rename the tab that holds this terminal.
#[derive(Clone, Debug)]
pub struct TerminalTitle(pub String);

impl EventEmitter<TerminalTitle> for TerminalView {}

/// A URL or path the user asked to open from the terminal.
#[derive(Clone, Debug)]
pub struct TerminalLink {
    pub target: String,
    /// The shell's working directory as its OSC 7 report last named it, so
    /// a relative path resolves where the command that printed it ran.
    pub cwd: Option<std::path::PathBuf>,
}

impl EventEmitter<TerminalLink> for TerminalView {}

/// Callback for clipboard store requests.
///
/// This callback is invoked when the terminal wants to store data to the clipboard
/// via OSC 52 escape sequence. Applications like tmux and vim can use this to
/// copy text to the system clipboard.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
/// * `text` - The text to store in the clipboard
///
/// # Example
///
/// ```ignore
/// use gpui_terminal::Clipboard;
///
/// terminal.with_clipboard_store_callback(|window, cx, text| {
///     if let Ok(mut clipboard) = Clipboard::new() {
///         clipboard.copy(text).ok();
///     }
/// });
/// ```
pub type ClipboardStoreCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>, &str)>;

/// Callback for terminal exit events.
///
/// This callback is invoked when the terminal process exits (e.g., shell exits,
/// process terminates). This is detected when the PTY reader reaches EOF.
///
/// # Arguments
///
/// * `window` - The GPUI window
/// * `cx` - The context for the TerminalView
///
/// # Example
///
/// ```ignore
/// terminal.with_exit_callback(|window, cx| {
///     // Option 1: Quit the application
///     cx.quit();
///
///     // Option 2: Close this terminal tab/pane
///     // terminal_manager.close_terminal(terminal_id);
///
///     // Option 3: Show an exit message
///     // show_notification("Terminal exited");
/// });
/// ```
pub type ExitCallback = Box<dyn Fn(&mut Window, &mut Context<TerminalView>)>;

/// The main terminal view component for GPUI applications.
///
/// `TerminalView` is a GPUI entity that implements the [`Render`] trait,
/// providing a complete terminal emulator that can be embedded in any GPUI application.
///
/// # Responsibilities
///
/// - **Terminal State**: Manages the grid, cursor, and colors via [`TerminalState`]
/// - **I/O Streams**: Reads from PTY stdout and writes to PTY stdin
/// - **Event Handling**: Processes keyboard, mouse, and resize events
/// - **Rendering**: Paints text, backgrounds, and cursor via [`TerminalRenderer`]
/// - **Callbacks**: Dispatches events to user-provided callbacks
///
/// # Creating a Terminal
///
/// Use [`TerminalView::new`] within a GPUI entity context:
///
/// ```ignore
/// let terminal = cx.new(|cx| {
///     TerminalView::new(writer, reader, config, cx)
///         .with_resize_callback(resize_callback)
///         .with_exit_callback(|_, cx| cx.quit())
/// });
/// ```
///
/// # Focus
///
/// The terminal must be focused to receive keyboard input:
///
/// ```ignore
/// terminal.read(cx).focus_handle().focus(window);
/// ```
///
/// # Callbacks
///
/// Configure behavior through builder methods:
///
/// - [`with_resize_callback`](Self::with_resize_callback) - PTY size changes
/// - [`with_exit_callback`](Self::with_exit_callback) - Process exit
/// - [`with_key_handler`](Self::with_key_handler) - Key event interception
/// - [`with_bell_callback`](Self::with_bell_callback) - Terminal bell
/// - [`with_title_callback`](Self::with_title_callback) - Title changes
/// - [`with_clipboard_store_callback`](Self::with_clipboard_store_callback) - Clipboard writes
///
/// # Thread Safety
///
/// `TerminalView` is not `Send` as it contains GPUI handles. The stdin writer
/// is internally wrapped in `Arc<parking_lot::Mutex<>>` for safe concurrent access.
pub struct TerminalView {
    /// The terminal state managing the grid and VTE parser
    state: TerminalState,

    /// The renderer for drawing terminal content
    renderer: TerminalRenderer,

    /// Focus handle for keyboard event handling
    focus_handle: FocusHandle,

    /// Writer for sending input to the terminal process
    stdin_writer: Arc<parking_lot::Mutex<Box<dyn Write + Send>>>,

    /// Receiver for terminal events from the event proxy
    event_rx: mpsc::Receiver<TerminalEvent>,

    /// Configuration used to create this terminal
    config: TerminalConfig,

    /// Async task that reads bytes and notifies the view (push-based)
    #[allow(dead_code)]
    _reader_task: Task<()>,

    /// Callback to notify the PTY about size changes
    resize_callback: Option<Arc<ResizeCallback>>,

    /// Optional callback to intercept key events before terminal processing
    key_handler: Option<Arc<KeyHandler>>,

    /// Callback for terminal bell events
    bell_callback: Option<BellCallback>,

    /// Callback for terminal title changes
    title_callback: Option<TitleCallback>,

    /// Callback for clipboard store requests
    clipboard_store_callback: Option<ClipboardStoreCallback>,

    /// Callback for terminal exit events
    exit_callback: Option<ExitCallback>,

    /// The canvas bounds and measured cell size last painted, for turning a
    /// pointer position into a grid cell.
    viewport_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    viewport_cell: Rc<Cell<(Pixels, Pixels)>>,

    /// The pointer is dragging a selection, and the cell it started on.
    selecting: bool,
    select_anchor: Option<AlacPoint>,
    select_type: SelectionType,

    /// Text an IME is composing; the input method commits it through
    /// [`EntityInputHandler::replace_text_in_range`].
    marked_text: Option<String>,

    /// With `REPORT_ALL_KEYS_AS_ESC` a printable key already reached the
    /// shell as a kitty report; the input handler's commit for the same
    /// keystroke must not send it a second time.
    suppress_commit: Option<String>,

    /// The last cell a mouse report was sent for, and the button held.
    last_mouse_cell: Option<(usize, usize)>,
    mouse_button: Option<MouseButton>,

    /// Files from outside the app being dragged over the grid.
    drop_hover: Option<Vec<std::path::PathBuf>>,

    /// The link under the pointer and the cells it covers, for the
    /// underline and pointing hand; the pointer position anchors the
    /// target tooltip.
    hover_link: Option<(String, SearchMatch)>,
    hover_point: Point<Pixels>,

    /// A partial OSC 7 sequence (the shell's working directory) carried
    /// across PTY reads.
    osc_tail: Vec<u8>,
    /// The shell's working directory as its most recent OSC 7 reported it.
    cwd: Option<std::path::PathBuf>,

    /// The cursor is in the visible half of its blink cycle.
    cursor_on: bool,
    blink_task: Option<Task<()>>,
    /// A BEL flash: the pane's border lights for a moment.
    bell_flash: bool,
    bell_task: Option<Task<()>>,

    /// The scrollbar thumb is being dragged.
    scrollbar_drag: bool,

    /// A right click opened the context menu here.
    context_menu: Option<Point<Pixels>>,

    /// The floating find panel, its text field, options, and every hit in
    /// the grid and scrollback.
    find_open: bool,
    find_query: Entity<TextInput>,
    find_match_case: bool,
    find_regex: bool,
    find_matches: Vec<SearchMatch>,
    find_current: usize,
    _find_subscription: Subscription,
    /// The query field is hidden; give the keyboard back to the grid after
    /// the next paint.
    refocus_after_find: bool,
}

impl TerminalView {
    /// Create a new terminal with provided I/O streams.
    ///
    /// This method initializes a new terminal emulator with the given stdin writer
    /// and stdout reader. It spawns a background task to read from stdout and
    /// process incoming bytes through the VTE parser.
    ///
    /// # Arguments
    ///
    /// * `stdin_writer` - Writer for sending input bytes to the terminal process
    /// * `stdout_reader` - Reader for receiving output bytes from the terminal process
    /// * `config` - Terminal configuration (dimensions, font, etc.)
    /// * `cx` - GPUI context for this view
    ///
    /// # Returns
    ///
    /// A new `TerminalView` instance ready to be rendered.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // In a GPUI window context:
    /// let terminal = cx.new(|cx| {
    ///     TerminalView::new(stdin_writer, stdout_reader, TerminalConfig::default(), cx)
    /// });
    /// ```
    pub fn new<W, R>(
        stdin_writer: W,
        stdout_reader: R,
        config: TerminalConfig,
        cx: &mut Context<Self>,
    ) -> Self
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        // Create event channel for terminal events
        let (event_tx, event_rx) = mpsc::channel();

        // Clone event_tx for the reader task to send Exit event when PTY closes
        let exit_event_tx = event_tx.clone();

        // Create event proxy for alacritty
        let event_proxy = GpuiEventProxy::new(event_tx);

        // Create terminal state
        let state = TerminalState::new(config.cols, config.rows, event_proxy);

        // Create renderer with font settings and color palette
        let renderer = TerminalRenderer::new(
            config.font_family.clone(),
            config.font_size,
            config.line_height_multiplier,
            config.colors.clone(),
        );

        // Create focus handle
        let focus_handle = cx.focus_handle();

        // Wrap stdin writer in Arc<Mutex> for thread-safe access
        let stdin_writer = Arc::new(parking_lot::Mutex::new(
            Box::new(stdin_writer) as Box<dyn Write + Send>
        ));

        // Create async channel for bytes (push-based notification)
        // Using flume instead of smol::channel because flume is executor-agnostic
        // and properly wakes GPUI's async executor when data arrives
        let (bytes_tx, bytes_rx) = flume::unbounded::<Vec<u8>>();

        // Spawn background thread to read from stdout
        // This thread sends bytes through the async channel
        thread::spawn(move || {
            Self::read_stdout_blocking(stdout_reader, bytes_tx);
        });

        // Spawn async task that awaits on the channel and notifies the view
        // This is push-based: the task blocks until bytes arrive, then immediately notifies
        let reader_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                // Wait for bytes from the background reader (blocks until data arrives)
                match bytes_rx.recv_async().await {
                    Ok(bytes) => {
                        // Process bytes and notify the view
                        let result = this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            // OSC 7 carries the shell's working directory;
                            // the grid ignores it, so it is read here before
                            // the bytes reach the parser.
                            view.observe_cwd(&bytes);
                            view.state.process_bytes(&bytes);
                            cx.notify();
                        });
                        if result.is_err() {
                            // View was dropped, exit
                            break;
                        }
                    }
                    Err(_) => {
                        // Channel closed - PTY has finished, send Exit event
                        let _ = exit_event_tx.send(TerminalEvent::Exit);
                        // Notify view to process the Exit event
                        let _ = this.update(cx, |_view, cx: &mut Context<Self>| {
                            cx.notify();
                        });
                        break;
                    }
                }
            }
        });

        let find_query = cx.new(|cx| TextInput::new("Find", cx));
        let find_subscription =
            cx.subscribe(&find_query, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Changed => this.refresh_find(cx),
                InputEvent::Submit => this.find_next(false, cx),
                InputEvent::Cancel => this.close_find(cx),
                InputEvent::Up => this.find_next(true, cx),
                InputEvent::Down => this.find_next(false, cx),
            });

        Self {
            state,
            renderer,
            focus_handle,
            stdin_writer,
            event_rx,
            config,
            _reader_task: reader_task,
            resize_callback: None,
            key_handler: None,
            bell_callback: None,
            title_callback: None,
            clipboard_store_callback: None,
            exit_callback: None,
            viewport_bounds: Rc::new(Cell::new(None)),
            viewport_cell: Rc::new(Cell::new((px(0.), px(0.)))),
            selecting: false,
            select_anchor: None,
            select_type: SelectionType::Simple,
            marked_text: None,
            suppress_commit: None,
            last_mouse_cell: None,
            mouse_button: None,
            drop_hover: None,
            hover_link: None,
            hover_point: Point::default(),
            osc_tail: Vec::new(),
            cwd: None,
            cursor_on: true,
            blink_task: None,
            bell_flash: false,
            bell_task: None,
            scrollbar_drag: false,
            context_menu: None,
            find_open: false,
            find_query,
            find_match_case: false,
            find_regex: false,
            find_matches: Vec::new(),
            find_current: 0,
            _find_subscription: find_subscription,
            refocus_after_find: false,
        }
    }

    /// Set a callback to be invoked when the terminal is resized.
    ///
    /// This callback should resize the underlying PTY to match the new dimensions.
    /// The callback receives (cols, rows) as arguments.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with (cols, rows) on resize
    pub fn with_resize_callback(
        mut self,
        callback: impl Fn(usize, usize) + Send + Sync + 'static,
    ) -> Self {
        self.resize_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Set a callback to intercept key events before terminal processing.
    ///
    /// The callback receives the key event and should return `true` to consume
    /// the event (prevent the terminal from processing it), or `false` to allow
    /// normal terminal processing.
    ///
    /// # Arguments
    ///
    /// * `handler` - A function that receives key events and returns whether to consume them
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_key_handler(|event| {
    ///     // Handle Ctrl++ to increase font size
    ///     if event.keystroke.modifiers.control && event.keystroke.key == "+" {
    ///         // Handle the event
    ///         return true; // Consume the event
    ///     }
    ///     false // Let terminal handle it
    /// })
    /// ```
    pub fn with_key_handler(
        mut self,
        handler: impl Fn(&KeyDownEvent) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.key_handler = Some(Arc::new(Box::new(handler)));
        self
    }

    /// Set a callback to be invoked when the terminal bell is triggered.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// allowing you to play a sound or show a visual indicator.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called when the bell is triggered
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_bell_callback(|window, cx| {
    ///     // Play a sound or flash the screen
    /// })
    /// ```
    pub fn with_bell_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>) + 'static,
    ) -> Self {
        self.bell_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal title changes.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// along with the new title string.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with the new title
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_title_callback(|window, cx, title| {
    ///     // Update window title or tab title
    /// })
    /// ```
    pub fn with_title_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>, &str) + 'static,
    ) -> Self {
        self.title_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal wants to store data to the clipboard.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// along with the text to store. This is typically triggered by OSC 52 escape sequences.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with the text to store
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_clipboard_store_callback(|window, cx, text| {
    ///     // Store text to system clipboard
    /// })
    /// ```
    pub fn with_clipboard_store_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>, &str) + 'static,
    ) -> Self {
        self.clipboard_store_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal process exits.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// allowing you to close the terminal view or show an exit message.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called when the process exits
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_exit_callback(|window, cx| {
    ///     // Close the terminal tab or show exit message
    /// })
    /// ```
    pub fn with_exit_callback(
        mut self,
        callback: impl Fn(&mut Window, &mut Context<TerminalView>) + 'static,
    ) -> Self {
        self.exit_callback = Some(Box::new(callback));
        self
    }

    /// Background thread that reads from stdout.
    ///
    /// This function runs in a background thread, continuously reading bytes
    /// from the stdout reader and sending them through the async channel.
    /// The async channel allows the main async task to be woken up immediately
    /// when data arrives (push-based).
    fn read_stdout_blocking<R: Read + Send + 'static>(
        mut stdout_reader: R,
        bytes_tx: flume::Sender<Vec<u8>>,
    ) {
        let mut buffer = [0u8; 4096];

        loop {
            match stdout_reader.read(&mut buffer) {
                Ok(0) => {
                    // EOF - channel will be dropped, signaling completion
                    break;
                }
                Ok(n) => {
                    // Send bytes to the async task
                    let bytes = buffer[..n].to_vec();
                    if bytes_tx.send(bytes).is_err() {
                        break; // Channel closed
                    }
                }
                Err(_) => {
                    // Read error
                    break;
                }
            }
        }
    }

    /// Handle keyboard input events.
    ///
    /// Converts GPUI keystrokes to terminal escape sequences and writes them
    /// to the stdin writer. If a key handler is set and returns true, the event
    /// is consumed and not sent to the terminal.
    fn find_action(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        self.refocus_after_find = false;
        let handle = self.find_query.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.find_query.update(cx, |query, cx| query.select_all(cx));
        self.refresh_find(cx);
        cx.notify();
    }

    /// Shift+Enter in the panel's field: the previous hit, as the editor's
    /// find does.
    fn find_previous_action(
        &mut self,
        _: &FindPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_next(true, cx);
    }

    fn close_find(&mut self, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        self.find_open = false;
        self.find_matches.clear();
        self.find_current = 0;
        self.refocus_after_find = true;
        cx.notify();
    }

    fn toggle_find_option(&mut self, regex: bool, cx: &mut Context<Self>) {
        if regex {
            self.find_regex = !self.find_regex;
        } else {
            self.find_match_case = !self.find_match_case;
        }
        self.refresh_find(cx);
    }

    fn clear_action(&mut self, _: &Clear, _window: &mut Window, _cx: &mut Context<Self>) {
        self.write_pty(&[0x0c]);
    }

    /// BEL: a short border flash, the visual bell a pane can offer without a
    /// system sound.
    fn flash_bell(&mut self, cx: &mut Context<Self>) {
        if self.bell_flash {
            return;
        }
        self.bell_flash = true;
        cx.notify();
        self.bell_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(140))
                .await;
            this.update(cx, |view, cx| {
                view.bell_flash = false;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Starts the cursor's blink cycle once, when the config asks for it.
    fn ensure_blink(&mut self, cx: &mut Context<Self>) {
        if !self.config.cursor_blink || self.blink_task.is_some() {
            return;
        }
        self.blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let alive = this.update(cx, |view, cx| {
                    if !view.config.cursor_blink {
                        return false;
                    }
                    // Keystrokes hide the blink; the next tick brings the
                    // cursor back, so typing never feels like it lost it.
                    view.cursor_on = !view.cursor_on;
                    if view.state.mode().contains(TermMode::SHOW_CURSOR) {
                        cx.notify();
                    }
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        // The find panel owns the keyboard while it is open; its TextInput
        // handles typing, Enter, Escape and the arrows. Escape still closes
        // the panel when the grid took the keyboard back with a click.
        if self.find_open {
            if event.keystroke.key == "escape" {
                self.close_find(_cx);
                _cx.stop_propagation();
            }
            return;
        }
        // A keystroke always shows the cursor, even mid-blink.
        self.cursor_on = true;
        if event.keystroke.modifiers.platform && event.keystroke.key == "f" {
            self.find_action(&Find, _window, _cx);
            return;
        }
        // Option as Meta is the shell convention; with it off, Option types
        // the character the layout produced instead of ESC + key.
        if !self.config.option_as_meta
            && event.keystroke.modifiers.alt
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.platform
            && let Some(text) = event
                .keystroke
                .key_char
                .as_deref()
                .filter(|text| !text.is_empty())
        {
            self.send_text(text);
            _cx.stop_propagation();
            return;
        }
        // Shift with the page/arrow/home/end keys scrolls the viewport,
        // as it does in Zed's terminal; it is not text for the shell.
        if self.scroll_key(&event.keystroke, _cx) {
            return;
        }
        // Cmd-A selects the grid and its scrollback, so Cmd-C can copy it.
        if event.keystroke.modifiers.platform && event.keystroke.key == "a" {
            self.select_all(_cx);
            _cx.stop_propagation();
            return;
        }
        // Cmd-C copies the terminal's selection; with none, it is the
        // application's (a note's selection is not on screen here).
        let copy = event.keystroke.key == "c"
            && (event.keystroke.modifiers.platform
                || (!cfg!(target_os = "macos")
                    && event.keystroke.modifiers.control
                    && event.keystroke.modifiers.shift));
        if copy {
            self.copy_selection(_cx);
            _cx.stop_propagation();
            return;
        }
        // Cmd-V pastes into the shell, wrapped for programs that asked for
        // bracketed paste; the app's own paste binding is for notes.
        let paste = event.keystroke.key == "v"
            && (event.keystroke.modifiers.platform
                || (!cfg!(target_os = "macos")
                    && event.keystroke.modifiers.control
                    && event.keystroke.modifiers.shift));
        if paste {
            self.paste_from_clipboard(_cx);
            _cx.stop_propagation();
            return;
        }
        // Check if key handler wants to consume this event
        if let Some(ref handler) = self.key_handler
            && handler(event)
        {
            return; // Event consumed by handler
        }

        let mode = self.state.mode();
        let kind = if event.is_held {
            KeyEventKind::Repeat
        } else {
            KeyEventKind::Press
        };
        if let Some(bytes) = keystroke_to_bytes_for_event(&event.keystroke, mode, kind) {
            self.scroll_to_bottom();
            // Under `REPORT_ALL_KEYS_AS_ESC` the text has already been
            // reported as a kitty key event; the input handler's commit for
            // that same keystroke is a duplicate.
            if !bytes.is_empty()
                && mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC)
                && event
                    .keystroke
                    .key_char
                    .as_deref()
                    .is_some_and(|text| !text.is_empty())
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt
            {
                self.suppress_commit = event.keystroke.key_char.clone();
            }
            let mut writer = self.stdin_writer.lock();
            let _ = writer.write_all(&bytes);
            let _ = writer.flush();
            // The key belongs to the shell: Ctrl-W must not close the tab,
            // and the arrow keys must not walk the app's focus.
            _cx.stop_propagation();
        }
    }

    /// A kitty-protocol application that asked for event types also wants
    /// the release; without the protocol the release types nothing.
    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open {
            return;
        }
        let mode = self.state.mode();
        if !mode.contains(TermMode::REPORT_EVENT_TYPES) {
            return;
        }
        if let Some(bytes) =
            keystroke_to_bytes_for_event(&event.keystroke, mode, KeyEventKind::Release)
            && !bytes.is_empty()
        {
            self.write_pty(&bytes);
            cx.stop_propagation();
        }
    }

    /// Starts a text selection at the clicked cell.
    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Request focus when clicking the terminal
        window.focus(&self.focus_handle, cx);
        if event.button == MouseButton::Right && self.mouse_report_mode().is_none() {
            self.context_menu = Some(event.position);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        self.context_menu = None;
        // A click in the scrollbar strip scrolls there and starts a drag; it
        // never begins a text selection under the thumb.
        if event.button == MouseButton::Left
            && self.mouse_report_mode().is_none()
            && let Some(track) = self.scrollbar_track()
            && track.contains(&event.position)
        {
            let offset = self.scrollbar_offset_at(event.position.y, track);
            self.set_display_offset(offset);
            self.scrollbar_drag = true;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if event.modifiers.platform
            && let Some((target, _)) = self.link_span_at(event.position)
        {
            cx.emit(TerminalLink {
                target,
                cwd: self.cwd.clone(),
            });
            cx.stop_propagation();
            return;
        }
        if let Some((col, row)) = self.screen_cell(event.position)
            && self.mouse_report_mode().is_some()
            && !event.modifiers.shift
        {
            self.mouse_button = Some(event.button);
            self.report_mouse(Some(event.button), col, row, false, false, event.modifiers);
            cx.notify();
            return;
        }
        if event.button == MouseButton::Left
            && let Some(point) = self.point_at(event.position)
        {
            self.select_anchor = Some(point);
            self.select_type = match event.click_count {
                2 => SelectionType::Semantic,
                n if n >= 3 => SelectionType::Lines,
                _ if event.modifiers.alt => SelectionType::Block,
                _ => SelectionType::Simple,
            };
            self.state.with_term_mut(|term| {
                term.selection = Some(Selection::new(self.select_type, point, Side::Left));
            });
            self.selecting = true;
            cx.notify();
        }
    }

    /// Ends a drag selection; the range stays for Cmd-C.
    fn on_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((col, row)) = self.screen_cell(event.position)
            && self.mouse_report_mode().is_some()
            && !event.modifiers.shift
        {
            self.report_mouse(Some(event.button), col, row, true, false, event.modifiers);
            self.mouse_button = None;
            cx.notify();
            return;
        }
        self.selecting = false;
        self.scrollbar_drag = false;
        if self.config.copy_on_select
            && let Some(text) = self.state.with_term(|term| term.selection_to_string())
            && !text.is_empty()
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Extends the selection to the cell under the pointer.
    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar_drag
            && event.pressed_button == Some(MouseButton::Left)
            && let Some(track) = self.scrollbar_track()
        {
            let offset = self.scrollbar_offset_at(event.position.y, track);
            self.set_display_offset(offset);
            cx.notify();
            return;
        }
        if let Some(mode) = self.mouse_report_mode()
            && !event.modifiers.shift
            && mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
            && (mode.contains(TermMode::MOUSE_MOTION) || event.pressed_button.is_some())
            && let Some((col, row)) = self.screen_cell(event.position)
        {
            if self.last_mouse_cell != Some((col, row)) {
                self.last_mouse_cell = Some((col, row));
                self.report_mouse(
                    event.pressed_button.or(self.mouse_button),
                    col,
                    row,
                    false,
                    true,
                    event.modifiers,
                );
            }
            return;
        }
        if !self.selecting {
            // Every move over a link redraws the underline and anchor for
            // the tooltip; leaving it takes them away.
            let hover = event
                .pressed_button
                .is_none()
                .then(|| self.link_span_at(event.position))
                .flatten();
            let changed = match (&hover, &self.hover_link) {
                (None, None) => false,
                (Some((target, span)), Some((shown, shown_span))) => {
                    target != shown || span != shown_span
                }
                _ => true,
            };
            self.hover_point = event.position;
            if changed {
                self.hover_link = hover;
                cx.notify();
            }
            return;
        }
        if let Some(point) = self.point_at(event.position)
            && let Some(anchor) = self.select_anchor
        {
            // Build the range from the ordered pair on every move: dragging
            // up or left must include the anchor cell, not trim it away.
            let (start, end) = if anchor <= point {
                (anchor, point)
            } else {
                (point, anchor)
            };
            self.state.with_term_mut(|term| {
                let mut selection = Selection::new(self.select_type, start, Side::Left);
                selection.update(end, Side::Right);
                term.selection = Some(selection);
            });
            cx.notify();
        }
    }

    /// The terminal modes an application set for mouse reporting.
    fn mouse_report_mode(&self) -> Option<TermMode> {
        let mode = self.state.mode();
        mode.intersects(
            TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
        )
        .then_some(mode)
    }

    /// The link under a window position and the cells it covers: an OSC 8
    /// URI, a URL, or a path-like token in the text.
    fn link_span_at(&self, position: Point<Pixels>) -> Option<(String, SearchMatch)> {
        let (col, row) = self.screen_cell(position)?;
        let offset = self
            .state
            .with_term(|term| term.grid().display_offset() as i32);
        let line = Line(row as i32 - offset);
        let point = AlacPoint::new(line, Column(col));
        let linked_uri = self.state.with_term(|term| {
            term.grid()[point]
                .hyperlink()
                .map(|link| link.uri().to_string())
        });
        if let Some(uri) = linked_uri {
            // An OSC 8 link is a run of cells carrying the same URI.
            let columns = self.state.with_term(|term| term.last_column().0 + 1);
            let linked = |col: usize| {
                self.state.with_term(|term| {
                    term.grid()[AlacPoint::new(line, Column(col))]
                        .hyperlink()
                        .is_some_and(|link| link.uri() == uri)
                })
            };
            let mut start = col;
            while start > 0 && linked(start - 1) {
                start -= 1;
            }
            let mut end = col + 1;
            while end < columns && linked(end) {
                end += 1;
            }
            return Some((
                uri,
                SearchMatch {
                    line: line.0,
                    start_col: start,
                    end_col: end,
                },
            ));
        }
        let (text, columns) = self.line_text(line);
        let char_ix = columns.iter().rposition(|start| *start <= col)?;
        let (token, start_char, end_char) = link_token_span_at(&text, char_ix)?;
        let start_col = *columns.get(start_char)?;
        let end_col = columns
            .get(end_char.saturating_sub(1))
            .map(|col| col + 1)
            .unwrap_or(start_col + 1);
        Some((
            token,
            SearchMatch {
                line: line.0,
                start_col,
                end_col,
            },
        ))
    }

    /// Reads OSC 7 (`ESC ] 7 ; file://host/path BEL|ST`) out of a PTY read
    /// without disturbing the grid. A sequence may straddle two reads, so
    /// the tail waits for the terminator.
    fn observe_cwd(&mut self, bytes: &[u8]) {
        const MARK: &[u8] = b"\x1b]7;";
        self.osc_tail.extend_from_slice(bytes);
        loop {
            let Some(start) = find_subslice(&self.osc_tail, MARK) else {
                // Keep only a suffix that could be the start of the marker.
                let keep = (1..MARK.len())
                    .rev()
                    .find(|n| {
                        self.osc_tail.len() >= *n
                            && MARK.starts_with(&self.osc_tail[self.osc_tail.len() - n..])
                    })
                    .unwrap_or(0);
                let drop = self.osc_tail.len() - keep;
                self.osc_tail.drain(..drop);
                return;
            };
            let body_start = start + MARK.len();
            let terminated =
                self.osc_tail[body_start..]
                    .iter()
                    .enumerate()
                    .find_map(|(ix, byte)| match byte {
                        0x07 => Some((body_start + ix, body_start + ix + 1)),
                        0x1b if self.osc_tail.get(body_start + ix + 1) == Some(&b'\\') => {
                            Some((body_start + ix, body_start + ix + 2))
                        }
                        _ => None,
                    });
            let Some((body_end, next)) = terminated else {
                self.osc_tail.drain(..start);
                // A runaway sequence must not grow without bound.
                if self.osc_tail.len() > 4096 {
                    let drop = self.osc_tail.len() - 4096;
                    self.osc_tail.drain(..drop);
                }
                return;
            };
            let body = String::from_utf8_lossy(&self.osc_tail[body_start..body_end]).into_owned();
            if let Some(cwd) = parse_osc7_path(&body) {
                self.cwd = Some(cwd);
            }
            self.osc_tail.drain(..next);
        }
    }

    /// A content line's cells as text, with each character's grid column.
    fn line_text(&self, line: Line) -> (String, Vec<usize>) {
        let columns = self.state.with_term(|term| term.last_column().0 + 1);
        let mut text = String::new();
        let mut columns_of_char = Vec::new();
        self.state.with_term(|term| {
            let grid = term.grid();
            for col in 0..columns {
                let cell = &grid[AlacPoint::new(line, Column(col))];
                if cell.flags.intersects(
                    alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER
                        | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER,
                ) {
                    continue;
                }
                text.push(cell.c);
                columns_of_char.push(col);
            }
        });
        (text, columns_of_char)
    }

    /// The editor's find panel, floating at the terminal's top right: the
    /// same UI shell and text field as the editor's search.
    fn render_find_panel(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let query_empty = self.find_query.read(cx).text().is_empty();
        let count = self.find_matches.len();
        let status: SharedString = match (query_empty, count) {
            (true, _) => String::new(),
            (false, 0) => "No results".to_string(),
            (false, _) => format!("{} of {count}", self.find_current + 1),
        }
        .into();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_2()
                .py_px()
                .rounded_sm()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme.tokens.text_normal)
                .bg(theme.tokens.background_modifier_hover)
                .hover(|s| s.bg(theme.tokens.background_modifier_hover))
                .child(label)
        };
        let toggle = |id: &'static str, label: &'static str, on: bool| {
            let b = button(id, label).border_1();
            if on {
                b.border_color(theme.tokens.interactive_accent)
                    .text_color(theme.tokens.interactive_accent)
            } else {
                b.border_color(theme.tokens.background_modifier_hover)
            }
        };
        div()
            .id("terminal-find-panel")
            .occlude()
            .absolute()
            .top(px(8.))
            .right(px(24.))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(theme.tokens.background_secondary)
            .border_1()
            .border_color(theme.tokens.background_modifier_border)
            .font_family(theme.ui_font.clone())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(div().w(px(220.)).child(self.find_query.clone()))
                    .child(
                        toggle("terminal-find-match-case", "Aa", self.find_match_case).on_click(
                            cx.listener(|this, _, _, cx| this.toggle_find_option(false, cx)),
                        ),
                    )
                    .child(
                        toggle("terminal-find-regex", ".*", self.find_regex).on_click(
                            cx.listener(|this, _, _, cx| this.toggle_find_option(true, cx)),
                        ),
                    )
                    .child(
                        button("terminal-find-prev", "↑")
                            .on_click(cx.listener(|this, _, _, cx| this.find_next(true, cx))),
                    )
                    .child(
                        button("terminal-find-next", "↓")
                            .on_click(cx.listener(|this, _, _, cx| this.find_next(false, cx))),
                    )
                    .child(
                        div()
                            .min_w(px(70.))
                            .text_size(px(11.))
                            .text_color(theme.tokens.text_muted)
                            .child(status),
                    )
                    .child(
                        button("terminal-find-close", "×")
                            .on_click(cx.listener(|this, _, _, cx| this.close_find(cx))),
                    ),
            )
    }

    /// The link target under the pointer, as a floating label; long paths
    /// wrap instead of running off the pane.
    fn render_link_tooltip(
        &self,
        target: &str,
        point: Point<Pixels>,
        theme: &Theme,
    ) -> gpui::AnyElement {
        let origin = self
            .viewport_bounds
            .get()
            .map(|bounds| bounds.origin)
            .unwrap_or_default();
        div()
            .absolute()
            .left(point.x - origin.x + px(12.))
            .top(point.y - origin.y + px(18.))
            .max_w(px(420.))
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(theme.tokens.background_secondary)
            .border_1()
            .border_color(theme.tokens.background_modifier_border)
            .font_family(theme.ui_font.clone())
            .text_size(px(11.))
            .text_color(theme.tokens.text_normal)
            .child(target.to_string())
            .into_any_element()
    }

    /// The right-click menu: the terminal's own clipboard, selection,
    /// search and clear actions.
    fn render_context_menu(
        &mut self,
        point: Point<Pixels>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let origin = self
            .viewport_bounds
            .get()
            .map(|bounds| bounds.origin)
            .unwrap_or_default();
        let entry = |label: &'static str| {
            div()
                .id(label)
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme.tokens.text_normal)
                .hover(|style| style.bg(theme.tokens.background_modifier_hover))
                .child(label)
        };
        div()
            .id("terminal-context-menu")
            .occlude()
            .absolute()
            .left(point.x - origin.x)
            .top(point.y - origin.y)
            .flex()
            .flex_col()
            .min_w(px(160.))
            .p_1()
            .rounded_md()
            .bg(theme.tokens.background_secondary)
            .border_1()
            .border_color(theme.tokens.background_modifier_border)
            .font_family(theme.ui_font.clone())
            .child(entry("Copy").on_click(cx.listener(|this, _, _, cx| {
                this.copy_selection(cx);
                this.context_menu = None;
                cx.notify();
            })))
            .child(entry("Paste").on_click(cx.listener(|this, _, _, cx| {
                this.paste_from_clipboard(cx);
                this.context_menu = None;
                cx.notify();
            })))
            .child(entry("Select All").on_click(cx.listener(|this, _, _, cx| {
                this.select_all(cx);
                this.context_menu = None;
                cx.notify();
            })))
            .child(entry("Find").on_click(cx.listener(|this, _, window, cx| {
                this.context_menu = None;
                this.find_action(&Find, window, cx);
            })))
            .child(entry("Clear").on_click(cx.listener(|this, _, window, cx| {
                this.context_menu = None;
                this.clear_action(&Clear, window, cx);
            })))
            .into_any_element()
    }

    /// Recomputes every find hit and scrolls the first into view.
    fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query.read(cx).text().to_string();
        self.find_matches = if query.is_empty() {
            Vec::new()
        } else {
            self.search_grid(&query)
        };
        self.find_current = 0;
        self.scroll_to_find();
        cx.notify();
    }

    fn search_grid(&self, query: &str) -> Vec<SearchMatch> {
        const MAX_MATCHES: usize = 2_000;
        if query.is_empty() {
            return Vec::new();
        }
        let regex = if self.find_regex {
            regex::RegexBuilder::new(query)
                .case_insensitive(!self.find_match_case)
                .build()
                .ok()
        } else {
            None
        };
        if self.find_regex && regex.is_none() {
            return Vec::new();
        }
        let needle = if self.find_match_case {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        let (top, bottom) = self
            .state
            .with_term(|term| (term.topmost_line(), term.bottommost_line()));
        let mut matches = Vec::new();
        for line_ix in top.0..=bottom.0 {
            let (text, columns_of_char) = self.line_text(Line(line_ix));
            let found: Vec<(usize, usize)> = if let Some(regex) = &regex {
                regex
                    .find_iter(&text)
                    .filter(|m| m.start() != m.end())
                    .map(|m| (m.start(), m.end()))
                    .collect()
            } else {
                let haystack = if self.find_match_case {
                    text.clone()
                } else {
                    text.to_lowercase()
                };
                haystack
                    .match_indices(&needle)
                    .map(|(start, matched)| (start, start + matched.len()))
                    .collect()
            };
            for (start_byte, end_byte) in found {
                let char_ix = text[..start_byte].chars().count();
                let char_len = text[start_byte..end_byte].chars().count().max(1);
                let Some(&start_col) = columns_of_char.get(char_ix) else {
                    continue;
                };
                let end_col = columns_of_char
                    .get(char_ix + char_len - 1)
                    .map(|col| col + 1)
                    .unwrap_or(start_col + 1);
                matches.push(SearchMatch {
                    line: line_ix,
                    start_col,
                    end_col,
                });
                if matches.len() >= MAX_MATCHES {
                    return matches;
                }
            }
        }
        matches
    }

    fn find_next(&mut self, back: bool, cx: &mut Context<Self>) {
        if self.find_matches.is_empty() {
            return;
        }
        let n = self.find_matches.len();
        self.find_current = if back {
            (self.find_current + n - 1) % n
        } else {
            (self.find_current + 1) % n
        };
        self.scroll_to_find();
        cx.notify();
    }

    fn scroll_to_find(&mut self) {
        let Some(hit) = self.find_matches.get(self.find_current).copied() else {
            return;
        };
        let rows = self.state.rows() as i32;
        let current = self
            .state
            .with_term(|term| term.grid().display_offset() as i32);
        let screen = hit.line + current;
        let desired = if hit.line < 0 {
            (-hit.line + rows / 3).max(0)
        } else if screen < 0 || screen >= rows {
            (-hit.line).max(0)
        } else {
            current
        };
        let delta = desired - current;
        if delta != 0 {
            self.state
                .with_term_mut(|term| term.scroll_display(Scroll::Delta(delta)));
        }
    }

    /// The screen cell under a pointer position.
    fn screen_cell(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let bounds = self.viewport_bounds.get()?;
        if !bounds.contains(&position) {
            return None;
        }
        let (cell_width, cell_height) = self.viewport_cell.get();
        if cell_width <= px(0.) || cell_height <= px(0.) {
            return None;
        }
        let x = position.x - bounds.origin.x - self.config.padding.left;
        let y = position.y - bounds.origin.y - self.config.padding.top;
        let col = (f32::from(x) / f32::from(cell_width)).floor().max(0.) as usize;
        let row = (f32::from(y) / f32::from(cell_height)).floor().max(0.) as usize;
        Some((
            col.min(self.state.cols().saturating_sub(1)),
            row.min(self.state.rows().saturating_sub(1)),
        ))
    }

    /// Sends an xterm mouse report (SGR when the application asked for it,
    /// X10 otherwise).
    fn report_mouse(
        &mut self,
        button: Option<MouseButton>,
        col: usize,
        row: usize,
        release: bool,
        motion: bool,
        modifiers: Modifiers,
    ) {
        let Some(mode) = self.mouse_report_mode() else {
            return;
        };
        let button_code = match button {
            Some(MouseButton::Left) => 0,
            Some(MouseButton::Middle) => 1,
            Some(MouseButton::Right) => 2,
            Some(_) | None => 3,
        };
        let mods = (u32::from(modifiers.shift))
            + (u32::from(modifiers.alt) << 1)
            + (u32::from(modifiers.control) << 2);
        let code = if motion {
            32 + button_code
        } else {
            button_code
        } + mods;
        let mut bytes = Vec::new();
        if mode.contains(TermMode::SGR_MOUSE) {
            bytes.extend_from_slice(
                format!(
                    "\x1b[<{};{};{}{}",
                    code,
                    col + 1,
                    row + 1,
                    if release { 'm' } else { 'M' }
                )
                .as_bytes(),
            );
        } else {
            let code = if release { 3 + mods } else { code };
            bytes.extend_from_slice(&[0x1b, b'[', b'M']);
            bytes.push((32 + code).min(255) as u8);
            bytes.push((33 + col).min(255) as u8);
            bytes.push((33 + row).min(255) as u8);
        }
        let mut writer = self.stdin_writer.lock();
        let _ = writer.write_all(&bytes);
        let _ = writer.flush();
    }

    /// Sends a wheel report to an application that asked for mouse events.
    fn report_wheel(&mut self, up: bool, col: usize, row: usize, modifiers: Modifiers) {
        let Some(mode) = self.mouse_report_mode() else {
            return;
        };
        let mods = (u32::from(modifiers.shift))
            + (u32::from(modifiers.alt) << 1)
            + (u32::from(modifiers.control) << 2);
        let code = if up { 64 } else { 65 } + mods;
        let mut bytes = Vec::new();
        if mode.contains(TermMode::SGR_MOUSE) {
            bytes.extend_from_slice(format!("\x1b[<{};{};{}M", code, col + 1, row + 1).as_bytes());
        } else {
            bytes.extend_from_slice(&[0x1b, b'[', b'M']);
            bytes.push((32 + code).min(255) as u8);
            bytes.push((33 + col).min(255) as u8);
            bytes.push((33 + row).min(255) as u8);
        }
        let mut writer = self.stdin_writer.lock();
        let _ = writer.write_all(&bytes);
        let _ = writer.flush();
    }

    /// Shift-PageUp/Down, Shift-Up/Down and Shift-Home/End scroll the
    /// display, as Zed's terminal does; the shell never sees them.
    fn scroll_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if keystroke.modifiers.control
            || keystroke.modifiers.alt
            || !(keystroke.modifiers.shift || keystroke.modifiers.platform)
        {
            return false;
        }
        let shift = keystroke.modifiers.shift;
        let scroll = match keystroke.key.as_str() {
            "pageup" if shift => Scroll::PageUp,
            "pagedown" if shift => Scroll::PageDown,
            "up" if shift => Scroll::Delta(1),
            "down" if shift => Scroll::Delta(-1),
            "up" => Scroll::PageUp,
            "down" => Scroll::PageDown,
            "home" => Scroll::Top,
            "end" => Scroll::Bottom,
            _ => return false,
        };
        self.state.with_term_mut(|term| term.scroll_display(scroll));
        cx.notify();
        cx.stop_propagation();
        true
    }

    /// Puts the selection on the clipboard; nothing when there is none.
    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.state.with_term(|term| term.selection_to_string()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.notify();
        }
    }

    /// Pastes the clipboard: text (bracketed when asked), dropped paths, or
    /// a raw Ctrl-V so a TUI agent can read an image itself.
    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            match item.entries().first() {
                Some(ClipboardEntry::Image(image)) if !image.bytes.is_empty() => {
                    // Let a TUI agent (Claude Code, Codex, ...) read the
                    // image from the OS clipboard itself: Zed forwards a
                    // raw Ctrl-V and the agent shows its own `[Image 1]`.
                    self.write_pty(&[0x16]);
                }
                Some(ClipboardEntry::ExternalPaths(paths)) => {
                    self.paste_text(&quoted_paths(paths));
                }
                _ => {
                    if let Some(text) = item.text() {
                        self.paste_text(&text);
                    }
                }
            }
            cx.notify();
        }
    }

    /// Selects every cell the terminal still has, scrollback included.
    fn select_all(&mut self, cx: &mut Context<Self>) {
        let (top, bottom, last) = self.state.with_term(|term| {
            (
                term.topmost_line(),
                term.bottommost_line(),
                term.last_column(),
            )
        });
        let mut selection = Selection::new(
            SelectionType::Simple,
            AlacPoint::new(top, Column(0)),
            Side::Left,
        );
        selection.update(AlacPoint::new(bottom, last), Side::Right);
        self.state
            .with_term_mut(|term| term.selection = Some(selection));
        cx.notify();
    }

    /// The scrollbar strip down the right edge, when there is history.
    fn scrollbar_track(&self) -> Option<Bounds<Pixels>> {
        let history = self.state.with_term(|term| term.grid().history_size());
        if history == 0 {
            return None;
        }
        let bounds = self.viewport_bounds.get()?;
        Some(Bounds {
            origin: Point {
                x: bounds.origin.x + bounds.size.width - px(8.),
                y: bounds.origin.y + self.config.padding.top,
            },
            size: Size {
                width: px(8.),
                height: bounds.size.height - self.config.padding.top - self.config.padding.bottom,
            },
        })
    }

    /// The display offset a pointer at `y` in the track asks for: the top
    /// of the track is the top of history, the bottom is the live screen.
    fn scrollbar_offset_at(&self, y: Pixels, track: Bounds<Pixels>) -> usize {
        let history = self.state.with_term(|term| term.grid().history_size());
        if history == 0 || track.size.height <= px(0.) {
            return 0;
        }
        let from_top = ((y - track.origin.y) / track.size.height).clamp(0., 1.);
        (history as f32 * (1.0 - from_top)).round() as usize
    }

    /// Scrolls to an absolute history offset, expressed as a delta from
    /// where the viewport is now.
    fn set_display_offset(&mut self, offset: usize) {
        let current = self
            .state
            .with_term(|term| term.grid().display_offset() as i32);
        let delta = current - offset as i32;
        if delta != 0 {
            self.state
                .with_term_mut(|term| term.scroll_display(Scroll::Delta(delta)));
        }
    }

    /// The grid point under a window position, or `None` outside the grid.
    fn point_at(&self, position: Point<Pixels>) -> Option<AlacPoint> {
        let bounds = self.viewport_bounds.get()?;
        if !bounds.contains(&position) {
            return None;
        }
        let (cell_width, cell_height) = self.viewport_cell.get();
        if cell_width <= px(0.) || cell_height <= px(0.) {
            return None;
        }
        let padding = self.config.padding;
        let x = position.x - bounds.origin.x - padding.left;
        let y = position.y - bounds.origin.y - padding.top;
        let col = (f32::from(x) / f32::from(cell_width)).floor().max(0.) as usize;
        let row = (f32::from(y) / f32::from(cell_height)).floor().max(0.) as usize;
        let (cols, rows) = (self.state.cols(), self.state.rows());
        let offset = self
            .state
            .with_term(|term| term.grid().display_offset() as i32);
        Some(AlacPoint::new(
            Line(row.min(rows.saturating_sub(1)) as i32 - offset),
            Column(col.min(cols.saturating_sub(1))),
        ))
    }

    /// Handle scroll events.
    ///
    /// Currently a placeholder for future scrollback support.
    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if !event.modifiers.shift
            && self.mouse_report_mode().is_some()
            && let Some((col, row)) = self.screen_cell(event.position)
        {
            let up = f32::from(event.delta.pixel_delta(px(20.)).y) > 0.0;
            self.report_wheel(up, col, row, event.modifiers);
            return;
        }
        let line_height = self.renderer.cell_height;
        if line_height <= px(0.) {
            return;
        }
        let delta = event.delta.pixel_delta(line_height).y;
        let lines = (f32::from(delta) / f32::from(line_height)).round() as i32;
        if lines != 0 {
            self.state
                .with_term_mut(|term| term.scroll_display(Scroll::Delta(lines)));
            _cx.notify();
        }
    }

    /// Writes text to the shell as if it had been typed, for the control
    /// socket and for hosts that hand the terminal a string.
    pub fn send_text(&mut self, text: &str) {
        // Typing means the reader is done looking at history.
        self.scroll_to_bottom();
        let mut writer = self.stdin_writer.lock();
        let _ = writer.write_all(text.as_bytes());
        let _ = writer.flush();
    }

    fn scroll_to_bottom(&mut self) {
        self.state
            .with_term_mut(|term| term.scroll_display(Scroll::Bottom));
    }

    /// Writes a terminal protocol reply (a mode report, a colour response)
    /// back to the application on the PTY.
    fn write_pty(&self, bytes: &[u8]) {
        let mut writer = self.stdin_writer.lock();
        let _ = writer.write_all(bytes);
        let _ = writer.flush();
    }

    /// Pastes text, bracketed when the running application asked for it.
    fn paste_text(&mut self, text: &str) {
        self.scroll_to_bottom();
        let bracketed = self.state.mode().contains(TermMode::BRACKETED_PASTE);
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        self.write_pty(&bytes);
    }

    /// Files dragged over the grid: remember them for the mouse-up that
    /// releases the drag. gpui's `on_drop` is unreliable when the pointer's
    /// hover state is stale after a keystroke, so the release is caught by
    /// bounds in the paint listener as the editor does.
    fn drag_paths_over(
        &mut self,
        event: &DragMoveEvent<ExternalPaths>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drop_hover = event
            .bounds
            .contains(&event.event.position)
            .then(|| event.drag(cx).paths().to_vec());
    }

    /// Files dropped from Finder: write their shell-quoted paths, as Zed
    /// does, so a shell or a TUI agent can take them as arguments.
    fn drop_paths(
        &mut self,
        paths: &[std::path::PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.paste_text(&quoted_path_list(paths));
        cx.notify();
    }

    /// Process pending terminal events.
    ///
    /// This method drains all available events from the event receiver
    /// and handles them appropriately. Note: bytes are processed in the
    /// async reader task, not here.
    fn process_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Process terminal events (from alacritty event proxy)
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                TerminalEvent::Wakeup => {
                    // Terminal has new content - already handled by async task
                }
                TerminalEvent::Bell => {
                    if self.config.bell {
                        self.flash_bell(cx);
                    }
                    if let Some(ref callback) = self.bell_callback {
                        callback(window, cx);
                    }
                }
                TerminalEvent::Title(title) => {
                    if let Some(ref callback) = self.title_callback {
                        callback(window, cx, &title);
                    }
                    cx.emit(TerminalTitle(title));
                }
                TerminalEvent::ClipboardStore(text) => {
                    if let Some(ref callback) = self.clipboard_store_callback {
                        callback(window, cx, &text);
                    }
                }
                TerminalEvent::ClipboardLoad => {
                    // Terminal wants to load data from clipboard
                    // TODO: Implement clipboard integration
                }
                TerminalEvent::Exit => {
                    if let Some(ref callback) = self.exit_callback {
                        callback(window, cx);
                    }
                }
                TerminalEvent::PtyWrite(text) => self.write_pty(text.as_bytes()),
                TerminalEvent::ColorRequest(index, format) => {
                    let palette = &self.config.colors;
                    let color = if index == NamedColor::Foreground as usize {
                        palette.foreground()
                    } else if index == NamedColor::Background as usize {
                        palette.background()
                    } else if index < 16 {
                        palette.ansi_colors()[index]
                    } else if index < 256 {
                        palette.extended_colors()[index]
                    } else {
                        palette.foreground()
                    };
                    let c = gpui::Rgba::from(color);
                    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
                    let reply = format(alacritty_terminal::vte::ansi::Rgb {
                        r: byte(c.r),
                        g: byte(c.g),
                        b: byte(c.b),
                    });
                    self.write_pty(reply.as_bytes());
                }
                TerminalEvent::TextAreaSizeRequest(format) => {
                    let (cell_width, cell_height) = self.viewport_cell.get();
                    let reply = format(alacritty_terminal::event::WindowSize {
                        num_lines: self.state.rows() as u16,
                        num_cols: self.state.cols() as u16,
                        cell_width: f32::from(cell_width).round() as u16,
                        cell_height: f32::from(cell_height).round() as u16,
                    });
                    self.write_pty(reply.as_bytes());
                }
            }
        }
    }

    /// Get the current terminal dimensions.
    ///
    /// # Returns
    ///
    /// A tuple of (columns, rows).
    pub fn dimensions(&self) -> (usize, usize) {
        (self.state.cols(), self.state.rows())
    }

    /// Resize the terminal to new dimensions.
    ///
    /// This method should be called when the terminal view size changes.
    /// It updates the internal grid and notifies the terminal process of the new size.
    ///
    /// # Arguments
    ///
    /// * `cols` - New number of columns
    /// * `rows` - New number of rows
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.state.resize(cols, rows);
    }

    /// Get the current terminal configuration.
    ///
    /// # Returns
    ///
    /// A reference to the current configuration.
    pub fn config(&self) -> &TerminalConfig {
        &self.config
    }

    /// Get the focus handle for this terminal view.
    ///
    /// # Returns
    ///
    /// A reference to the focus handle.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Update the terminal configuration.
    ///
    /// This method updates the terminal's configuration, including font settings,
    /// padding, and color palette. Changes take effect on the next render.
    ///
    /// # Arguments
    ///
    /// * `config` - The new configuration to apply
    /// * `cx` - The context for triggering a repaint
    pub fn update_config(&mut self, config: TerminalConfig, cx: &mut Context<Self>) {
        // Update renderer with new font settings and palette
        self.renderer.font_family = config.font_family.clone();
        self.renderer.font_size = config.font_size;
        self.renderer.line_height_multiplier = config.line_height_multiplier;
        self.renderer.palette = config.colors.clone();

        // Store the new config
        self.config = config;

        // Trigger a repaint - cell dimensions will be recalculated via measure_cell()
        cx.notify();
    }

    /// Calculate terminal dimensions from pixel bounds and cell size.
    ///
    /// Helper method to determine how many columns and rows fit in the given bounds.
    #[allow(dead_code)]
    fn calculate_dimensions(&self, bounds: Bounds<Pixels>) -> (usize, usize) {
        let width_f32: f32 = bounds.size.width.into();
        let height_f32: f32 = bounds.size.height.into();
        let cell_width_f32: f32 = self.renderer.cell_width.into();
        let cell_height_f32: f32 = self.renderer.cell_height.into();

        let cols = ((width_f32 / cell_width_f32) as usize).max(1);
        let rows = ((height_f32 / cell_height_f32) as usize).max(1);
        (cols, rows)
    }
}

/// The URL or path-like token around character `char_ix` in a line, with the
/// character range it occupies (so the hover underline can cover it).
fn link_token_span_at(text: &str, char_ix: usize) -> Option<(String, usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    if char_ix >= chars.len() {
        return None;
    }
    let is_separator =
        |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '|' | '`');
    let mut start = char_ix;
    while start > 0 && !is_separator(chars[start - 1]) {
        start -= 1;
    }
    let mut end = char_ix + 1;
    while end < chars.len() && !is_separator(chars[end]) {
        end += 1;
    }
    let is_edge = |c: char| {
        matches!(
            c,
            '(' | '[' | '{' | '<' | '"' | '\'' | '`' | ']' | ')' | '}' | '>'
        )
    };
    while start < end && is_edge(chars[start]) {
        start += 1;
    }
    while end > start && (is_edge(chars[end - 1]) || matches!(chars[end - 1], ',')) {
        end -= 1;
    }
    while end > start && matches!(chars[end - 1], '.' | ';' | ':' | '!' | '?') {
        end -= 1;
    }
    if start >= end {
        return None;
    }
    let token: String = chars[start..end].iter().collect();
    let scheme = ["http://", "https://", "file://", "mailto:", "obsidian://"];
    if scheme.iter().any(|prefix| token.starts_with(prefix)) {
        return Some((token, start, end));
    }
    let has_path_shape = token.starts_with('/')
        || token.starts_with("~/")
        || token.starts_with("./")
        || token.starts_with("../");
    let known_file = token.rsplit('.').next().is_some_and(|ext| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "md" | "canvas"
                | "pdf"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "svg"
                | "webp"
                | "txt"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "rs"
                | "py"
                | "js"
                | "ts"
        )
    });
    ((has_path_shape || (token.contains('/') && known_file))
        && !token.contains(char::is_whitespace))
    .then_some((token, start, end))
}

/// [`link_token_span_at`] without the range.
#[cfg(test)]
fn link_token_at(text: &str, char_ix: usize) -> Option<String> {
    link_token_span_at(text, char_ix).map(|(token, _, _)| token)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The local path in an OSC 7 payload (`file://host/path`, percent-encoded).
fn parse_osc7_path(body: &str) -> Option<std::path::PathBuf> {
    let rest = body.strip_prefix("file://").unwrap_or(body);
    let slash = rest.find('/')?;
    let encoded = &rest[slash..];
    let decoded = percent_decode(encoded);
    // `file:///C:/dir` on Windows: the leading slash is the URL's, not part
    // of the drive path.
    let path = if cfg!(windows) && decoded.len() > 2 && decoded.as_bytes()[0] == b'/' {
        let bytes = decoded.as_bytes();
        if bytes[2] == b':' {
            decoded[1..].to_string()
        } else {
            decoded
        }
    } else {
        decoded
    };
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%'
            && ix + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[ix + 1]), hex_value(bytes[ix + 2]))
        {
            out.push(high * 16 + low);
            ix += 3;
        } else {
            out.push(bytes[ix]);
            ix += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Paths shell-quoted, with a leading and trailing space, as Zed sends
/// files dropped on a terminal.
fn quoted_path_list(paths: &[std::path::PathBuf]) -> String {
    let mut text = String::new();
    for path in paths {
        text.push(' ');
        text.push_str(&format!("{path:?}"));
    }
    text.push(' ');
    text
}

fn quoted_paths(paths: &ExternalPaths) -> String {
    quoted_path_list(paths.paths())
}

fn trace_ime(kind: &str, text: &str) {
    if std::env::var_os("TFT_TERM_TRACE").is_some() {
        eprintln!("terminal ime {kind}: {text:?}");
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // An alternate-screen application draws its own text; there is no
        // local selection for the IME to anchor to.
        if self.state.mode().contains(TermMode::ALT_SCREEN) {
            None
        } else {
            Some(UTF16Selection {
                range: 0..0,
                reversed: false,
            })
        }
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.marked_text
            .as_ref()
            .filter(|text| !text.is_empty())
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_text.take().is_some() {
            trace_ime("unmark", "");
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace_ime("commit", text);
        self.marked_text = None;
        // A kitty report-all application already received this keystroke as
        // an escape sequence; the input method's duplicate commit is dropped.
        if self.suppress_commit.take().as_deref() == Some(text) {
            cx.notify();
            return;
        }
        if !text.is_empty() {
            self.send_text(text);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace_ime("mark", new_text);
        self.marked_text = Some(new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (cell_width, cell_height) = self.viewport_cell.get();
        if cell_width <= px(0.) || cell_height <= px(0.) {
            return None;
        }
        let offset = self
            .state
            .with_term(|term| term.grid().display_offset() as i32);
        let cursor = self.state.with_term(|term| term.grid().cursor.point);
        let row = (cursor.line.0 - offset).max(0) as f32;
        let origin = Point {
            x: element_bounds.origin.x
                + self.config.padding.left
                + cell_width * cursor.column.0 as f32,
            y: element_bounds.origin.y + self.config.padding.top + cell_height * row,
        };
        Some(Bounds::new(origin, size(cell_width, cell_height)))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Process any pending events
        self.process_events(window, cx);
        self.ensure_blink(cx);

        // Get terminal state and renderer for rendering
        let state_arc = self.state.term_arc();
        let renderer = self.renderer.clone();
        let resize_callback = self.resize_callback.clone();
        let padding = self.config.padding;
        let viewport_bounds = self.viewport_bounds.clone();
        let viewport_cell = self.viewport_cell.clone();
        let input_entity = cx.entity();
        let input_focus = self.focus_handle.clone();
        if std::mem::take(&mut self.refocus_after_find) {
            window.focus(&self.focus_handle, cx);
        }
        let theme = cx
            .try_global::<Theme>()
            .cloned()
            .unwrap_or_else(Theme::dark);
        let tooltip = self
            .hover_link
            .as_ref()
            .map(|(target, _)| (target.clone(), self.hover_point));
        let context_menu = self.context_menu;
        let find_panel = self
            .find_open
            .then(|| self.render_find_panel(&theme, cx).into_any_element());
        let hovering = self.hover_link.is_some();
        let tooltip_element = tooltip
            .as_ref()
            .map(|(target, point)| self.render_link_tooltip(target, *point, &theme));
        let menu_element = context_menu.map(|point| self.render_context_menu(point, &theme, cx));
        let marked_text = self.marked_text.clone();
        let find_matches = self.find_matches.clone();
        let find_current = self.find_current;
        let marked_font = self.config.font_family.clone();
        let marked_size = self.config.font_size;
        let marked_color = self.config.colors.foreground();
        let hover_match = self.hover_link.as_ref().map(|(_, span)| *span);
        let cursor_visible = !self.config.cursor_blink || self.cursor_on;
        let scrollbar_color = self.config.colors.foreground().opacity(0.35);

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(self.config.colors.background())
            .when(self.bell_flash, |div| {
                div.border_1().border_color(theme.tokens.color_red)
            })
            .cursor_text()
            .when(hovering, |div| div.cursor_pointer())
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::find_action))
            .on_action(cx.listener(Self::find_previous_action))
            .on_action(cx.listener(Self::clear_action))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                let had_menu = this.context_menu.take().is_some();
                let had_link = this.hover_link.take().is_some();
                if had_menu || had_link {
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_drag_move(cx.listener(Self::drag_paths_over))
            .child(
                div().flex_1().min_h_0().child(
                    canvas(
                        move |bounds, _window, _cx| bounds,
                        move |bounds, _, window, cx| {
                            viewport_bounds.set(Some(bounds));
                            if input_focus.is_focused(window) {
                                window.handle_input(
                                    &input_focus,
                                    ElementInputHandler::new(bounds, input_entity.clone()),
                                    cx,
                                );
                            }
                            let drop_entity = input_entity.clone();
                            window.on_mouse_event(
                                move |event: &MouseUpEvent, phase, window, cx| {
                                    if phase != DispatchPhase::Bubble
                                        || !cx.has_active_drag()
                                        || !bounds.contains(&event.position)
                                    {
                                        return;
                                    }
                                    drop_entity.update(cx, |view, cx| {
                                        if let Some(paths) = view.drop_hover.take() {
                                            cx.stop_active_drag(window);
                                            view.drop_paths(&paths, window, cx);
                                        }
                                    });
                                },
                            );
                            use alacritty_terminal::grid::Dimensions;

                            // Measure actual cell dimensions from the font
                            let mut measured_renderer = renderer.clone();
                            measured_renderer.measure_cell(window);
                            viewport_cell
                                .set((measured_renderer.cell_width, measured_renderer.cell_height));

                            // Calculate available space after padding
                            let available_width: f32 =
                                (bounds.size.width - padding.left - padding.right).into();
                            let available_height: f32 =
                                (bounds.size.height - padding.top - padding.bottom).into();
                            let cell_width_f32: f32 = measured_renderer.cell_width.into();
                            let cell_height_f32: f32 = measured_renderer.cell_height.into();

                            let cols = ((available_width / cell_width_f32) as usize).max(1);
                            let rows = ((available_height / cell_height_f32) as usize).max(1);

                            // Helper struct implementing Dimensions for resize
                            struct TermSize {
                                cols: usize,
                                rows: usize,
                            }
                            impl Dimensions for TermSize {
                                fn total_lines(&self) -> usize {
                                    self.rows
                                }
                                fn screen_lines(&self) -> usize {
                                    self.rows
                                }
                                fn columns(&self) -> usize {
                                    self.cols
                                }
                                fn last_column(&self) -> alacritty_terminal::index::Column {
                                    alacritty_terminal::index::Column(self.cols.saturating_sub(1))
                                }
                                fn bottommost_line(&self) -> alacritty_terminal::index::Line {
                                    alacritty_terminal::index::Line(self.rows as i32 - 1)
                                }
                                fn topmost_line(&self) -> alacritty_terminal::index::Line {
                                    alacritty_terminal::index::Line(0)
                                }
                            }

                            // Resize terminal if dimensions changed
                            let mut term = state_arc.lock();
                            let current_cols = term.columns();
                            let current_rows = term.screen_lines();
                            if cols != current_cols || rows != current_rows {
                                // Notify the PTY about the resize
                                if let Some(ref callback) = resize_callback {
                                    callback(cols, rows);
                                }
                                term.resize(TermSize { cols, rows });
                            }

                            // Paint the terminal with measured dimensions
                            measured_renderer.paint(
                                bounds,
                                padding,
                                &term,
                                crate::render::PaintOptions {
                                    search: (!find_matches.is_empty())
                                        .then_some((find_matches.as_slice(), find_current)),
                                    hover: hover_match,
                                    cursor_visible,
                                    scrollbar_color,
                                },
                                window,
                                cx,
                            );

                            // An IME's composing text is drawn over the cursor
                            // cell; it reaches the shell when the input method
                            // commits it.
                            if input_focus.is_focused(window)
                                && let Some(marked) =
                                    marked_text.as_deref().filter(|t| !t.is_empty())
                            {
                                let display_offset = term.grid().display_offset() as i32;
                                let cursor = term.grid().cursor.point;
                                let row = cursor.line.0 - display_offset;
                                if row >= 0 && (row as usize) < rows {
                                    let x = bounds.origin.x
                                        + padding.left
                                        + measured_renderer.cell_width * cursor.column.0 as f32;
                                    let y = bounds.origin.y
                                        + padding.top
                                        + measured_renderer.cell_height * row as f32;
                                    let run = TextRun {
                                        len: marked.len(),
                                        font: Font {
                                            family: marked_font.clone().into(),
                                            ..Default::default()
                                        },
                                        color: marked_color,
                                        background_color: None,
                                        underline: Some(UnderlineStyle {
                                            thickness: px(1.),
                                            color: Some(marked_color),
                                            wavy: false,
                                        }),
                                        strikethrough: None,
                                    };
                                    let line = window.text_system().shape_line(
                                        SharedString::from(marked.to_string()),
                                        marked_size,
                                        &[run],
                                        None,
                                    );
                                    let base_height = measured_renderer.cell_height
                                        / measured_renderer.line_height_multiplier;
                                    let vertical_offset =
                                        (measured_renderer.cell_height - base_height) / 2.0;
                                    let _ = line.paint(
                                        Point {
                                            x,
                                            y: y + vertical_offset,
                                        },
                                        measured_renderer.cell_height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                        },
                    )
                    .size_full(),
                ),
            )
            .children(find_panel)
            .children(tooltip_element)
            .children(menu_element)
    }
}

// Tests are omitted due to macro expansion issues with the test attribute
// in this configuration. Integration tests can be added separately.

#[cfg(test)]
mod tests {
    use super::{link_token_at, parse_osc7_path, percent_decode};

    #[test]
    fn link_tokens_are_urls_and_paths() {
        let text = "see https://example.com/a?b=1.";
        assert_eq!(
            link_token_at(text, 6),
            Some("https://example.com/a?b=1".to_string())
        );
        assert_eq!(
            link_token_at("open Sub/Deep.md now", 7),
            Some("Sub/Deep.md".to_string())
        );
        assert_eq!(
            link_token_at("/Users/me/Note.md", 3),
            Some("/Users/me/Note.md".to_string())
        );
        assert_eq!(link_token_at("and/or", 2), None);
        assert_eq!(link_token_at("plain words", 3), None);
    }

    #[test]
    fn osc7_names_the_shells_directory() {
        assert_eq!(
            parse_osc7_path("file://host/Users/me/My%20Vault"),
            Some(std::path::PathBuf::from("/Users/me/My Vault"))
        );
        assert_eq!(
            parse_osc7_path("file:///tmp/x"),
            Some(std::path::PathBuf::from("/tmp/x"))
        );
        assert_eq!(parse_osc7_path("file://host"), None);
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        // A stray percent is literal, not a malformed escape.
        assert_eq!(percent_decode("100%"), "100%");
    }
}
