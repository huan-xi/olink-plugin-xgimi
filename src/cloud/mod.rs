//! 极米云：签名、会话、HTTP 客户端。

pub mod client;
pub mod session;
pub mod sign;

pub use client::{CloudDevice, LoginResult, XgimiCloudClient};
pub use session::AccountSession;
pub use sign::{build_sign_raw, now_millis, sign_md5, SIGN_SALT};
