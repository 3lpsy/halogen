pub use halogen_handlers as handlers;
pub use halogen_logging as logging;
pub use halogen_runtime_control as restart;
mod cors;
#[cfg(feature = "embed-frontend")]
mod embedded;
mod hosting;
mod public;
pub mod routers {
    pub use crate::hosting::build_router;
    pub use halogen_router::API_PREFIX;
    pub use halogen_routes::*;
}
