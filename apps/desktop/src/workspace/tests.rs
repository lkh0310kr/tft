//! Workspace integration and serialization regression tests.

use super::*;
use crate::explorer::kind_name;

/// A tab a link opens goes right after the active one, and with
/// "Always focus new tabs" off it is a tab holding the file that has
/// not been loaded, the active one left where it was (2026-11-15,
/// `settings-gaps.mjs` s02: Alpha, Links (active), Beta became Alpha,
/// Links, Target, Beta).
#[test]
fn a_tab_opened_from_a_link_goes_after_the_active_one() {
    let note = |p: &str| Tab::unloaded(p.to_string(), ViewMode::LivePreview);
    let mut tabs = vec![note("Alpha.md"), note("Links.md"), note("Beta.md")];
    let at = place_new_tab(&mut tabs, 1, note("Target.md"));
    assert_eq!(at, 2);
    let order: Vec<_> = tabs.iter().map(|t| t.path.clone().unwrap()).collect();
    assert_eq!(order, ["Alpha.md", "Links.md", "Target.md", "Beta.md"]);
    // At the end of the strip it is the last tab.
    assert_eq!(place_new_tab(&mut tabs, 3, note("End.md")), 4);
    // The background tab: the file, one entry to go back from.
    let t = Tab::unloaded("Target.md".into(), ViewMode::Reading);
    assert_eq!(t.path.as_deref(), Some("Target.md"));
    assert_eq!(t.history_ix, 1);
    assert_eq!(t.history[0].path, "Target.md");
    assert_eq!(t.mode, ViewMode::Reading);
    assert!(!t.pinned);
}

use crate::title_field::TitleField;
use std::fs;
use std::path::Path;

/// A symlink to a folder or a file on either platform. Windows refuses
/// to make one without Developer Mode or elevation, so a test that
/// needs one skips that part when creation is denied there.
fn symlink(target: &Path, link: &Path) -> Option<()> {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    match made {
        Ok(()) => Some(()),
        Err(e) if cfg!(windows) => {
            eprintln!("symlinks are not permitted here, skipping: {e}");
            None
        }
        Err(e) => panic!("symlink {}: {e}", link.display()),
    }
}

fn vault() -> (tempfile::TempDir, VaultState) {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join("Sub")).unwrap();
    fs::create_dir_all(r.join(".tft")).unwrap();
    fs::write(r.join(".tft/app.json"), r#"{"trashOption":"local"}"#).unwrap();
    fs::write(r.join("A.md"), "# A\n\nSee [[B]] and [S](Sub/S.md).\n").unwrap();
    fs::write(
        r.join("B.md"),
        "\u{feff}---\r\nrelated: \"[[A]]\"\r\n---\r\n[[A]] and [[A#A|the a]] and `[[A]]`\r\n",
    )
    .unwrap();
    fs::write(r.join("Sub/S.md"), "[A](../A.md) [[B]]\n").unwrap();
    let state = VaultState::load(&r.to_path_buf()).unwrap();
    (dir, state)
}

#[test]
fn the_node_graph_tracks_the_path_vault() {
    let (_dir, state) = vault();
    assert!(state.verify_data().is_ok());
    let node = state.data().lookup("Sub/S.md").unwrap();
    assert_eq!(state.data().path(node), Some("Sub/S.md"));
    state.create_note("New/N.md", "x").unwrap();
    assert!(state.verify_data().is_ok());
    let a = state.data().lookup("A.md").unwrap();
    state.rename("A.md", "C.md", false).unwrap();
    assert_eq!(state.data().lookup("C.md"), Ok(a), "a rename keeps NodeId");
    assert!(state.verify_data().is_ok());
    state.trash("C.md").unwrap();
    assert!(state.verify_data().is_ok());
}

/// `workspace:copy-full-path` copies the file from the file system's
/// root, its folders joined with the platform's separator, while
/// `workspace:copy-path` copies the vault-relative `Sub/S.md` as is.
#[test]
fn copy_full_path_is_the_vault_root_joined_with_the_vault_path() {
    let (dir, state) = vault();
    let text = full_path_text(state.vault().absolute("Sub/S.md"));
    let expected = dir.path().join("Sub").join("S.md");
    assert_eq!(text, expected.to_string_lossy());
    assert!(Path::new(&text).is_absolute());
    assert!(Path::new(&text).is_file());
}

/// A vault opened by a relative path still copies an absolute one, made
/// against the working folder, as the command's name promises.
#[test]
fn copy_full_path_of_a_relatively_opened_vault_is_absolute() {
    let text = full_path_text(PathBuf::from("vault").join("Note.md"));
    let expected = std::env::current_dir()
        .unwrap()
        .join("vault")
        .join("Note.md");
    assert_eq!(text, expected.to_string_lossy());
}

/// The window cluster's commands the 2026-11-09 probe settled are
/// runnable and are Obsidian's ids; the ones gpui blocks, the tab
/// switcher Obsidian does not register on desktop, and the two whose
/// screens report on Obsidian itself (its version, its changelog) are
/// not.
#[test]
fn the_observed_app_and_window_commands_are_runnable() {
    let known = tft_commands::known_ids();
    for id in [
        "workspace:copy-path",
        "workspace:copy-full-path",
        "workspace:show-trash",
        "open-with-default-app:open",
        "open-with-default-app:show",
        "theme:toggle-light-dark",
    ] {
        assert!(RUNNABLE.contains(&id), "{id} runnable");
        assert!(known.contains(&id), "{id} is a known command ID");
    }
    for id in [
        "window:zoom-in",
        "window:zoom-out",
        "window:reset-zoom",
    ] {
        assert!(RUNNABLE.contains(&id), "{id} runnable");
        assert!(known.contains(&id), "{id} is a known command ID");
    }
    // Not registered on a desktop (2026-11-15 probe).
    assert!(!RUNNABLE.contains(&"app:show-tab-switcher"));
    for id in ACTIVE_FILE_ONLY {
        assert!(RUNNABLE.contains(id), "{id} runnable");
    }
}

#[test]
fn show_trash_names_the_missing_folder_in_curly_quotes() {
    assert_eq!(
        missing_file_notice("/tmp/claude-1000/appwin/main/Probe Vault/.trash"),
        "The file or folder \u{201c}/tmp/claude-1000/appwin/main/Probe Vault/.trash\u{201d} does not exist."
    );
    assert_eq!(TRASH_FOLDER, ".trash");
}

/// Focus on last note is offered while any tab is a note: an empty tab
/// and a canvas alone do not count, nor do an image, a PDF, a base or a
/// tab showing the graph.
#[test]
fn focus_on_last_note_is_offered_only_with_a_markdown_leaf() {
    assert!(!any_markdown_leaf(
        [(None, false), (Some("Board.canvas"), false)].into_iter()
    ));
    assert!(!any_markdown_leaf(
        [
            (Some("img/pic.png"), false),
            (Some("Paper.pdf"), false),
            (Some("Note.md"), true),
        ]
        .into_iter()
    ));
    assert!(any_markdown_leaf(
        [(Some("Board.canvas"), false), (Some("Note A.md"), false)].into_iter()
    ));
    assert!(!any_markdown_leaf(std::iter::empty()));
}

fn history(entries: &[&str]) -> Vec<HistoryEntry> {
    entries
        .iter()
        .map(|e| HistoryEntry::new(e.to_string(), ViewMode::LivePreview))
        .collect()
}

/// `can_run` checks `RUNNABLE` first, so an id only in `EDITOR_ONLY`
/// would be filtered twice over and never offered at all.
#[test]
fn every_editor_only_id_is_runnable() {
    for id in EDITOR_ONLY {
        assert!(
            RUNNABLE.contains(id),
            "{id} is editor-only but not runnable"
        );
    }
    // The table commands have their own rule in `can_run` and must not
    // be in the editor-only list as well, or the narrower check never
    // runs.
    for id in EDITOR_ONLY {
        assert!(
            TableCommand::from_id(id).is_none(),
            "{id} is both a table command and editor-only"
        );
    }
}

/// Neither list may name an id the registry does not have: a typo here
/// is a command that silently never appears.
#[test]
fn both_lists_name_real_commands() {
    let registry = CommandRegistry::with_known_ids();
    for id in RUNNABLE.iter().chain(EDITOR_ONLY) {
        assert!(registry.get(id).is_some(), "{id} is not a known command");
    }
}

