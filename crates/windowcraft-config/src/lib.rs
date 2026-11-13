// Made by WindowCraft Dev Team
//! Typed schemas and embedded catalogs.
//!
//! The catalogs are written for WindowCraft, and every operation in them is a
//! typed [`schema::NativeOp`], never a script. `catalog/winutil-import.json`
//! maps the ids in WinUtil exports so `--config` can read those files. That
//! table is the only WinUtil material here, see `catalog/WINUTIL-LICENSE.txt`.

pub mod catalog;
pub mod localize;
pub mod onboarding;
pub mod profile;
pub mod schema;
pub mod search;
pub mod validate;

pub use catalog::Catalog;
pub use onboarding::{Answers, Area, CleanScope, Depth, Goal, Onboarding, OnboardingError, StartPlan};
pub use profile::{Profile, ProfileError, ResolvedProfile, import_any, parse_json};
pub use schema::*;
pub use search::{SearchHit, SearchKind, search};
