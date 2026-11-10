//! Saved preferences of the optional add-ons ("Build with MIRA" and "Agent
//! connection"). They are always compiled, so a build without an add-on still
//! reads and writes back what a full build saved instead of dropping it.
//! The behaviour that uses them lives with each add-on.

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.endlesstokens.ai/v1";
pub(super) const DEFAULT_PORT: u16 = 47615;

/// Where the panel lives. Position is the outer window corner and size the
/// inner size, both in points; they are only meaningful while `detached`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct WindowPreferences {
    pub detached: bool,
    pub position: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
}

/// Remembered across restarts. The API key is deliberately not here.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct AssistantPreferences {
    pub base_url: String,
    pub model: String,
    pub include_selection: bool,
    /// The one-line "what is sent" notice was dismissed for good.
    pub privacy_dismissed: bool,
    pub window: WindowPreferences,
}
impl Default for AssistantPreferences {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            model: String::new(),
            include_selection: true,
            privacy_dismissed: false,
            window: WindowPreferences::default(),
        }
    }
}
/// Remembered across restarts (in `GeneralSettings`). The pairing token is
/// deliberately not here.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct AgentPreferences {
    pub enabled: bool,
    pub port: u16,
}
impl Default for AgentPreferences {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT }
    }
}
