//! One file per page (D§12.5): each page registers its own `#[page]`
//! route and re-exports from here so `lib.rs` stays a flat list.

mod cloud;
mod settings;

pub use cloud::cloud_page;
pub use settings::settings_page;
