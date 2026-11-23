//! Vault-relative path helpers shared by every backend.

/// The folder part of a vault path (`""` at the root).
pub fn folder_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// The folder a new attachment goes into for `attachmentFolderPath`.
pub fn attachment_folder(setting: Option<&str>, note: &str) -> String {
    let setting = setting.unwrap_or("");
    match setting.strip_prefix("./") {
        Some(under) => {
            let under = under.trim_matches('/');
            let folder = folder_of(note);
            if under.is_empty() {
                folder.to_string()
            } else {
                join(folder, under)
            }
        }
        None => setting.trim_matches('/').to_string(),
    }
}

/// The path the attachment `name` lands at.
pub fn attachment_path(setting: Option<&str>, note: &str, name: &str) -> String {
    join(&attachment_folder(setting, note), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_folder_settings_are_relative_absolute_or_root() {
        assert_eq!(attachment_folder(None, "Sub/Nested.md"), "");
        assert_eq!(
            attachment_folder(Some("./"), "Sub/Deep/Note.md"),
            "Sub/Deep"
        );
        assert_eq!(
            attachment_folder(Some("./Attachments/"), "Sub/Nested.md"),
            "Sub/Attachments"
        );
        assert_eq!(
            attachment_folder(Some("/Attachments/"), "Note.md"),
            "Attachments"
        );
        assert_eq!(
            attachment_path(Some("Attachments"), "Target.md", "photo.png"),
            "Attachments/photo.png"
        );
    }
}
