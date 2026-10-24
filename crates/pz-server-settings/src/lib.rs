//! `zombie.network.ServerOptions`, `ServerSettings`,
//! `ServerSettingsManager` and `SpawnRegions`: the server configuration the
//! host-a-server screens edit, with the local (no server running)
//! behaviour of the original.
//!
//! Option definitions and other code-defined lists are read from the
//! installed jar at runtime, never shipped.

pub mod options;
pub mod settings;
pub mod spawn_regions;

pub use options::{ServerOption, ServerOptionKind, ServerOptions};
pub use settings::{file_name_sanitize, ServerSettings, ServerSettingsManager};
pub use spawn_regions::{Point, Profession, Region};

/// `System.lineSeparator()`.
pub(crate) fn line_separator() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}