/// The converse of [`every_editor_only_id_is_runnable`], and the one
/// that was missing: an id `run_command` answers but `RUNNABLE` does not
/// name is a command that works and nothing can reach. `can_run` tests
/// `RUNNABLE` first, so the palette leaves the id out, the socket
/// reports it as not runnable, `bind_keys` never binds its hotkey and
/// `run_command` does not even count it as recent. "Open help" sat that
/// way with F1 in `commands` and no key bound to it.
///
/// The ids are read off the match itself rather than listed again here,
/// because a list kept by hand is what failed the first time.
#[test]
fn every_handled_id_is_runnable() {
    const SOURCE: &str = include_str!("workspace/commands.rs");
    let body = SOURCE
        .split_once("pub fn run_command(")
        .expect("run_command is declared in this file")
        .1;
    let mut handled: Vec<String> = Vec::new();
    // An arm's pattern is the lines at the match's own indentation that
    // start with an id or carry on an `|` chain, up to the `=>`: a long
    // chain is wrapped over several lines, and a guard (`if ...`) sits
    // between the last id and the arrow. An arm's body is indented
    // past this, so a nested match and a string literal in a call are
    // both out of reach.
    let mut pattern = String::new();
    for line in body.lines() {
        if line.starts_with("            _ =>") {
            break;
        }
        if !(line.starts_with("            \"") || line.starts_with("            | \"")) {
            continue;
        }
        pattern.push_str(line);
        let Some((ids, _)) = pattern.split_once("=>") else {
            continue;
        };
        // Every id is quoted, so the odd pieces of a split on the quote
        // are the ids and the even ones are the `|`s and the guard.
        handled.extend(ids.split('"').skip(1).step_by(2).map(|id| id.to_string()));
        pattern.clear();
    }
    // A floor, so that a change to the match's shape shows up as a
    // failure here rather than as a test that quietly checks nothing.
    assert!(
        handled.len() >= 95,
        "only {} arms found; the reader above has stopped matching \
             run_command's match",
        handled.len()
    );
    for id in &handled {
        assert!(
            RUNNABLE.contains(&id.as_str()),
            "{id} is handled by run_command but not in RUNNABLE, so \
                 nothing can reach it"
        );
    }
}

/// A deleted note leaves the history, so back and forward cannot land
/// on a file the vault no longer has. Entries before the cursor take it
/// back with them; the notes it can still reach do not change.
fn leaf(id: &str, file: &str, mode: &str, source: bool) -> Value {
    // A pinned leaf, in the shape Obsidian writes one (the
    // tab-header probe's `pinnedWorkspace`).
    json!({
        "id": id,
        "type": "leaf",
        "pinned": true,
        "state": {
            "type": "markdown",
            "state": {"file": file, "mode": mode, "source": source},
            "pinned": true,
            "icon": "lucide-file",
            "title": file.rsplit('/').next().unwrap().trim_end_matches(".md"),
        },
    })
}

#[test]
fn a_restored_leaf_is_written_back_as_it_was_read() {
    let read = leaf("a1072f377b3d1c69", "Sub/Note.md", "source", false);
    let tab = Tab {
        node: None,
        path: Some("Sub/Note.md".into()),
        mode: ViewMode::LivePreview,
        history: vec![HistoryEntry::new(
            "Sub/Note.md".into(),
            ViewMode::LivePreview,
        )],
        history_ix: 1,
        leaf: Some(read.clone()),
        view_state: None,
        pinned: true,
    };
    assert_eq!(leaf_json(&tab), read);
}

#[test]
fn a_mode_change_rewrites_only_the_mode_keys() {
    let read = leaf("a1072f377b3d1c69", "Note.md", "source", false);
    let tab = Tab {
        node: None,
        path: Some("Note.md".into()),
        mode: ViewMode::Reading,
        history: Vec::new(),
        history_ix: 0,
        leaf: Some(read.clone()),
        view_state: None,
        pinned: true,
    };
    let written = leaf_json(&tab);
    assert_eq!(written["id"], read["id"]);
    assert_eq!(written["state"]["pinned"], json!(true));
    assert_eq!(written["state"]["state"]["mode"], json!("preview"));
    assert_eq!(written["state"]["state"]["source"], json!(false));
    let tab = Tab {
        mode: ViewMode::Source,
        ..tab
    };
    let written = leaf_json(&tab);
    assert_eq!(written["state"]["state"]["mode"], json!("source"));
    assert_eq!(written["state"]["state"]["source"], json!(true));
}

/// A canvas tab is written as Obsidian's canvas leaf: its view type and
/// icon, the title without the extension, and the `viewState` the view
/// reported; a leaf restored with one and never moved keeps it.
#[test]
fn a_canvas_tab_is_written_as_a_canvas_leaf() {
    let tab = Tab {
        node: None,
        path: Some("Plans/Board.canvas".into()),
        mode: ViewMode::LivePreview,
        history: Vec::new(),
        history_ix: 0,
        leaf: None,
        view_state: Some(json!({"x": 100, "y": -20, "zoom": -1})),
        pinned: false,
    };
    let written = leaf_json(&tab);
    assert_eq!(written["state"]["type"], json!("canvas"));
    assert_eq!(written["state"]["icon"], json!("lucide-layout-dashboard"));
    assert_eq!(written["state"]["title"], json!("Board"));
    assert_eq!(
        written["state"]["state"],
        json!({"file": "Plans/Board.canvas", "viewState": {"x": 100, "y": -20, "zoom": -1}})
    );
    assert!(written["state"]["state"].get("mode").is_none());
    let known = |_: &str| true;
    let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
    assert_eq!(restored.view_state, tab.view_state);
    assert_eq!(leaf_json(&restored), written);
}

/// Audio and video tabs are Obsidian's `audio` and `video` leaves:
/// observed, `{"type":"audio","state":{"file":"tone.mp3"},"icon":
/// "lucide-file-audio","title":"tone"}` and the video's with the plain
/// file icon (`2026-11-15-embeds-media.json`, `s09`).
#[test]
fn audio_and_video_tabs_are_written_as_reference_writes_them() {
    let tab = |path: &str| Tab {
        node: None,
        path: Some(path.into()),
        mode: ViewMode::Reading,
        history: Vec::new(),
        history_ix: 0,
        leaf: None,
        view_state: None,
        pinned: false,
    };
    let audio = leaf_json(&tab("tone.mp3"));
    assert_eq!(audio["state"]["type"], json!("audio"));
    assert_eq!(audio["state"]["icon"], json!("lucide-file-audio"));
    assert_eq!(audio["state"]["title"], json!("tone"));
    assert_eq!(audio["state"]["state"], json!({"file": "tone.mp3"}));
    let video = leaf_json(&tab("Clips/clip.MP4"));
    assert_eq!(video["state"]["type"], json!("video"));
    assert_eq!(video["state"]["icon"], json!("lucide-file"));
    assert_eq!(video["state"]["title"], json!("clip"));
    assert!(shown_as_embed(FileKind::Audio) && shown_as_embed(FileKind::Video));
    assert!(!shown_as_embed(FileKind::Pdf));
}

/// A PDF tab is written as Obsidian's `pdf` leaf, which is the file
/// alone: observed, `state.state` is `{"file": ..}` before and after
/// scrolling, zooming and a quit, the icon is `lucide-file-text` and
/// the title drops the extension. Where the view is in the file never
/// reaches the leaf, and a leaf read with extra keys is written back as
/// read.
#[test]
fn a_pdf_tab_is_written_as_a_file_only_pdf_leaf() {
    let tab = Tab {
        node: None,
        path: Some("Papers/Field.pdf".into()),
        mode: ViewMode::LivePreview,
        history: Vec::new(),
        history_ix: 0,
        leaf: None,
        view_state: None,
        pinned: false,
    };
    let written = leaf_json(&tab);
    assert_eq!(written["state"]["type"], json!("pdf"));
    assert_eq!(written["state"]["icon"], json!("lucide-file-text"));
    assert_eq!(written["state"]["title"], json!("Field"));
    assert_eq!(
        written["state"]["state"],
        json!({"file": "Papers/Field.pdf"})
    );
    assert!(written["state"]["state"].get("mode").is_none());
    assert_eq!(tab.title(), "Field", "the strip shows no extension either");
    assert_eq!(tab_display_name("Field.PDF"), "Field");
    let known = |_: &str| true;
    let (_, restored) = tab_from_leaf(&written, ViewMode::Reading, &known).unwrap();
    assert_eq!(restored.view_state, None);
    assert_eq!(leaf_json(&restored), written);
    // A leaf somebody else wrote with keys of its own keeps them.
    let read = json!({
        "id": "0f6c3a9d2b1e4c5a",
        "type": "leaf",
        "state": {
            "type": "pdf",
            "state": {"file": "Papers/Field.pdf", "mystery": true},
            "icon": "lucide-file-text",
            "title": "Field"
        }
    });
    let (_, restored) = tab_from_leaf(&read, ViewMode::Reading, &known).unwrap();
    assert_eq!(leaf_json(&restored), read, "never moved: written as read");
}

