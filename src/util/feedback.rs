//! Footer "feedback" link target — composes a mailto: URL pre-filled with
//! plugin version, OS, and arch so user reports land with triage context
//! already attached. The link in the editor footer hands off to the system
//! mail client via the `webbrowser` crate (xdg-open / open / start).

const FEEDBACK_EMAIL: &str = "feedback@hyperfocusdsp.com";
const SUBJECT: &str = "Niner feedback";

pub fn build_mailto_url() -> String {
    let body = format!(
        "Niner v{} on {} ({})\n\n[your thoughts here]",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    format!(
        "mailto:{}?subject={}&body={}",
        FEEDBACK_EMAIL,
        urlencoding::encode(SUBJECT),
        urlencoding::encode(&body),
    )
}

pub fn open_feedback() {
    let url = build_mailto_url();
    if let Err(e) = webbrowser::open(&url) {
        tracing::warn!("failed to open feedback mailto: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_url_contains_destination_subject_and_metadata() {
        let url = build_mailto_url();
        assert!(url.starts_with("mailto:feedback@hyperfocusdsp.com?"));
        assert!(url.contains("subject=Niner%20feedback"));
        assert!(url.contains(env!("CARGO_PKG_VERSION")));
        assert!(url.contains(std::env::consts::OS));
        assert!(url.contains(std::env::consts::ARCH));
    }
}
