//! Remote destinations, as Obsidian's link and embed code sees them.

/// Whether a destination is fetched over `http:` or `https:`.
pub fn is_remote(destination: &str) -> bool {
    let d = destination.trim_start();
    ["http://", "https://"].iter().any(|scheme| {
        d.get(..scheme.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_are_remote() {
        assert!(is_remote("https://e.com/x.png"));
        assert!(is_remote("http://127.0.0.1:8731/x"));
        assert!(is_remote("HTTPS://E.COM/X.PNG"));
        assert!(!is_remote("//e.com/x.png"));
        assert!(!is_remote("file:///c/x.png"));
        assert!(!is_remote("data:image/png;base64,AAAA"));
        assert!(!is_remote("app://local/x.png"));
        assert!(!is_remote("pic.png"));
        assert!(!is_remote("http:"));
    }
}
