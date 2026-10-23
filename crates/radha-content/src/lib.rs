//! # Radha Content
//!
//! Exercise content model and the repository abstraction used to read it.
//!
//! ## Scope in Phase 0
//!
//! Scaffold only — no exercise text ships in this crate yet. Phase 14
//! (`Resource Migration`) imports the ~1100 exercises from the existing Golu
//! resources.
//!
//! ## Why a repository trait
//!
//! `RULES.MD` Rule 12 forbids hard-coding exercise content in Rust source. All
//! content is loaded through [`ExerciseRepository`], whose only production
//! implementation will be SQLite-backed (in `radha-db`). Tests can supply an
//! in-memory implementation without a database.
//!
//! Establishing the trait now means later phases add an implementation rather
//! than restructuring call sites.

use radha_layouts::LayoutId;

/// Which product an exercise belongs to.
///
/// Mirrors the nine Home-screen cards in `STRUCTURE.MD`. Closed enum so adding a
/// product is a compiler-checked change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductId {
    /// English Typing Mastery (Classic).
    EnglishClassic,
    /// Typing Flow — dedicated UX per `DESIGN.md` Section 8.
    TypingFlow,
    /// Number + Punctuation Mastery.
    NumberPunctuation,
    /// Hindi Typing Mastery, for one of the three Hindi layouts.
    HindiMastery(LayoutId),
    /// Hindi Learn (Beginner), for one of the three Hindi layouts.
    HindiLearn(LayoutId),
}

impl ProductId {
    /// Stable identifier, used as the SQLite `product` value and as the token the
    /// UI passes back when a card is clicked.
    ///
    /// A slug rather than a bare index: it survives reordering the Home screen,
    /// and a navigation log line reads as `hindi-learn-cbi` instead of `7`.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            ProductId::EnglishClassic => "english-classic",
            ProductId::TypingFlow => "typing-flow",
            ProductId::NumberPunctuation => "number-punctuation",
            ProductId::HindiMastery(LayoutId::Qwerty) => "hindi-mastery-qwerty",
            ProductId::HindiMastery(LayoutId::RemingtonGail) => "hindi-mastery-gail",
            ProductId::HindiMastery(LayoutId::RemingtonCbi) => "hindi-mastery-cbi",
            ProductId::HindiMastery(LayoutId::KrutiDev) => "hindi-mastery-krutidev",
            ProductId::HindiLearn(LayoutId::Qwerty) => "hindi-learn-qwerty",
            ProductId::HindiLearn(LayoutId::RemingtonGail) => "hindi-learn-gail",
            ProductId::HindiLearn(LayoutId::RemingtonCbi) => "hindi-learn-cbi",
            ProductId::HindiLearn(LayoutId::KrutiDev) => "hindi-learn-krutidev",
        }
    }

    /// Parse a [`ProductId::slug`] back into a product.
    ///
    /// # Errors
    ///
    /// Returns [`radha_core::CoreError::ResourceMissing`] for an unrecognised
    /// slug, so a stale or malformed value from the UI degrades gracefully rather
    /// than panicking (`RULES.MD` Rule 23).
    pub fn from_slug(slug: &str) -> radha_core::Result<Self> {
        ProductId::all()
            .into_iter()
            .find(|product| product.slug() == slug)
            .ok_or_else(|| radha_core::CoreError::ResourceMissing {
                path: std::path::PathBuf::from(format!("product:{slug}")),
            })
    }

    /// Name shown in the UI.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            ProductId::EnglishClassic => "English Typing Mastery",
            ProductId::TypingFlow => "Typing Flow",
            ProductId::NumberPunctuation => "Number + Punctuation Mastery",
            ProductId::HindiMastery(layout) => match layout {
                LayoutId::Qwerty => "Hindi Typing Mastery",
                LayoutId::RemingtonGail => "Remington GAIL Typing",
                LayoutId::RemingtonCbi => "Remington CBI Typing",
                LayoutId::KrutiDev => "Kruti Dev Typing",
            },
            ProductId::HindiLearn(layout) => match layout {
                LayoutId::Qwerty => "Hindi Learn",
                // The Learn cards drop the "Typing" suffix, per PRD.md Section 6.4.
                LayoutId::RemingtonGail => "Remington GAIL",
                LayoutId::RemingtonCbi => "Remington CBI",
                LayoutId::KrutiDev => "Kruti Dev",
            },
        }
    }

    /// The nine products shown on the Home screen, in presentation order.
    ///
    /// Excludes the `Qwerty` Hindi combinations, which the type system permits but
    /// the product does not have — Hindi is only ever GAIL, CBI, or Kruti Dev.
    #[must_use]
    pub fn all() -> Vec<ProductId> {
        vec![
            // English Typing Mastery
            ProductId::EnglishClassic,
            ProductId::TypingFlow,
            ProductId::NumberPunctuation,
            // Hindi Typing Mastery
            ProductId::HindiMastery(LayoutId::RemingtonGail),
            ProductId::HindiMastery(LayoutId::RemingtonCbi),
            ProductId::HindiMastery(LayoutId::KrutiDev),
            // Hindi Learn (Beginner)
            ProductId::HindiLearn(LayoutId::RemingtonGail),
            ProductId::HindiLearn(LayoutId::RemingtonCbi),
            ProductId::HindiLearn(LayoutId::KrutiDev),
        ]
    }

    /// The layout this product is typed in.
    #[must_use]
    pub fn layout(self) -> LayoutId {
        match self {
            // The three English products all use QWERTY.
            ProductId::EnglishClassic
            | ProductId::TypingFlow
            | ProductId::NumberPunctuation => LayoutId::Qwerty,
            // Hindi products carry their own layout.
            ProductId::HindiMastery(layout) | ProductId::HindiLearn(layout) => layout,
        }
    }

    /// Whether this product is a beginner (Hindi Learn) experience.
    ///
    /// Used by the UI to pick between the Mastery and Learn design systems
    /// (`DESIGN.md` Section 5).
    #[must_use]
    pub fn is_beginner(self) -> bool {
        matches!(self, ProductId::HindiLearn(_))
    }
}

