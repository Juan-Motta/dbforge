//! The update dialog's texts, as the former web UI's `updates/messages.ts`.

/// The update dialog's texts in one language.
pub struct UpdateMessages {
    pub title: &'static str,
    pub checking: &'static str,
    /// The new version.
    pub available: fn(&str) -> String,
    /// The version installed.
    pub current: fn(&str) -> String,
    pub notes: &'static str,
    pub no_notes: &'static str,
    pub up_to_date: &'static str,
    /// The version installed, which is the latest.
    pub up_to_date_detail: fn(&str) -> String,
    pub downloading: &'static str,
    /// Megabytes so far, of how many.
    pub downloaded: fn(&str, &str) -> String,
    /// Megabytes so far, the total unknown.
    pub downloaded_so_far: fn(&str) -> String,
    /// After the download: checking the signature and putting the new version in place.
    pub installing: &'static str,
    pub failed: &'static str,
    pub later: &'static str,
    pub install: &'static str,
    pub retry: &'static str,
    pub ok: &'static str,
}
