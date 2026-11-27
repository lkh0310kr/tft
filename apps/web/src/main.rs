#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{prelude::*, App};

#[cfg(not(feature = "browser-test"))]
mod drive_app;
mod web_workspace;

#[cfg(all(target_arch = "wasm32", not(feature = "browser-test")))]
use crate::drive_app::DriveApp;
#[cfg(feature = "browser-test")]
use crate::web_workspace::{NoteDocument, WebWorkspace};

fn run() {
    gpui_platform::single_threaded_web()
        .with_assets(tft_app::assets::Assets)
        .run(|cx: &mut App| {
            #[cfg(feature = "browser-test")]
            js_sys::Reflect::set(
                &js_sys::global(),
                &wasm_bindgen::JsValue::from_str("__tftEditorBooted"),
                &wasm_bindgen::JsValue::TRUE,
            )
            .expect("publish browser-test boot marker");
            cx.set_global(tft_app::theme::Theme::from_appearance(
                &tft_app::formats::config::AppearanceConfig::default(),
                system_is_dark(),
            ));
            cx.text_system()
                .add_fonts(tft_app::assets::fonts())
                .expect("register TFT fonts");
            tft_app::editor::bind_keys(cx);
            tft_app::text_input::bind_keys(cx);
            web_workspace::bind_keys(cx);
            #[cfg(not(feature = "browser-test"))]
            drive_app::bind_keys(cx);
            #[cfg(target_arch = "wasm32")]
            {
                let (client_id, _, _) = tft_drive_web::config();
                js_sys::Reflect::set(
                    &js_sys::global(),
                    &wasm_bindgen::JsValue::from_str("__tftDriveAdapterLoaded"),
                    &wasm_bindgen::JsValue::TRUE,
                )
                .expect("publish Drive adapter status");
                js_sys::Reflect::set(
                    &js_sys::global(),
                    &wasm_bindgen::JsValue::from_str("__tftDriveConfigured"),
                    &wasm_bindgen::JsValue::from_bool(!client_id.is_empty()),
                )
                .expect("publish Drive configuration status");
            }

            #[cfg(feature = "browser-test")]
            let documents = vec![
                NoteDocument {
                    path: "A.md".into(),
                    contents: "alpha beta gamma\n\n한글 입력 테스트\n- [ ] task\n[[Notes/B]] #tag\n".into(),
                },
                NoteDocument {
                    path: "Notes/B.md".into(),
                    contents: "---\nstatus: draft\n---\n\n# Second note\n\nShared TextBuffer, Markdown renderer, file-tree model, tabs and theme.\n".into(),
                },
            ];
            let bounds = gpui::Bounds::centered(None, gpui::size(gpui::px(1100.), gpui::px(760.)), cx);
            cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |_window, cx| {
                    #[cfg(feature = "browser-test")]
                    {
                        cx.new(|cx| WebWorkspace::from_documents(documents, _window, cx))
                    }
                    #[cfg(not(feature = "browser-test"))]
                    {
                        cx.new(|cx| DriveApp::new(cx))
                    }
                },
            )
            .expect("open shared TFT workspace window");
            cx.activate(true);
        });
}

fn system_is_dark() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        tft_drive_web::system_is_dark()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run();
}