/// A single typing exercise.
///
/// Field set is deliberately narrow for Phase 0 — it will grow with the schema
/// in Phase 3, but the shape (content is *data*, loaded from a repository) is
/// already fixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exercise {
    /// Stable database identifier.
    pub id: i64,
    /// Which product this belongs to.
    pub product: ProductId,
    /// Position within the product, 1-based, as shown in the UI
    /// ("Exercise 12 / 200").
    pub number: u32,
    /// Short title.
    pub title: String,
    /// The text the user must type.
    pub content: String,
}

/// Read access to exercise content.
///
/// The production implementation is SQLite-backed. Kept as a trait so the UI
/// and engine never depend on a concrete storage mechanism.
pub trait ExerciseRepository {
    /// Exercise metadata for a product, ordered by [`Exercise::number`].
    ///
    /// Implementations should avoid loading full `content` here — the Exercise
    /// screen shows a list of many exercises, and `PRD.md` Section 2.3 requires
    /// content to be loaded lazily, one exercise at a time.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store is unreachable.
    fn list_for_product(&self, product: ProductId) -> radha_core::Result<Vec<Exercise>>;

    /// Load one exercise, including its full content.
    ///
    /// # Errors
    ///
    /// Returns [`radha_core::CoreError::ResourceMissing`] if no exercise has
    /// that id.
    fn load(&self, exercise_id: i64) -> radha_core::Result<Exercise>;
}

/// A personal best: the highest figures recorded, per `TASK.MD` Section 32.
///
/// Each metric is an independent maximum, so they may come from different
/// sessions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonalBest {
    /// Best gross WPM.
    pub wpm: f64,
    /// Best net WPM.
    pub net_wpm: f64,
    /// Best accuracy, as a percentage.
    pub accuracy: f64,
    /// Best consistency percentage, once the engine measures it.
    pub consistency: Option<f64>,
    /// Best composite score. `None` until the Result engine defines its formula.
    pub score: Option<f64>,
}

/// Progress on one exercise for one profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExerciseProgress {
    /// Completed attempts.
    pub attempts: u32,
    /// Whether the exercise has been completed at least once.
    pub completed: bool,
    /// Total time spent on the exercise, in seconds.
    pub total_practice_seconds: u64,
}

