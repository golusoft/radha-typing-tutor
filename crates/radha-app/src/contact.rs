//! Contact and social links (HOME-01).
//!
//! The URLs live here, not in `.slint` (the asset policy forbids `http(s)`
//! strings in the UI layer, and Rule 16 keeps decisions in Rust). The UI
//! sends a fixed link id; only ids on this allow-list open anything, so no
//! UI-supplied text ever reaches the shell.

/// A support or social destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactLink {
    Website,
    WhatsApp,
    Phone,
    Instagram,
    Facebook,
}

impl ContactLink {
    /// Every link, in display order.
    #[must_use]
    pub fn all() -> [ContactLink; 5] {
        [
            ContactLink::Website,
            ContactLink::WhatsApp,
            ContactLink::Phone,
            ContactLink::Instagram,
            ContactLink::Facebook,
        ]
    }

    /// The id the UI sends.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            ContactLink::Website => "website",
            ContactLink::WhatsApp => "whatsapp",
            ContactLink::Phone => "phone",
            ContactLink::Instagram => "instagram",
            ContactLink::Facebook => "facebook",
        }
    }

    /// Parse [`ContactLink::id`].
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::all().into_iter().find(|l| l.id() == id)
    }

    /// Where the link goes.
    #[must_use]
    pub fn url(self) -> &'static str {
        match self {
            ContactLink::Website => "https://www.golupdf.online/",
            ContactLink::WhatsApp => "https://wa.me/918603064972",
            ContactLink::Phone => "tel:+918789086401",
            ContactLink::Instagram => {
                "https://www.instagram.com/golukumar_yt_?srtk=a3cxaXVlZ2FoeXg2"
            }
            ContactLink::Facebook => "https://www.facebook.com/share/1DQqQW6G6M/",
        }
    }
}

/// Open `link` with the system's default handler (browser, dialer).
///
/// # Errors
///
/// When the handler process cannot be started.
pub fn open(link: ContactLink) -> std::io::Result<()> {
    let url = link.url();
    // A fixed URL as a single argument: no shell, nothing to escape.
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer");
    #[cfg(not(windows))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_unknown_ids_open_nothing() {
        for link in ContactLink::all() {
            assert_eq!(ContactLink::from_id(link.id()), Some(link));
        }
        assert_eq!(ContactLink::from_id("youtube"), None);
        assert_eq!(ContactLink::from_id("https://example.invalid"), None);
    }

    #[test]
    fn links_carry_the_real_contact_details() {
        assert!(ContactLink::Website.url().contains("golupdf.online"));
        assert!(ContactLink::WhatsApp.url().ends_with("918603064972"));
        assert_eq!(ContactLink::Phone.url(), "tel:+918789086401");
        assert!(ContactLink::Instagram
            .url()
            .contains("instagram.com/golukumar_yt_"));
        assert!(ContactLink::Facebook
            .url()
            .contains("facebook.com/share/1DQqQW6G6M"));
    }
}
