//! OAuth support for `git.auth`: cipher, in memory store, encrypted persistence,
//! and the device flow HTTP client. Port of the C# `Git/OAuth/*`.

pub mod cipher;
pub mod device_flow;
pub mod persistence;
pub mod store;

pub use cipher::{decode_key, generate_key_base64};
pub use persistence::SqliteOAuthPersistence;
pub use store::{OAuthSession, OAuthTokenStore, TOKEN_KEY_ENV};
