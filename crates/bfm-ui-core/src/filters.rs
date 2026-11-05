//! What to ask for when starting a session.
//!
//! Activities and genres are *per mental state* — Focus offers ten base
//! genres, Sleep offers one — so a front-end cannot hold a single global
//! filter set and expect it to mean anything. These types carry the catalog
//! for one state alongside the user's choices within it.

/// How strong an effect to ask for.
///
/// The request side of the API takes these as enum strings while the
/// response reports a float, so the two are deliberately not one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    Low,
    Medium,
    #[default]
    High,
}

impl Effort {
    pub const ALL: [Effort; 3] = [Effort::Low, Effort::Medium, Effort::High];

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    /// What the level actually means, since "neural effect" is not
    /// self-explanatory.
    pub fn description(self) -> &'static str {
        match self {
            Self::Low => "Gentlest — for sensitive listeners",
            Self::Medium => "Balanced",
            Self::High => "Strongest — the default",
        }
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).copied().unwrap_or_default()
    }

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|e| *e == self).unwrap_or(2) as u32
    }
}

/// The catalog for one mental state, as the daemon reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub state: String,
    /// `(id, display name)`. The first is the state's default.
    pub activities: Vec<(String, String)>,
    pub genres: Vec<String>,
    /// `(seconds, label)` suggested by the service. Interface version 2.
    pub lengths: Vec<(u32, String)>,
}

impl Catalog {
    pub fn activity_names(&self) -> Vec<&str> {
        self.activities.iter().map(|(_, n)| n.as_str()).collect()
    }

    pub fn activity_id(&self, index: u32) -> Option<&str> {
        self.activities
            .get(index as usize)
            .map(|(id, _)| id.as_str())
    }

    pub fn index_of_activity(&self, id: &str) -> u32 {
        self.activities
            .iter()
            .position(|(a, _)| a == id)
            .unwrap_or(0) as u32
    }
}

/// A user's choices for the session they are about to start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    /// Empty means the state's default activity.
    pub activity: String,
    pub effort: Effort,
    /// Empty means no genre filter at all, which is not the same as every
    /// genre selected — the service picks freely rather than being pinned.
    pub genres: Vec<String>,
}

impl Filters {
    /// Toggle one genre, keeping the list in catalog order so the request is
    /// stable regardless of the order the user clicked.
    pub fn toggle_genre(&mut self, genre: &str, on: bool, catalog_order: &[String]) {
        self.genres.retain(|g| g != genre);
        if on {
            self.genres.push(genre.to_owned());
        }
        self.genres.sort_by_key(|g| {
            catalog_order
                .iter()
                .position(|c| c == g)
                .unwrap_or(usize::MAX)
        });
    }

    pub fn has_genre(&self, genre: &str) -> bool {
        self.genres.iter().any(|g| g == genre)
    }

    /// A short line for the row that opens the editor.
    pub fn summary(&self, catalog: &Catalog) -> String {
        let activity = if self.activity.is_empty() {
            catalog
                .activities
                .first()
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "Default".into())
        } else {
            catalog
                .activities
                .iter()
                .find(|(id, _)| *id == self.activity)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| self.activity.clone())
        };

        let genres = match self.genres.len() {
            0 => "any genre".to_owned(),
            1 => self.genres[0].clone(),
            n => format!("{n} genres"),
        };
        format!("{activity} · {} effect · {genres}", self.effort.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog {
            state: "focus".into(),
            activities: vec![
                ("QYN78dJrrzs5mD_TJgBcq".into(), "Deep Work".into()),
                ("LpcmC-6QIHPot2sOVGV7B".into(), "Motivation".into()),
            ],
            genres: vec!["Acoustic".into(), "Electronic".into(), "Lofi".into()],
            lengths: vec![(3600, "1 hour".into())],
        }
    }

    #[test]
    fn effort_maps_to_the_wire_strings_the_api_wants() {
        assert_eq!(Effort::Low.as_wire(), "low");
        assert_eq!(Effort::Medium.as_wire(), "medium");
        assert_eq!(Effort::High.as_wire(), "high");
    }

    #[test]
    fn effort_defaults_to_high_and_round_trips_by_index() {
        assert_eq!(Effort::default(), Effort::High);
        for e in Effort::ALL {
            assert_eq!(Effort::from_index(e.index()), e);
        }
        // An out-of-range index must not panic the UI.
        assert_eq!(Effort::from_index(99), Effort::High);
    }

    #[test]
    fn an_empty_activity_means_the_states_default() {
        let c = catalog();
        let f = Filters::default();
        assert!(f.summary(&c).starts_with("Deep Work"), "{}", f.summary(&c));
    }

    #[test]
    fn activity_lookup_is_by_index_and_by_id() {
        let c = catalog();
        assert_eq!(c.activity_id(1), Some("LpcmC-6QIHPot2sOVGV7B"));
        assert_eq!(c.index_of_activity("LpcmC-6QIHPot2sOVGV7B"), 1);
        // Unknown ids fall back to the first rather than erroring.
        assert_eq!(c.index_of_activity("nope"), 0);
        assert_eq!(c.activity_id(99), None);
    }

    /// Request order should depend on the catalog, not on click order, so
    /// the same selection always produces the same request.
    #[test]
    fn genres_stay_in_catalog_order() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle_genre("Lofi", true, &c.genres);
        f.toggle_genre("Acoustic", true, &c.genres);
        f.toggle_genre("Electronic", true, &c.genres);
        assert_eq!(f.genres, vec!["Acoustic", "Electronic", "Lofi"]);
    }

    #[test]
    fn toggling_a_genre_off_removes_exactly_one() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle_genre("Lofi", true, &c.genres);
        f.toggle_genre("Acoustic", true, &c.genres);
        f.toggle_genre("Lofi", false, &c.genres);
        assert_eq!(f.genres, vec!["Acoustic"]);
        assert!(!f.has_genre("Lofi"));
        assert!(f.has_genre("Acoustic"));
    }

    #[test]
    fn toggling_the_same_genre_on_twice_does_not_duplicate_it() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle_genre("Lofi", true, &c.genres);
        f.toggle_genre("Lofi", true, &c.genres);
        assert_eq!(f.genres, vec!["Lofi"]);
    }

    /// No genres selected means "no filter", which is not the same as
    /// selecting all of them — the service picks freely instead of being
    /// pinned to a list.
    #[test]
    fn no_genres_reads_as_unfiltered() {
        let c = catalog();
        let f = Filters::default();
        assert!(f.genres.is_empty());
        assert!(f.summary(&c).contains("any genre"));
    }

    #[test]
    fn summary_counts_rather_than_listing_many_genres() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle_genre("Acoustic", true, &c.genres);
        assert!(f.summary(&c).contains("Acoustic"));
        f.toggle_genre("Lofi", true, &c.genres);
        assert!(f.summary(&c).contains("2 genres"));
    }
}
