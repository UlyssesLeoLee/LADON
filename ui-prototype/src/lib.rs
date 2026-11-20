//! Library root of the LADON Explorer prototype.
//!
//! The `model` module is available in every feature set. The `app` module
//! requires the `kernel` feature.

pub mod model;

#[cfg(feature = "kernel")]
pub mod app;
