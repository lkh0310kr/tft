//! The control socket's wire types, method table and blocking client. One
//! JSON request and response per line; no gpui, so tests can drive a window.
//!
//! ```
//! # use tft_control::Request;
//! let req: Request = serde_json::from_str(r#"{"id":1,"method":"state","params":{"rows":5}}"#)
//!     .expect("a request line");
//! assert_eq!(req.opt_usize("rows"), Some(5));
//! // Bad input is an error string, never a panic: it crossed a socket.
//! assert!(req.str_param("rows").is_err());
//! ```

mod client;
mod methods;
pub mod net;
mod protocol;

/// The socket is a Unix domain socket on every platform ([`net`] says how
/// on Windows), and so is the client.
pub use client::Client;
pub use methods::{
    is_instance_method, method, params_schema, protocol_json, MethodSpec, ParamSpec, ParamType,
    INSTANCE_METHODS, METHODS,
};
pub use protocol::{Request, Response};

/// Wire protocol version, reported by `ping` and `protocol`. Bump it when a
/// method's params or result change shape, not when one is added.
/// 2: `state` gained `status_bar` and `window.title`, `state.modal` gained
/// the Settings tab's rows, and `window`'s result gained `title`.
pub const PROTOCOL: u32 = 2;

/// This crate's version, which is the app's: `ping` reports it so a client can
/// tell which build it is driving.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