/// Read access to personal bests and progress.
///
/// Read-only for now: the write path arrives with the Result screen, where a
/// completed session and its bests are saved in one transaction (`RULES.MD`
/// Rule 14).
///
/// Every method returns `Ok(None)` when nothing has been recorded. On a new
/// install that is the normal state, not a failure, and callers should present
/// it as "no best yet".
pub trait ProgressRepository {
    /// Best result on one exercise.
    ///
    /// # Errors
    ///
    /// Returns an error only if the store cannot be read.
    fn exercise_best(
        &self,
        profile_id: i64,
        exercise_id: i64,
    ) -> radha_core::Result<Option<PersonalBest>>;

    /// Best result across a whole product ("English Best", "GAIL Best", …).
    ///
    /// # Errors
    ///
    /// Returns an error only if the store cannot be read.
    fn product_best(
        &self,
        profile_id: i64,
        product: ProductId,
    ) -> radha_core::Result<Option<PersonalBest>>;

    /// Progress on one exercise.
    ///
    /// # Errors
    ///
    /// Returns an error only if the store cannot be read.
    fn exercise_progress(
        &self,
        profile_id: i64,
        exercise_id: i64,
    ) -> radha_core::Result<Option<ExerciseProgress>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_products_all_use_qwerty() {
        assert_eq!(ProductId::EnglishClassic.layout(), LayoutId::Qwerty);
        assert_eq!(ProductId::TypingFlow.layout(), LayoutId::Qwerty);
        assert_eq!(ProductId::NumberPunctuation.layout(), LayoutId::Qwerty);
    }

    #[test]
    fn hindi_products_carry_their_own_layout() {
        assert_eq!(
            ProductId::HindiMastery(LayoutId::RemingtonCbi).layout(),
            LayoutId::RemingtonCbi
        );
        assert_eq!(
            ProductId::HindiLearn(LayoutId::KrutiDev).layout(),
            LayoutId::KrutiDev
        );
    }

    #[test]
    fn home_screen_has_exactly_nine_products() {
        // PRD.md Section 6 / STRUCTURE.MD: three sections of three cards.
        assert_eq!(ProductId::all().len(), 9);
    }

    #[test]
    fn slugs_round_trip() {
        for product in ProductId::all() {
            let parsed = ProductId::from_slug(product.slug()).expect("known slug must parse");
            assert_eq!(parsed, product);
        }
    }

    #[test]
    fn slugs_are_unique() {
        // A duplicate slug would silently route two Home cards to the same place.
        let mut slugs: Vec<&str> = ProductId::all().iter().map(|p| p.slug()).collect();
        let total = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), total, "product slugs must be unique");
    }

    #[test]
    fn unknown_slug_is_an_error_not_a_panic() {
        let err = ProductId::from_slug("not-a-product").unwrap_err();
        assert!(err.is_recoverable(), "a bad slug must be survivable");
    }

    #[test]
    fn mastery_and_learn_slugs_differ_for_the_same_layout() {
        // Both exist for GAIL; conflating them would send a beginner into the
        // Mastery experience.
        assert_ne!(
            ProductId::HindiMastery(LayoutId::RemingtonGail).slug(),
            ProductId::HindiLearn(LayoutId::RemingtonGail).slug()
        );
    }

    #[test]
    fn learn_cards_drop_the_typing_suffix() {
        // PRD.md Section 6.4 wireframe: Learn cards are titled "Remington GAIL",
        // while Mastery cards are "Remington GAIL Typing".
        assert_eq!(
            ProductId::HindiLearn(LayoutId::RemingtonGail).display_name(),
            "Remington GAIL"
        );
        assert_eq!(
            ProductId::HindiMastery(LayoutId::RemingtonGail).display_name(),
            "Remington GAIL Typing"
        );
    }

    #[test]
    fn mastery_and_learn_are_different_products_for_the_same_layout() {
        // Both exist for GAIL, but they are distinct experiences with
        // dedicated UIs — the type system must not conflate them.
        let mastery = ProductId::HindiMastery(LayoutId::RemingtonGail);
        let learn = ProductId::HindiLearn(LayoutId::RemingtonGail);

        assert_ne!(mastery, learn);
        assert_eq!(mastery.layout(), learn.layout());
        assert!(!mastery.is_beginner());
        assert!(learn.is_beginner());
    }
}
