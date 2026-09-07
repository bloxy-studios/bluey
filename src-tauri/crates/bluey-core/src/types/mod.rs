//! Serde data model. Every struct/enum here mirrors a TypeScript type in
//! `src/lib/types/*.ts` **byte-for-byte on the wire** (camelCase fields, the same
//! string tags). When you change one side, change the other.

pub mod ai;
pub mod app_state;
pub mod auth;
pub mod context;
pub mod dev;
pub mod documents;
pub mod mode;
pub mod permissions;
pub mod response;
pub mod session;
pub mod settings;
pub mod transcript;

pub use ai::*;
pub use app_state::*;
pub use auth::*;
pub use context::*;
pub use dev::*;
pub use documents::*;
pub use mode::*;
pub use permissions::*;
pub use response::*;
pub use session::*;
pub use settings::*;
pub use transcript::*;