/// Pinning writes `pinned` twice, in Obsidian's places, byte for byte
/// against the probe's file; unpinning takes both keys out.
#[test]
fn a_pinned_leaf_is_written_in_references_shape() {
    let probed = r#"{
  "id": "726a32fa984bda69",
  "type": "leaf",
  "pinned": true,
  "state": {
    "type": "markdown",
    "state": {
      "file": "Alpha.md",
      "mode": "source",
      "source": false
    },
    "pinned": true,
    "icon": "lucide-file",
    "title": "Alpha"
  }
}"#;
    let read: Value = serde_json::from_str(probed).unwrap();
    let known = |_: &str| true;
    let (_, mut tab) = tab_from_leaf(&read, ViewMode::LivePreview, &known).unwrap();
    assert!(tab.pinned);
    tab.pinned = false;
    let unpinned = leaf_json(&tab);
    assert!(unpinned.get("pinned").is_none());
    assert!(unpinned["state"].get("pinned").is_none());
    tab.leaf = Some(unpinned);
    tab.pinned = true;
    let pinned = serde_json::to_string_pretty(&leaf_json(&tab)).unwrap();
    assert_eq!(pinned, probed);
}

#[test]
fn a_tab_that_moved_to_another_note_gets_a_fresh_leaf() {
    let read = leaf("a1072f377b3d1c69", "Old.md", "source", false);
    let tab = Tab {
        node: None,
        path: Some("New.md".into()),
        mode: ViewMode::LivePreview,
        history: Vec::new(),
        history_ix: 0,
        leaf: Some(read),
        view_state: None,
        pinned: false,
    };
    let written = leaf_json(&tab);
    let id = written["id"].as_str().unwrap();
    assert_eq!(id.len(), 16);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(id, "a1072f377b3d1c69");
    assert_eq!(written["state"]["type"], json!("markdown"));
    assert_eq!(written["state"]["state"]["file"], json!("New.md"));
    assert_eq!(written["state"]["title"], json!("New"));
    assert!(written["state"].get("pinned").is_none());
}

#[test]
fn an_empty_tab_and_a_graph_tab_write_their_own_leaf_types() {
    let empty = leaf_json(&Tab::empty(ViewMode::LivePreview));
    assert_eq!(empty["state"]["type"], json!("empty"));
    assert_eq!(empty["state"]["title"], json!("New tab"));
    let graph = leaf_json(&Tab::empty(ViewMode::Graph));
    assert_eq!(graph["state"]["type"], json!("graph"));
    assert_eq!(graph["state"]["icon"], json!("lucide-git-fork"));
}

/// An empty leaf is called "New tab" wherever its name is shown: the
/// tab strip, the view header and the window title all ask for the one
/// string, so none of them can read "No file is open" (see
/// [[Specification#The empty tab]]).
#[test]
fn an_empty_leaf_is_called_new_tab_wherever_its_name_is_shown() {
    for mode in [
        ViewMode::Reading,
        ViewMode::LivePreview,
        ViewMode::Source,
        ViewMode::Reading,
    ] {
        assert_eq!(leaf_title(None, mode), "New tab");
        assert_eq!(Tab::empty(mode).title(), "New tab");
    }
    assert_eq!(leaf_title(None, ViewMode::Graph), "Graph view");
    assert_eq!(
        leaf_title(Some("Folder/Note.md"), ViewMode::LivePreview),
        "Note"
    );
}

/// The empty tab's three pills, in Obsidian's order and with the text it
/// draws on them: the hotkeys come from the registry, so `New note` and
/// `Go to file` carry the chords their commands are bound to and the
/// other two carry none.
#[test]
fn the_empty_tab_offers_references_four_pills() {
    let registry = CommandRegistry::with_known_ids();
    let actions = empty_tab::empty_tab_actions(&registry);
    let mod_key = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    let texts: Vec<String> = actions.iter().map(empty_tab::EmptyAction::text).collect();
    assert_eq!(
        texts,
        [
            format!("New note {mod_key} + N"),
            "New canvas".to_string(),
            format!("Go to file {mod_key} + O"),
        ]
    );
    let commands: Vec<&str> = actions.iter().map(|a| a.command).collect();
    assert_eq!(
        commands,
        ["file-explorer:new-file", "canvas:new-file", "switcher:open"]
    );
    // Each pill runs its command, so each has to be one the workspace
    // will actually run.
    for action in &actions {
        assert!(
            RUNNABLE.contains(&action.command),
            "{} is not runnable",
            action.command
        );
    }
}

/// A pill whose command the registry does not have is not drawn: the
/// registry holds the commands of the core plugins that are on, so
/// turning Canvas off takes "New canvas" with it rather than leaving a
/// pill that does nothing.
#[test]
fn a_pill_for_a_disabled_plugin_is_left_out() {
    let mut registry = CommandRegistry::with_known_ids();
    registry.retain_enabled(|plugin| plugin != "canvas");
    let labels: Vec<&str> = empty_tab::empty_tab_actions(&registry)
        .iter()
        .map(|a| a.label)
        .collect();
    assert_eq!(labels, ["New note", "Go to file"]);
}

/// The status bar is not hidden on an empty tab: with one active
/// Obsidian's reads `0 properties`. A canvas and a base show nothing,
/// and a note shows its caret.
#[test]
fn an_empty_tabs_status_bar_reads_zero_rather_than_nothing() {
    assert_eq!(Workspace::status_items(None, ""), ["0 properties"]);
    assert!(Workspace::status_items(Some("Board.canvas"), "").is_empty());
    assert_eq!(
        Workspace::status_items(Some("Note.md"), "Ln 1, Col 1  ·  "),
        ["Ln 1, Col 1  ·  "]
    );
}

/// The empty tab's header carries the two navigation buttons and no
/// mode switch: there is nothing to read or edit. The graph keeps its
/// own toggle instead, as it already did.
#[test]
fn the_empty_tabs_header_has_no_mode_switch() {
    for mode in [
        ViewMode::Reading,
        ViewMode::LivePreview,
        ViewMode::Source,
        ViewMode::Graph,
    ] {
        assert!(
            !view_header::header_shows_mode_switch(&None, mode),
            "an empty tab in {mode:?} must not offer the mode switch"
        );
    }
    assert!(view_header::header_shows_mode_switch(
        &Some("Note.md".to_string()),
        ViewMode::Reading
    ));
    assert!(!view_header::header_shows_mode_switch(
        &Some("Note.md".to_string()),
        ViewMode::Graph
    ));
}

/// Reaching the empty tab by closing every tab gives the same view as
/// a fresh vault's: [`Workspace::close_tab`] pushes a [`Tab::empty`] in
/// the default mode, and every one of those is the same leaf -- no
/// file, the "New tab" name, the empty main view and the zeroed status
/// bar -- however the window got there.
#[test]
fn closing_every_tab_leaves_the_tab_a_fresh_vault_opens_with() {
    let fresh = Tab::empty(ViewMode::Reading);
    for mode in [ViewMode::LivePreview, ViewMode::Source, ViewMode::Reading] {
        let closed = Tab::empty(mode);
        assert_eq!(closed.path, fresh.path);
        assert_eq!(closed.title(), fresh.title());
        assert_eq!(
            Workspace::main_view_for(closed.mode, false, false, false, false, false),
            MainView::Empty
        );
        assert_eq!(
            Workspace::status_items(closed.path.as_deref(), ""),
            Workspace::status_items(fresh.path.as_deref(), "")
        );
        assert!(!view_header::header_shows_mode_switch(
            &closed.path,
            closed.mode
        ));
    }
}

/// Enter in the view-header title renames the file the field opened on,
/// through the path the rename prompt's box commits through: the field
/// holds the stem, so the extension goes back on and the name is
/// checked before the filesystem is asked for it. The gesture the probe
/// recorded: `Props` clicked at offset 3 and typed `X` reads `ProXps`
/// while the file is still `Props.md`, and Enter asks for `ProXps.md`.
#[test]
fn the_header_titles_commit_asks_for_the_path_the_rename_prompt_would() {
    let mut field = TitleField::new("Props");
    field.click(3, 1, false);
    field.insert("X");
    assert_eq!(
        field.text(),
        "ProXps",
        "the title changed, the file has not"
    );
    assert_eq!(
        file_operations::renamed_path("Props.md", field.text().trim(), false).as_deref(),
        Ok("ProXps.md")
    );
    assert_eq!(
        file_operations::renamed_path("Folder/Props.md", "New name", false).as_deref(),
        Ok("Folder/New name.md")
    );
    assert_eq!(
        file_operations::renamed_path("Folder/Board.canvas", "Board 2", false).as_deref(),
        Ok("Folder/Board 2.canvas")
    );
    // A folder has no extension to put back.
    assert_eq!(
        file_operations::renamed_path("Folder", "Renamed", true).as_deref(),
        Ok("Renamed")
    );
    // And a name the filesystem would refuse never gets there: the
    // field stays open on the error instead.
    assert!(file_operations::renamed_path("Props.md", "", false).is_err());
    assert!(file_operations::renamed_path("Props.md", "a/b", false).is_err());
    assert!(file_operations::renamed_path("Props.md", &"x".repeat(300), false).is_err());
}

