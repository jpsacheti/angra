pub mod benchmark;
pub mod config;
pub mod lockfile;
pub mod manifest;
pub mod maven;
mod pom;
pub mod resolver;
pub mod settings;

pub use resolver::{ResolveError, ResolveOptions, resolve_project};
