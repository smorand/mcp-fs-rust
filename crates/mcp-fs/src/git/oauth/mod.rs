//! OAuth token storage for `git.auth`: cipher, in memory store, encrypted
//! persistence. Port of the C# `Git/OAuth/*` minus the device flow itself, which
//! belongs to the tools layer (it makes outbound HTTP calls, this module does not).

pub mod cipher;
pub mod persistence;
pub mod store;

pub use cipher::{decode_key, generate_key_base64};
pub use persistence::SqliteOAuthPersistence;
pub use store::{OAuthSession, OAuthTokenStore, TOKEN_KEY_ENV};