/// Obsidian's `main` after "Split right" on A, then B opened in the new
/// group and "Split down" there, as observed on 2026-10-20: a vertical
/// root over a `tabs` group and a nested horizontal split.
fn nested_main() -> Value {
    json!({
        "id": "449f821d63addc06",
        "type": "split",
        "children": [
            {"id": "3ab610226e243404", "type": "tabs", "dimension": 29.411764705882355,
             "children": [leaf("3c245a8fc7dc4598", "A.md", "source", false)]},
            {"id": "0fa61223ab358f4f", "type": "split", "dimension": 70.58823529411765,
             "children": [
                {"id": "6c9c4444fe9200bb", "type": "tabs",
                 "children": [leaf("f0559f5cf8f3ac65", "B.md", "source", false)]},
                {"id": "189916eb401608c6", "type": "tabs", "currentTab": 1,
                 "children": [
                    leaf("747a588e39ce11fa", "B.md", "source", false),
                    leaf("52ae6ba5c0105abd", "C.md", "preview", false),
                 ]},
             ],
             "direction": "horizontal"},
        ],
        "direction": "vertical",
    })
}

#[test]
fn main_is_read_as_a_tree_of_groups_with_their_own_keys() {
    let spec = read_main(&nested_main(), ViewMode::LivePreview, &|_| true).unwrap();
    let LayoutSpec::Split {
        direction,
        children,
        node,
    } = spec
    else {
        panic!("the root is a split");
    };
    assert_eq!(direction, SplitDirection::Vertical);
    assert_eq!(node.get("id"), Some(&json!("449f821d63addc06")));
    assert!(!node.contains_key("children") && !node.contains_key("direction"));
    assert_eq!(children.len(), 2);
    let LayoutSpec::Group(left) = &children[0] else {
        panic!("the first child is a group");
    };
    assert_eq!(left.tabs.len(), 1);
    assert_eq!(left.tabs[0].0, "3c245a8fc7dc4598");
    assert_eq!(left.tabs[0].1.mode, ViewMode::LivePreview);
    assert_eq!(left.node.get("dimension"), Some(&json!(29.411764705882355)));
    let LayoutSpec::Split {
        direction,
        children,
        ..
    } = &children[1]
    else {
        panic!("the second child is a split");
    };
    assert_eq!(*direction, SplitDirection::Horizontal);
    let LayoutSpec::Group(bottom) = &children[1] else {
        panic!("a group");
    };
    assert_eq!(bottom.active_tab, 1);
    assert_eq!(bottom.tabs[1].1.mode, ViewMode::Reading);
}

#[test]
fn a_group_whose_files_are_all_gone_is_dropped_with_its_empty_split() {
    let spec = read_main(&nested_main(), ViewMode::LivePreview, &|f| f == "A.md").unwrap();
    let LayoutSpec::Split { children, .. } = spec else {
        panic!("the root is a split");
    };
    assert_eq!(children.len(), 1, "the horizontal split had nothing left");
    assert!(read_main(&nested_main(), ViewMode::LivePreview, &|_| false).is_none());
}

/// Obsidian 1.15.0's `workspace.json` with four popouts open, as the
/// probe recorded it (`tools/reference-probe/results/2026-11-09-popout-windows.json`,
/// `s04-new-window`): the vault window holding one empty tab, a window
/// each on Beta, Alpha and Gamma, and a fourth on an empty tab, which
/// is the active leaf.
const FOUR_POPOUTS: &str = include_str!("../tests/fixtures/workspace-four-popouts.json");

/// A `LayoutSpec` written back the way the workspace writes its groups,
/// so the reading and the writing can be checked without a window.
fn spec_json(spec: &LayoutSpec) -> Value {
    match spec {
        LayoutSpec::Group(group) => tabs_node_json(
            group.node.clone(),
            group.tabs.iter().map(|(_, t)| leaf_json(t)).collect(),
            group.active_tab,
        ),
        LayoutSpec::Split {
            direction,
            children,
            node,
        } => split_node_json(
            node.clone(),
            children.iter().map(spec_json).collect(),
            *direction,
        ),
    }
}

fn window_spec_json(w: &WindowSpec) -> Value {
    let LayoutSpec::Split {
        direction,
        children,
        ..
    } = &w.tree
    else {
        panic!("a window's root is a split");
    };
    window_node_json(
        &w.node,
        children.iter().map(spec_json).collect(),
        *direction,
    )
}

#[test]
fn floating_is_read_as_windows_over_groups_with_their_bounds() {
    let layout: Value = serde_json::from_str(FOUR_POPOUTS).unwrap();
    let windows = read_floating(&layout["floating"], ViewMode::LivePreview, &|_| true);
    assert_eq!(windows.len(), 4);
    let first = &windows[0];
    assert_eq!(first.node.get("id"), Some(&json!("fbe32d82ccdfaec7")));
    assert_eq!(first.node.get("type"), Some(&json!("window")));
    assert!(!first.node.contains_key("children") && !first.node.contains_key("direction"));
    assert_eq!(
        first.node.keys().collect::<Vec<_>>(),
        ["id", "type", "x", "y", "width", "height", "maximize", "zoom"]
    );
    let LayoutSpec::Split {
        direction,
        children,
        node,
    } = &first.tree
    else {
        panic!("a window's root is a split");
    };
    assert_eq!(*direction, SplitDirection::Vertical);
    assert!(node.is_empty(), "the window's keys live on the window");
    let LayoutSpec::Group(group) = &children[0] else {
        panic!("one group");
    };
    assert_eq!(group.tabs[0].1.path.as_deref(), Some("Beta.md"));
    assert_eq!(group.tabs[0].1.mode, ViewMode::LivePreview);
    // The recorded bounds come through the node the workspace keeps.
    let node = WindowNode::from_node(
        1,
        desktop_split(SplitDirection::Vertical, vec![Layout::Group(1)]),
        first.node.clone(),
    );
    assert_eq!(
        node.bounds(),
        Some(WindowBounds {
            x: 16,
            y: 10,
            width: 980,
            height: 759
        })
    );
    // The fourth window holds an empty tab, which is a tab here too.
    let LayoutSpec::Split { children, .. } = &windows[3].tree else {
        panic!("a split");
    };
    let LayoutSpec::Group(group) = &children[0] else {
        panic!("one group");
    };
    assert_eq!(group.tabs[0].1.path, None);
    // No `floating` key, no windows; and a `floating` with none is none.
    assert!(read_floating(
        &json!({"id": "x", "type": "floating", "children": []}),
        ViewMode::LivePreview,
        &|_| true
    )
    .is_empty());
}

#[test]
fn a_window_whose_files_are_all_gone_is_dropped() {
    let layout: Value = serde_json::from_str(FOUR_POPOUTS).unwrap();
    let windows = read_floating(&layout["floating"], ViewMode::LivePreview, &|f| {
        f != "Gamma.md"
    });
    assert_eq!(
        windows.len(),
        3,
        "Gamma's window went; the empty tab's stays"
    );
    assert_eq!(windows[2].node.get("id"), Some(&json!("b3bc438a3f884b7d")));
}

/// The Writer gate's rule for the whole file, applied to `floating`:
/// read Obsidian's bytes into the model and write them back, and
/// nothing moves. `main` goes through the same writer, so the test
/// covers the file as the workspace writes it, `active` and
/// `lastOpenFiles` untouched here because the pure writers do not set
/// them.
#[test]
fn references_popout_layout_round_trips_byte_for_byte() {
    let layout: Value = serde_json::from_str(FOUR_POPOUTS).unwrap();
    let main = read_main(&layout["main"], ViewMode::LivePreview, &|_| true).unwrap();
    let windows = read_floating(&layout["floating"], ViewMode::LivePreview, &|_| true);
    let mut written = layout.clone();
    written["main"] = spec_json(&main);
    let existing = layout["floating"].as_object().cloned();
    written["floating"] = floating_json(
        existing.as_ref(),
        windows.iter().map(window_spec_json).collect(),
    );
    assert_eq!(
        serde_json::to_string_pretty(&written).unwrap(),
        FOUR_POPOUTS
    );
}

