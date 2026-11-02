// Made by ycangignacy
//! Typed schemas and embedded catalogs.
//!
//! Catalog data derives from WinUtil's MIT-licensed configuration
//! (see `catalog/WINUTIL-LICENSE.txt`), with every PowerShell block replaced by
//! a typed [`schema::NativeOp`].

pub mod catalog;
pub mod profile;
pub mod schema;
pub mod search;
pub mod validate;

pub use catalog::Catalog;
pub use profile::{Profile, ProfileError, ResolvedProfile, import_any};
pub use schema::*;
pub use search::{SearchHit, SearchKind, search};
