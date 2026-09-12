//! In-app updates (mirrors `src/lib/types/updates.ts`): the release channel a
//! build follows, the feed each channel reads, and the status the HUD pill and
//! Settings → General → Updates render. The updater itself is
//! `tauri-plugin-updater` behind `crate::updates` in the app crate; everything
//! here is data (docs/UPDATES.md).

use serde::{Deserialize, Serialize};

use crate::BlueyError;

/// Stable releases: GitHub resolves `releases/latest` to the newest non-prerelease.
pub const LATEST_FEED_URL: &str =
    "https://github.com/bloxy-studios/bluey/releases/latest/download/latest.json";
/// The rolling `nightly` prerelease rebuilt from `main` (`.github/workflows/nightly.yml`).
pub const NIGHTLY_FEED_URL: &str =
    "https://github.com/bloxy-studios/bluey/releases/download/nightly/latest.json";

/// Where updates come from. `Latest` follows stable releases; `Nightly`
/// follows the prerelease built from `main` every night `main` changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Latest,
    Nightly,
}

impl UpdateChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::Nightly => "nightly",
        }
    }

    /// The static updater feed — `latest.json` in Tauri's updater format
    /// (`version`, `pub_date`, `notes`, `platforms.darwin-<arch>.{url,signature}`).
    pub fn feed_url(&self) -> &'static str {
        match self {
            Self::Latest => LATEST_FEED_URL,
            Self::Nightly => NIGHTLY_FEED_URL,
        }
    }
}

/// Where the updater is in its cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// A newer version exists; `available` describes it. With automatic
    /// updates the manager moves on to `Downloading` by itself.
    Available,
    Downloading,
    /// Downloaded, verified and installed on disk — relaunch to run it.
    Ready,
    Error,
}

/// The newer version a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub channel: UpdateChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// RFC 3339, from the feed's `pub_date`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// Mirrors `UpdateStatus`; published as `update.status` on every transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub phase: UpdatePhase,
    pub current_version: String,
    pub channel: UpdateChannel,
    pub automatic: bool,
    /// False in builds that cannot replace themselves (debug builds, the
    /// browser mock): a manual check still answers, nothing is installed.
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available: Option<AvailableUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<UpdateProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BlueyError>,
    /// RFC 3339 of the last completed check (success or failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
}

impl UpdateStatus {
    pub fn idle(
        current_version: impl Into<String>,
        channel: UpdateChannel,
        automatic: bool,
        supported: bool,
    ) -> Self {
        Self {
            phase: UpdatePhase::Idle,
            current_version: current_version.into(),
            channel,
            automatic,
            supported,
            available: None,
            progress: None,
            error: None,
            last_checked_at: None,
        }
    }

    /// A check or install is in flight.
    pub fn is_busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking | UpdatePhase::Downloading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_serialize_as_lowercase_tags_and_name_their_feeds() {
        assert_eq!(
            serde_json::to_value(UpdateChannel::Latest).unwrap(),
            "latest"
        );
        assert_eq!(
            serde_json::to_value(UpdateChannel::Nightly).unwrap(),
            "nightly"
        );
        assert_eq!(
            serde_json::from_value::<UpdateChannel>(serde_json::json!("nightly")).unwrap(),
            UpdateChannel::Nightly
        );
        assert_eq!(UpdateChannel::default(), UpdateChannel::Latest);
        assert!(UpdateChannel::Latest
            .feed_url()
            .ends_with("/releases/latest/download/latest.json"));
        assert!(UpdateChannel::Nightly
            .feed_url()
            .ends_with("/releases/download/nightly/latest.json"));
        for channel in [UpdateChannel::Latest, UpdateChannel::Nightly] {
            assert!(channel
                .feed_url()
                .starts_with("https://github.com/bloxy-studios/bluey/"));
        }
    }

    #[test]
    fn status_serializes_camel_case_with_snake_case_phases() {
        let mut status = UpdateStatus::idle("0.1.1", UpdateChannel::Nightly, true, true);
        status.phase = UpdatePhase::UpToDate;
        status.last_checked_at = Some("2026-09-12T16:00:00Z".into());
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["phase"], "up_to_date");
        assert_eq!(v["currentVersion"], "0.1.1");
        assert_eq!(v["channel"], "nightly");
        assert_eq!(v["automatic"], true);
        assert_eq!(v["supported"], true);
        assert_eq!(v["lastCheckedAt"], "2026-09-12T16:00:00Z");
        assert!(v.get("available").is_none(), "absent optionals are omitted");
        assert!(v.get("error").is_none());
        let back: UpdateStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back, status);
    }

    #[test]
    fn busy_only_while_checking_or_downloading() {
        let mut status = UpdateStatus::idle("0.1.1", UpdateChannel::Latest, false, true);
        assert!(!status.is_busy());
        status.phase = UpdatePhase::Checking;
        assert!(status.is_busy());
        status.phase = UpdatePhase::Downloading;
        assert!(status.is_busy());
        status.phase = UpdatePhase::Ready;
        assert!(!status.is_busy());
    }
}
