pub mod config {
    pub struct Config;
}
pub mod adapter {
    pub use crate::config::Config as HiddenConfig;
    pub type WrappedConfig = crate::config::Config;
}
pub mod application {
    #![forbid(clippy::disallowed_types)]

    pub struct Leaks {
        pub direct: Option<crate::config::Config>,
        pub hidden: Option<crate::adapter::HiddenConfig>,
        pub alias: Option<crate::adapter::WrappedConfig>,
    }
}
fn main() {}