/// A window TFT opens is written with Obsidian's keys in Obsidian's
/// order, the bounds as whole numbers, and a `floating` the file never
/// had gets a fresh node.
#[test]
fn a_fresh_window_node_writes_references_keys_in_references_order() {
    let tree = desktop_split(SplitDirection::Vertical, vec![Layout::Group(3)]);
    let w = WindowNode::new(
        1,
        tree.clone(),
        WindowBounds {
            x: 16,
            y: 10,
            width: 1024,
            height: 800,
        },
    );
    let Layout::Split { direction, .. } = &tree else {
        panic!("a split");
    };
    let group = json!({"id": "g", "type": "tabs", "children": []});
    let written = window_node_json(&w.node, vec![group], *direction);
    assert_eq!(
        written.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "id",
            "type",
            "children",
            "direction",
            "x",
            "y",
            "width",
            "height",
            "maximize",
            "zoom"
        ]
    );
    assert_eq!(written["type"], json!("window"));
    assert_eq!(written["direction"], json!("vertical"));
    assert_eq!(written["x"], json!(16), "a whole number, not 16.0");
    assert_eq!(written["height"], json!(800));
    assert_eq!(written["maximize"], json!(false));
    assert_eq!(written["zoom"], json!(0));
    assert_eq!(written["id"].as_str().unwrap().len(), 16);
    let floating = floating_json(None, vec![written]);
    assert_eq!(
        floating.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["id", "type", "children"]
    );
    assert_eq!(floating["type"], json!("floating"));
    assert_eq!(floating["children"].as_array().unwrap().len(), 1);
}

#[test]
fn the_top_right_group_follows_the_splits_shape() {
    // Side by side: the last child. Stacked: the first.
    let row = desktop_split(
        SplitDirection::Vertical,
        vec![Layout::Group(1), Layout::Group(2)],
    );
    assert_eq!(row.top_right(), Some(2));
    let stack = desktop_split(
        SplitDirection::Horizontal,
        vec![Layout::Group(1), Layout::Group(2)],
    );
    assert_eq!(stack.top_right(), Some(1));
    // A stack on the right of a row: its top group holds the corner,
    // though the depth-first order ends on its bottom one.
    let nested = desktop_split(
        SplitDirection::Vertical,
        vec![
            Layout::Group(1),
            desktop_split(
                SplitDirection::Horizontal,
                vec![Layout::Group(2), Layout::Group(3)],
            ),
        ],
    );
    assert_eq!(nested.top_right(), Some(2));
    let mut order = Vec::new();
    nested.groups(&mut order);
    assert_eq!(order.last(), Some(&3));
}

#[test]
fn splitting_puts_a_sibling_in_a_split_that_runs_that_way_and_wraps_otherwise() {
    let mut tree = desktop_split(SplitDirection::Vertical, vec![Layout::Group(1)]);
    assert_eq!(
        tree.insert_beside(1, 2, SplitDirection::Vertical, true),
        Some(false)
    );
    let mut order = Vec::new();
    tree.groups(&mut order);
    assert_eq!(order, [1, 2]);
    assert_eq!(
        tree.insert_beside(2, 3, SplitDirection::Horizontal, true),
        Some(true)
    );
    assert_eq!(tree.path_to(3), Some(vec![1, 1]));
    assert!(matches!(
        tree.node_at(&[1]),
        Some(Layout::Split { direction: SplitDirection::Horizontal, children, .. }) if children.len() == 2
    ));
    // Neighbours: from 1, right is the whole right column; from 3, up is 2
    // and left is 1; from 2, down is 3, up is nothing.
    assert_eq!(tree.neighbors(1, SplitDirection::Vertical, true), [2, 3]);
    assert_eq!(tree.neighbors(3, SplitDirection::Horizontal, false), [2]);
    assert_eq!(tree.neighbors(3, SplitDirection::Vertical, false), [1]);
    assert_eq!(tree.neighbors(2, SplitDirection::Horizontal, true), [3]);
    assert!(tree
        .neighbors(2, SplitDirection::Horizontal, false)
        .is_empty());
    assert!(tree
        .neighbors(1, SplitDirection::Vertical, false)
        .is_empty());
    // Removing 2 leaves the horizontal split nested around 3, as Obsidian
    // keeps it; removing 3 too takes the split with it.
    assert!(tree.remove(2));
    assert_eq!(tree.path_to(3), Some(vec![1, 0]));
    assert!(tree.remove(3));
    let mut order = Vec::new();
    tree.groups(&mut order);
    assert_eq!(order, [1]);
    assert!(matches!(&tree, Layout::Split { children, .. } if children.len() == 1));
}

/// A tab dropped on a group's left or top edge goes before it: a sibling
/// in front when the split runs that way, the first child of the split
/// that wraps the pair when it does not.
#[test]
fn dropping_on_a_near_edge_puts_the_new_group_in_front() {
    let mut tree = desktop_split(SplitDirection::Vertical, vec![Layout::Group(1)]);
    assert_eq!(
        tree.insert_beside(1, 2, SplitDirection::Vertical, false),
        Some(false)
    );
    let mut order = Vec::new();
    tree.groups(&mut order);
    assert_eq!(order, [2, 1]);
    assert_eq!(
        tree.insert_beside(1, 3, SplitDirection::Horizontal, false),
        Some(true)
    );
    assert_eq!(tree.path_to(3), Some(vec![1, 0]));
    assert_eq!(tree.path_to(1), Some(vec![1, 1]));
    assert_eq!(tree.neighbors(1, SplitDirection::Horizontal, false), [3]);
}

/// The zones over a group's content: the outer band of each side splits
/// towards that side, the corner goes to the nearer edge, and the middle
/// joins the strip. The band is measured on the shorter side.
#[test]
fn drop_zones_follow_the_nearest_edge_within_the_band() {
    let bounds = gpui::Bounds {
        origin: point(px(100.), px(50.)),
        size: gpui::size(px(400.), px(200.)),
    };
    let at = |x: f32, y: f32| DropZone::at(bounds, point(px(100. + x), px(50. + y)));
    // The band is 30 percent of 200: 60 px.
    assert_eq!(at(200., 100.), DropZone::Center);
    assert_eq!(at(10., 100.), DropZone::Left);
    assert_eq!(at(390., 100.), DropZone::Right);
    assert_eq!(at(200., 10.), DropZone::Top);
    assert_eq!(at(200., 190.), DropZone::Bottom);
    assert_eq!(at(70., 100.), DropZone::Center);
    assert_eq!(at(200., 65.), DropZone::Center);
    // In a corner the nearer edge wins.
    assert_eq!(at(20., 40.), DropZone::Left);
    assert_eq!(at(40., 20.), DropZone::Top);
    assert_eq!(
        DropZone::Left.split(),
        Some((SplitDirection::Vertical, false))
    );
    assert_eq!(
        DropZone::Bottom.split(),
        Some((SplitDirection::Horizontal, true))
    );
    assert_eq!(DropZone::Center.split(), None);
}

#[test]
fn a_dimension_sits_after_type() {
    let mut node = Map::new();
    node.insert("id".into(), json!("x"));
    node.insert("type".into(), json!("tabs"));
    node.insert("extra".into(), json!(1));
    set_dimension(&mut node, Some(40.0));
    let keys: Vec<&String> = node.keys().collect();
    assert_eq!(keys, ["id", "type", "dimension", "extra"]);
    set_dimension(&mut node, None);
    assert!(!node.contains_key("dimension"));
}

#[test]
fn a_docks_current_tab_is_the_leaf_current_tab_points_at() {
    let layout = json!({
        "left": {"type": "split", "children": [{
            "type": "tabs",
            "currentTab": 1,
            "children": [
                {"type": "leaf", "state": {"type": "file-explorer"}},
                {"type": "leaf", "state": {"type": "search"}},
            ],
        }]},
        "right": {"type": "split", "children": [{
            "type": "tabs",
            "children": [{"type": "leaf", "state": {"type": "outgoing-link"}}],
        }]},
    });
    assert_eq!(
        dock_current_view(&layout, "left").as_deref(),
        Some("search")
    );
    assert_eq!(
        dock_current_view(&layout, "right").as_deref(),
        Some("outgoing-link")
    );
    assert_eq!(dock_current_view(&layout, "main"), None);
}

#[test]
fn prune_history_forgets_a_deleted_note() {
    let mut h = history(&["A.md", "B.md", "C.md"]);
    assert_eq!(prune_history(&mut h, 2, "B.md"), 1);
    assert_eq!(h, history(&["A.md", "C.md"]));

    let mut h = history(&["A.md", "B.md", "C.md"]);
    assert_eq!(prune_history(&mut h, 3, "A.md"), 2, "still at C.md");
    assert_eq!(h, history(&["B.md", "C.md"]));

    let mut h = history(&["A.md", "B.md"]);
    assert_eq!(prune_history(&mut h, 1, "B.md"), 1, "after the cursor");
    assert_eq!(h, history(&["A.md"]));
}

/// Deleting a folder deletes every note under it, and a repeat visit is
/// a repeat entry, so more than one entry can go at once.
#[test]
fn prune_history_forgets_a_whole_folder() {
    let mut h = history(&["Sub/S.md", "A.md", "Sub/T.md", "A.md"]);
    assert_eq!(prune_history(&mut h, 4, "Sub"), 2);
    assert_eq!(h, history(&["A.md", "A.md"]));

    let mut h = history(&["A.md", "A.md", "B.md"]);
    assert_eq!(prune_history(&mut h, 3, "A.md"), 1);
    assert_eq!(h, history(&["B.md"]));
}

