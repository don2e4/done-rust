//! Typed descriptions and retained native-widget reconciliation.
//! Backend integration types are internal framework contracts, not stable APIs.

#![forbid(unsafe_code)]

mod backend;
mod layout;
mod runtime;
mod view;

pub use backend::*;
pub use runtime::{BuildContext, Driver};
pub use view::*;
