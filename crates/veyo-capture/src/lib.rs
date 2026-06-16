pub mod backend;
pub mod downscale;
pub mod poll;

pub use backend::{CaptureBackend, CapturedFrame, SurfaceInfo};
pub use downscale::rgba_to_cells;
pub use poll::PollBackend;