/// A path the history never held, and a prefix that is not a folder
/// boundary, leave it alone.
#[test]
fn prune_history_keeps_what_it_does_not_name() {
    let mut h = history(&["A.md", "Subtotal/S.md"]);
    assert_eq!(prune_history(&mut h, 2, "Sub"), 2);
    assert_eq!(h, history(&["A.md", "Subtotal/S.md"]));

    let mut h = history(&["A.md"]);
    assert_eq!(prune_history(&mut h, 0, "A.md"), 0, "nothing open");
    assert!(h.is_empty());
}

/// The block id write: it reaches the target's bytes, keeps that
/// note's own line endings, and refuses a file it did not plan from.
#[test]
fn writing_a_block_id_reaches_the_disk_and_refuses_a_stale_file() {
    use tft_suggest::blocks::{plan_id, Placement};
    let (dir, state) = vault();
    let path = "A.md";
    let before = fs::read_to_string(dir.path().join(path)).unwrap();
    let end = before.find("See [[B]]").unwrap() + "See [[B]] and [S](Sub/S.md).".len();
    let insertion = plan_id(&before, end, Placement::Inline, "abc123", "\n").expect("a plan");
    state.write_block_id(path, &before, &insertion).unwrap();
    let after = fs::read_to_string(dir.path().join(path)).unwrap();
    assert_eq!(after, insertion.applied(&before));
    assert!(
        after.contains("^abc123"),
        "the id is not in the file: {after}"
    );

    // The same insertion against bytes the file no longer holds is
    // refused rather than written.
    let err = state
        .write_block_id(path, &before, &insertion)
        .expect_err("the file changed since");
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join(path)).unwrap(),
        after,
        "a refused write still changed the file"
    );
}

/// A CRLF note keeps its own endings when an id is written into it,
/// which is where TFT deliberately differs from Obsidian.
#[test]
fn a_block_id_written_into_a_crlf_note_keeps_its_endings() {
    use tft_suggest::blocks::{plan_id, Placement};
    let (dir, state) = vault();
    let path = "Crlf.md";
    let before = "# Crlf\r\n\r\npara in a CRLF file.\r\n";
    fs::write(dir.path().join(path), before).unwrap();
    let end = before.find("para in a CRLF file.").unwrap() + "para in a CRLF file.".len();
    let insertion = plan_id(before, end, Placement::Inline, "abc123", "\r\n").expect("a plan");
    state.write_block_id(path, before, &insertion).unwrap();
    let after = fs::read_to_string(dir.path().join(path)).unwrap();
    assert_eq!(after, "# Crlf\r\n\r\npara in a CRLF file. ^abc123\r\n");
    assert!(
        !after.contains("\n\n"),
        "a bare newline reached a CRLF note"
    );
}

/// The removed features left no runnable or editor-only id behind: the
/// command registry no longer knows them, and the workspace must not
/// offer them either.
#[test]
fn removed_features_leave_no_runnable_or_editor_only_ids() {
    for id in [
        "bases:new-file",
        "bases:insert",
        "bases:copy-table",
        "bases:change-view",
        "bases:add-view",
        "bases:add-item",
        "command-palette:open",
        "note-composer:merge-file",
        "note-composer:split-file",
        "note-composer:extract-heading",
        "file-recovery:open",
        "insert-template",
        "insert-current-date",
        "insert-current-time",
        "daily-notes",
        "daily-notes:goto-prev",
        "daily-notes:goto-next",
        "sync:setup",
        "audio-recorder:start",
        "audio-recorder:stop",
        "footnotes:open",
        "publish:publish-file",
        "slides:start",
        "webviewer:open",
        "workspaces:save",
        "zk-prefixer",
        "insert-unique-link",
        "random-note",
        "bookmarks:open",
        "bookmarks:bookmark-all-tabs",
        "backlink:open",
        "backlink:toggle-backlinks-in-document",
    ] {
        assert!(!RUNNABLE.contains(&id), "{id} is still runnable");
        assert!(!EDITOR_ONLY.contains(&id), "{id} is still editor-only");
    }
}

/// The probe's heading-rename vault: `Note.md` and its `Linker.md`.
fn heading_vault(app_json: &str) -> (tempfile::TempDir, VaultState) {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".tft")).unwrap();
    fs::write(r.join(".tft/app.json"), app_json).unwrap();
    fs::write(r.join("Note.md"), "# Parent\n\n## Old\n\nUnder old. Self [[#Old]] and [[#Old|self alias]] and [[Note#Old]].\n\nSetext Old\n----------\n\nUnder setext.\n").unwrap();
    fs::write(r.join("Linker.md"), "A [[Note#Old]]\nB [[Note#Old|alias]]\nC ![[Note#Old]]\nD [x](Note.md#Old)\nE [[Note#Parent#Old]]\nF [[Note#Old|Old]]\nG [[Note#old]]\nH [[Note#Old|Note > Old]]\nI [y](<Note.md#Old>)\nJ [[Note#Setext Old]] and [z](Note.md#Setext%20Old)\nK [[Note]]\n").unwrap();
    let state = VaultState::load(&r.to_path_buf()).unwrap();
    (dir, state)
}

fn plan_heading(state: &VaultState, own: &str, name: &str) -> tft_cache::HeadingRename {
    let texts = state.texts();
    state
        .cache()
        .plan_heading_rename("Note.md", own, 2, name, &|p| {
            texts.get(p).map(|t| t.to_string())
        })
        .unwrap()
}

/// `editor:rename-heading` writes every linking note on disk and in the
/// index, with the probe's bytes -- and the same bytes with
/// `alwaysUpdateLinks` off, which Obsidian does not consult here.
#[test]
fn heading_rename_rewrites_linking_notes_on_disk_whatever_always_update_links_says() {
    let expected = "A [[Note#New]]\nB [[Note#New|alias]]\nC ![[Note#New]]\nD [x](Note.md#New)\nE [[Note#Parent#Old]]\nF [[Note#New|Old]]\nG [[Note#New]]\nH [[Note#New|Note > Old]]\nI [y](<Note.md#New>)\nJ [[Note#Setext Old]] and [z](Note.md#Setext%20Old)\nK [[Note]]\n";
    for app in [
        r#"{"alwaysUpdateLinks":true}"#,
        r#"{"alwaysUpdateLinks":false}"#,
    ] {
        let (dir, state) = heading_vault(app);
        let own = fs::read_to_string(dir.path().join("Note.md")).unwrap();
        let plan = plan_heading(&state, &own, "New");
        assert_eq!(
            tft_cache::apply_edits(&own, &plan.own),
            "# Parent\n\n## New\n\nUnder old. Self [[#New]] and [[#New|self alias]] and [[Note#New]].\n\nSetext Old\n----------\n\nUnder setext.\n"
        );
        let (changes, stale) = state.write_link_rewrites(&plan.rewrites).unwrap();
        assert!(stale.is_empty());
        assert_eq!(changes.notes, vec!["Linker.md".to_string()]);
        assert_eq!(
            fs::read_to_string(dir.path().join("Linker.md")).unwrap(),
            expected,
            "{app}"
        );
        assert_eq!(state.texts()["Linker.md"].as_str(), expected);
        // The index follows: the rewritten links point at the new heading.
        assert!(state
            .cache()
            .file("Linker.md")
            .unwrap()
            .links
            .iter()
            .any(|l| l.link == "Note#New"));
    }
}

/// A linking note another app saved since it was indexed keeps its own
/// bytes, as a file rename leaves it.
#[test]
fn heading_rename_leaves_a_linking_note_that_changed_underneath_alone() {
    let (dir, state) = heading_vault("{}");
    let own = fs::read_to_string(dir.path().join("Note.md")).unwrap();
    let plan = plan_heading(&state, &own, "New");
    let theirs = "A [[Note#Old]] and the other app's edit\n";
    fs::write(dir.path().join("Linker.md"), theirs).unwrap();
    let (changes, stale) = state.write_link_rewrites(&plan.rewrites).unwrap();
    assert_eq!(stale, vec!["Linker.md".to_string()]);
    assert!(changes.files.is_empty());
    assert_eq!(
        fs::read_to_string(dir.path().join("Linker.md")).unwrap(),
        theirs
    );
}

/// Both commands run, the way their siblings do: rename-heading and
/// add-alias are offered only with an editor holding the keys.
#[test]
fn rename_heading_and_add_alias_are_runnable_editor_commands() {
    let known = tft_commands::known_ids();
    for id in ["editor:rename-heading", "markdown:add-alias"] {
        assert!(RUNNABLE.contains(&id), "{id} runnable");
        assert!(EDITOR_ONLY.contains(&id), "{id} needs the note");
        assert!(known.contains(&id), "{id} is a known command ID");
    }
}

