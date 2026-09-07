#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Viewer {
    LoggedIn(u64),
    #[default]
    LoggedOut,
}

impl Viewer {
    pub fn user_id(&self) -> Option<u64> {
        match self {
            Viewer::LoggedIn(id) => Some(*id),
            Viewer::LoggedOut => None,
        }
    }
}

pub const ADULT_AGE_YEARS: i32 = 18;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewerAge {
    Known(i32),
    NotStated,
    /// Gizmoduck viewer RPC errored or timed out. Distinct from `Unknown`
    /// (no such user) so age gates can fail closed instead of serving NSFW.
    LookupFailed,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default)]
pub struct ViewerFeatures {
    pub viewer: Viewer,
    pub allows_sensitive_media: bool,
    pub country_code: Option<String>,
    pub account_country_code: Option<String>,
    pub viewer_age: ViewerAge,
}

impl ViewerFeatures {
    pub fn viewer_is_underage(&self) -> bool {
        matches!(self.viewer_age, ViewerAge::Known(age) if age < ADULT_AGE_YEARS)
    }

    pub fn viewer_has_no_stated_age(&self) -> bool {
        matches!(self.viewer, Viewer::LoggedIn(_)) && self.viewer_age == ViewerAge::NotStated
    }

    /// Viewer age could not be confirmed because gizmoduck failed. Sensitive
    /// media must hard-drop globally (same sink as underage), not fail open
    /// as `Unknown` and not jurisdiction-scope as `NotStated`.
    pub fn viewer_age_lookup_failed(&self) -> bool {
        matches!(self.viewer, Viewer::LoggedIn(_)) && self.viewer_age == ViewerAge::LookupFailed
    }
}

impl ViewerFeatures {
    pub fn viewer_id(&self) -> Option<u64> {
        self.viewer.user_id()
    }

    pub fn viewer_is_logged_out(&self) -> bool {
        matches!(self.viewer, Viewer::LoggedOut)
    }
}
