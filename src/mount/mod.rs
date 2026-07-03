pub mod dest;
pub mod origin;
pub mod registry;
pub mod src_remote;

pub use dest::{resolve_destinations, DestMountSession};
pub use origin::{origin_slug_local, origin_slug_remote, SourcedPath};
pub use registry::MountRegistry;
pub use src_remote::{prepare_sources, srcmount_path, PreparedSources, StagingGuard};