#[test]
fn rename_leaves_a_note_that_changed_underneath_alone() {
    // The rewrites are planned from the text the index holds. If the
    // other app saved one of those notes since, writing the plan would
    // throw that save away, so the note keeps its own bytes.
    let (dir, state) = vault();
    let theirs = "# B\n\nRewritten by the other app, with [[A]] still in it.\n";
    fs::write(dir.path().join("B.md"), theirs).unwrap();
    let outcome = state.rename("A.md", "Renamed A.md", true).unwrap();
    assert_eq!(outcome.stale, vec!["B.md".to_string()]);
    assert_eq!(
        fs::read_to_string(dir.path().join("B.md")).unwrap(),
        theirs,
        "the other app's save was overwritten"
    );
    // The notes that did not change are still rewritten.
    assert!(outcome.rewritten_files >= 1);
    let sub = fs::read_to_string(dir.path().join("Sub/S.md")).unwrap();
    assert!(
        sub.contains("Renamed A"),
        "an untouched note was not rewritten: {sub}"
    );
}

#[test]
fn rename_rewrites_links_on_disk_and_in_the_index() {
    let (dir, state) = vault();
    let outcome = state.rename("A.md", "Renamed A.md", true).unwrap();
    assert_eq!(
        outcome.renamed,
        vec![("A.md".to_string(), "Renamed A.md".to_string())]
    );
    assert_eq!((outcome.rewritten_files, outcome.rewritten_links), (2, 4));
    let b = fs::read_to_string(dir.path().join("B.md")).unwrap();
    assert_eq!(
        b,
        "\u{feff}---\r\nrelated: \"[[Renamed A]]\"\r\n---\r\n[[Renamed A]] and [[Renamed A#A|the a]] and `[[A]]`\r\n",
        "the BOM and CRLF endings survive"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("Sub/S.md")).unwrap(),
        "[Renamed A](Renamed%20A.md) [[B]]\n"
    );
    assert!(dir.path().join("Renamed A.md").is_file());
    assert!(!dir.path().join("A.md").exists());
    {
        let cache = state.cache();
        assert!(cache.file("A.md").is_none());
        assert!(cache.file("Renamed A.md").is_some());
        // B's `related` property, its two body links, and S's markdown link.
        assert_eq!(cache.backlinks("Renamed A.md").len(), 4);
        assert!(cache.unresolved_links("B.md").is_none());
        assert_eq!(state.texts()["B.md"].as_str(), b);
    }
    assert!(outcome.changes.structural());
    // The watcher's events for the same paths find nothing new.
    let changes = state.apply(&[
        dir.path().join("A.md"),
        dir.path().join("Renamed A.md"),
        dir.path().join("B.md"),
    ]);
    assert!(changes
        .files
        .iter()
        .all(|c| matches!(c, FileChange::Modified(_))));
    assert_eq!(state.cache().backlinks("Renamed A.md").len(), 4);
}

#[test]
fn rename_without_updating_links_leaves_them_dangling() {
    let (dir, state) = vault();
    let outcome = state.rename("A.md", "A2.md", false).unwrap();
    assert_eq!(outcome.rewritten_links, 0);
    assert!(fs::read_to_string(dir.path().join("B.md"))
        .unwrap()
        .contains("[[A]] and"));
    assert_eq!(
        state.cache().unresolved_links("B.md").unwrap().get("A"),
        Some(&3),
        "two body links and the frontmatter link"
    );
    assert!(
        state.rename("A2.md", "B.md", true).is_err(),
        "destination exists"
    );
    assert_eq!(
        state.rename_preview("B.md", "Sub/B.md"),
        (0, 0),
        "a unique name moving folders changes no shortest link"
    );
    assert_eq!(state.rename_preview("B.md", "Sub/Bee.md"), (2, 2));
}

/// `alwaysUpdateLinks` absent means **ask**, not rewrite. Obsidian
/// rendered the toggle off on a vault whose `app.json` was `{}`
/// (`tools/reference-probe/results/2026-10-25-ui-rules.json`), and its own
/// description is "Turn off to be prompted to update links after renaming
/// a file". The gate's day 2 found this inverted, which made the
/// "Update links?" prompt unreachable on every vault that has never
/// written the key — the gate vault among them.
#[test]
fn a_vault_with_no_always_update_links_key_asks_before_rewriting() {
    let (_dir, state) = vault();
    assert!(
        state.app().always_update_links.is_none(),
        "the fixture must not write the key, or this asserts nothing"
    );
    assert!(
        !state.update_links_automatically(),
        "no key means ask, matching the unchecked toggle"
    );
    // And there is something to ask about, so the prompt is reached
    // rather than skipped by the `links == 0` shortcut.
    let (files, links) = state.rename_preview("A.md", "A2.md");
    assert!(
        files > 0 && links > 0,
        "renaming A should affect links, got {links} in {files}"
    );
}

/// Obsidian's modal, string for string and button for button
/// (`tools/reference-probe/results/2026-10-25-explorer-dnd.json`, `s05`).
/// The counts there were both 1, which is why the singular is the case
/// pinned to the probe and the plural is the inference.
#[test]
fn the_update_links_prompt_is_references_three_buttons_and_two_lines() {
    let one = update_links_prompt(1, 1);
    assert_eq!(one.title, "Update links");
    assert_eq!(
        one.detail,
        vec![
            "Do you want to update internal links that link to this file?",
            "This will affect 1 link in 1 file."
        ],
        "the two lines the probe read, concatenated, were exactly these"
    );
    let row: Vec<(&str, PromptAction, bool)> = one
        .buttons
        .iter()
        .map(|b| (b.label.as_ref(), b.action, b.cta))
        .collect();
    assert_eq!(
        row,
        vec![
            ("Always update", PromptAction::Tertiary, false),
            ("Just once", PromptAction::Submit, false),
            ("Do not update", PromptAction::Secondary, false),
        ],
        "the expected order, no accent on any, and no Cancel -- the modal has three buttons"
    );
    assert_eq!(
        update_links_prompt(6, 6).detail[1],
        "This will affect 6 links in 6 files.",
        "both nouns pluralise"
    );
}

/// A write through the control socket can only land inside the vault
/// folder, and the path it is given is usually one that does not exist
/// yet, so the folders it goes through are what the check has to look
/// at.
#[test]
fn vault_paths_cannot_leave_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("Sub")).unwrap();
    fs::write(root.join("A.md"), "a").unwrap();

    assert_eq!(vault_path_in(&root, "A.md").unwrap(), root.join("A.md"));
    assert_eq!(
        vault_path_in(&root, "Sub/New.md").unwrap(),
        root.join("Sub/New.md"),
        "a note that is not there yet, under a folder that is"
    );
    assert_eq!(vault_path_in(&root, "./A.md").unwrap(), root.join("A.md"));

    assert!(vault_path_in(&root, "../escape.md").is_err());
    assert!(vault_path_in(&root, "Sub/../../escape.md").is_err());
    assert!(vault_path_in(&root, "/etc/passwd").is_err());
    assert!(vault_path_in(&root, "").is_err(), "the folder itself");

    // A symlinked folder in the vault: the path is innocent, the folder
    // it goes through is not.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("there.md"), "o").unwrap();
    if symlink(outside.path(), &root.join("linked")).is_none() {
        return;
    }
    assert!(vault_path_in(&root, "linked/new.md").is_err());
    assert!(vault_path_in(&root, "linked/there.md").is_err());

    // A dangling link resolves to nothing, and a write through it would
    // create the file wherever it points.
    if symlink(&outside.path().join("gone.md"), &root.join("dangling.md")).is_none() {
        return;
    }
    assert!(vault_path_in(&root, "dangling.md").is_err());
}

/// A file kind reaches the state dump in lower case.
#[test]
fn file_kinds_are_named_in_lower_case() {
    assert_eq!(kind_name(FileKind::Markdown), "markdown");
    assert_eq!(kind_name(FileKind::Other), "other");
}

#[test]
fn create_duplicate_and_trash_keep_the_index_current() {
    let (dir, state) = vault();
    assert_eq!(
        state.new_note_path("Missing", Some("Sub/S.md")),
        "Missing.md"
    );
    assert_eq!(
        state.new_note_path("Deep/Also Missing", None),
        "Deep/Also Missing.md"
    );
    let changes = state.create_note("Deep/Also Missing.md", "").unwrap();
    assert_eq!(
        changes.files,
        vec![FileChange::Added("Deep/Also Missing.md".into())]
    );
    assert!(state.cache().file("Deep/Also Missing.md").is_some());
    assert!(state.vault().folders().contains(&"Deep".to_string()));
    let (copy, _) = state.duplicate("A.md").unwrap();
    assert_eq!(copy, "A 1.md");
    assert_eq!(
        fs::read_to_string(dir.path().join("A 1.md")).unwrap(),
        fs::read_to_string(dir.path().join("A.md")).unwrap()
    );
    let changes = state.trash("Sub").unwrap();
    assert_eq!(changes.removed_notes, vec!["Sub/S.md"]);
    assert!(dir.path().join(".trash/Sub/S.md").is_file());
    assert!(state.cache().file("Sub/S.md").is_none());
    assert_eq!(state.cache().backlinks("B.md").len(), 2, "A and its copy");
    assert!(state.vault().folders().iter().all(|f| f != "Sub"));
}

