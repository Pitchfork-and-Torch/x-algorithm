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
    /// Confirmed calendar age in `[1, 18)`. `0` and negatives are sentinels, not a child.
    pub fn viewer_is_underage(&self) -> bool {
        matches!(self.viewer, Viewer::LoggedIn(_))
            && matches!(self.viewer_age, ViewerAge::Known(age) if (1..ADULT_AGE_YEARS).contains(&age))
    }

    pub fn viewer_has_no_stated_age(&self) -> bool {
        matches!(self.viewer, Viewer::LoggedIn(_)) && self.viewer_age == ViewerAge::NotStated
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

#[cfg(test)]
mod tests {
    use super::*;

    fn logged_in(age: ViewerAge) -> ViewerFeatures {
        ViewerFeatures {
            viewer: Viewer::LoggedIn(1),
            viewer_age: age,
            ..Default::default()
        }
    }

    #[test]
    fn known_zero_is_not_underage() {
        let v = logged_in(ViewerAge::Known(0));
        assert!(!v.viewer_is_underage());
        assert!(!v.viewer_has_no_stated_age());
    }

    #[test]
    fn negative_known_age_is_not_underage() {
        assert!(!logged_in(ViewerAge::Known(-1)).viewer_is_underage());
    }

    #[test]
    fn fifteen_is_underage_eighteen_is_not() {
        assert!(logged_in(ViewerAge::Known(15)).viewer_is_underage());
        assert!(logged_in(ViewerAge::Known(17)).viewer_is_underage());
        assert!(!logged_in(ViewerAge::Known(18)).viewer_is_underage());
    }

    #[test]
    fn logged_out_known_age_is_not_underage() {
        let v = ViewerFeatures {
            viewer: Viewer::LoggedOut,
            viewer_age: ViewerAge::Known(15),
            ..Default::default()
        };
        assert!(!v.viewer_is_underage());
        assert!(v.viewer_is_logged_out());
    }

    #[test]
    fn unknown_and_not_stated_are_not_underage() {
        assert!(!logged_in(ViewerAge::Unknown).viewer_is_underage());
        assert!(!logged_in(ViewerAge::NotStated).viewer_is_underage());
        assert!(logged_in(ViewerAge::NotStated).viewer_has_no_stated_age());
    }
}
