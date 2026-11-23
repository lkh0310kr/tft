//! A Unix-domain socket on every platform. Windows uses `uds_windows`
//! because `std` exposes `AF_UNIX` only under `std::os::unix`; its folder
//! ACL, not mode bits, is the protection.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};