/// The socket's `focus` may not park the keyboard on a view the main
/// area is not painting: that handle is in no frame's dispatch tree, and
/// from there the app answers no key at all, the one that would undo it
/// included.
#[test]
fn focus_refuses_a_view_the_main_area_is_not_painting() {
    assert_eq!(
        Workspace::focus_target("editor", MainView::Editor),
        Ok(Some(MainView::Editor))
    );
    assert_eq!(
        Workspace::focus_target("workspace", MainView::Empty),
        Ok(None),
        "the root div paints in every frame"
    );
    let err = Workspace::focus_target("reading", MainView::Editor).unwrap_err();
    assert!(err.contains("painting editor, not reading"), "{err}");
    let err = Workspace::focus_target("graph", MainView::Empty).unwrap_err();
    assert!(err.contains("painting nothing, not graph"), "{err}");
    assert_eq!(
        Workspace::focus_target("pane", MainView::Editor),
        Err("unknown view: pane".to_string())
    );
}

/// The socket's name for the PDF view is `pdf`, the same three ways;
/// its page box is a field of its own, so `type` reaches it the way it
/// reaches any input.
#[test]
fn the_socket_calls_the_pdf_view_pdf() {
    assert_eq!(MainView::Pdf.as_str(), "pdf");
    assert_eq!(
        Workspace::focus_target("pdf", MainView::Pdf),
        Ok(Some(MainView::Pdf))
    );
    let err = Workspace::focus_target("pdf", MainView::Editor).unwrap_err();
    assert!(err.contains("painting editor, not pdf"), "{err}");
    assert_eq!(Workspace::typed_text_target("pdf", MainView::Pdf), "none");
}

/// An editor whose buffer names no file never gets the main area: on
/// launch over a vault with nothing to open the buffer belongs to
/// nothing, and anything typed into it would be saved nowhere.
#[test]
fn the_main_area_paints_no_editor_without_a_file_behind_it() {
    for mode in [ViewMode::LivePreview, ViewMode::Source] {
        assert_eq!(
            Workspace::main_view_for(mode, false, false, false, false, false),
            MainView::Empty,
            "{mode:?} with nothing open"
        );
        assert_eq!(
            Workspace::main_view_for(mode, true, false, true, false, false),
            MainView::Editor,
            "{mode:?} with a note open"
        );
        // The `--synthetic` spike hands the editor a note that is in no
        // vault: the buffer names a file, and that is what mounts it.
        assert_eq!(
            Workspace::main_view_for(mode, false, false, true, false, false),
            MainView::Editor,
            "{mode:?} with a buffer that belongs to no vault"
        );
        assert_eq!(
            Workspace::main_view_for(mode, true, true, true, false, false),
            MainView::Reading,
            "{mode:?} over an image"
        );
    }
    assert_eq!(
        Workspace::main_view_for(ViewMode::Reading, false, false, true, false, false),
        MainView::Empty,
        "a buffer is not a built note"
    );
    assert_eq!(
        Workspace::main_view_for(ViewMode::Graph, false, false, false, false, false),
        MainView::Graph,
        "the graph needs no file"
    );
    for mode in [ViewMode::Reading, ViewMode::LivePreview, ViewMode::Source] {
        assert_eq!(
            Workspace::main_view_for(mode, false, false, false, true, false),
            MainView::Canvas,
            "{mode:?} over a canvas"
        );
        assert_eq!(
            Workspace::main_view_for(mode, false, false, false, false, true),
            MainView::Pdf,
            "{mode:?} over a PDF"
        );
    }
    assert_eq!(
        Workspace::main_view_for(ViewMode::Graph, false, false, false, false, true),
        MainView::Graph,
        "the graph wins over a PDF"
    );
    // The graph is the one view that outranks a file: a canvas open
    // behind it stays behind it.
    assert_eq!(
        Workspace::main_view_for(ViewMode::Graph, false, false, false, true, false),
        MainView::Graph,
        "the graph wins over a canvas"
    );
}

/// The socket's `type` reaches the editor only while the main area is
/// painting it. The editor's handle can still hold the focus under the
/// empty state, and this route does not go through the dispatch tree
/// that would otherwise refuse it.
#[test]
fn typing_reaches_no_editor_the_main_area_is_not_painting() {
    for focus in ["editor", "property-field", "cell-field", "title-field"] {
        assert_eq!(
            Workspace::typed_text_target(focus, MainView::Editor),
            "editor"
        );
        assert_eq!(Workspace::typed_text_target(focus, MainView::Empty), "none");
        assert_eq!(
            Workspace::typed_text_target(focus, MainView::Reading),
            "none"
        );
    }
    assert_eq!(
        Workspace::typed_text_target("prompt-input", MainView::Empty),
        "input",
        "a modal's field is on screen whatever the main area paints"
    );
    assert_eq!(
        Workspace::typed_text_target("workspace", MainView::Editor),
        "none"
    );
}

/// A field of the File properties pane is the editor's, and the pane is
/// on screen whatever the main area paints, so typed text reaches it in
/// reading view too; the pane with no field takes none.
#[test]
fn typing_reaches_a_file_properties_field_in_every_view() {
    for painted in [MainView::Editor, MainView::Reading, MainView::Empty] {
        assert_eq!(
            Workspace::typed_text_target("file-properties-field", painted),
            "editor"
        );
        assert_eq!(
            Workspace::typed_text_target("file-properties", painted),
            "none"
        );
    }
}

/// A notice leaves on its own timer, and only the one that timer named:
/// a message put up later stays until its own turn comes.
#[test]
fn a_notice_expires_by_id_and_leaves_the_newer_one() {
    let mut n = Notices::default();
    let first = n.push("The cursor is not in a table".into(), None);
    let second = n.push("A.md saved".into(), None);
    assert_eq!(n.latest().map(|t| t.as_ref()), Some("A.md saved"));

    n.expire(first);
    assert_eq!(n.texts(), vec!["A.md saved".to_string()]);
    n.expire(second);
    assert!(n.texts().is_empty());
    assert!(n.latest().is_none(), "nothing is left to misread");
}

/// The autosave retries on a timer, so the same refusal arrives again
/// and again: it renews the message on screen instead of stacking
/// copies of it, and the renewed one outlives the first one's timer.
#[test]
fn a_repeated_notice_is_renewed_rather_than_stacked() {
    let mut n = Notices::default();
    let first = n.push("A.md changed on disk".into(), None);
    let again = n.push("A.md changed on disk".into(), None);
    assert_ne!(first, again);
    assert_eq!(n.texts(), vec!["A.md changed on disk".to_string()]);

    n.expire(first);
    assert_eq!(
        n.texts(),
        vec!["A.md changed on disk".to_string()],
        "the first timer must not take the renewed message down"
    );
    n.expire(again);
    assert!(n.texts().is_empty());
}

/// Clearing takes down everything, which is what the control socket's
/// `notice_clear` is for.
#[test]
fn clearing_takes_every_notice_down() {
    let mut n = Notices::default();
    n.push("one".into(), None);
    n.push("two".into(), None);
    n.clear();
    assert!(n.latest().is_none());
    assert_eq!(n.push("three".into(), None), 3, "ids keep counting up");
}

/// A notice stays in the window it went up in, whichever window has
/// the focus later, and falls back to the vault window once its popout
/// closes; a renewal moves it to the window of the renewal.
#[test]
fn a_notice_draws_in_its_own_window_until_that_closes() {
    let mut n = Notices::default();
    n.push("in the vault window".into(), None);
    n.push("in the popout".into(), Some(7));
    let texts = |it: &mut dyn Iterator<Item = &Notice>| -> Vec<String> {
        it.map(|n| n.text.to_string()).collect()
    };

    let open = |id| id == 7;
    assert_eq!(
        texts(&mut n.in_window(None, &open)),
        vec!["in the vault window"]
    );
    assert_eq!(
        texts(&mut n.in_window(Some(7), &open)),
        vec!["in the popout"]
    );
    assert_eq!(n.windows(&open), vec![None, Some(7)]);

    let closed = |_| false;
    assert_eq!(
        texts(&mut n.in_window(None, &closed)),
        vec!["in the vault window", "in the popout"],
        "a closed popout hands its notices to the vault window"
    );
    assert_eq!(n.windows(&closed), vec![None, None]);

    n.push("in the vault window".into(), Some(7));
    assert_eq!(n.windows(&open), vec![Some(7), Some(7)]);
}
