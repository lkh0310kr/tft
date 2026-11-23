//! Graph view: notes as nodes, resolved links as edges.

pub mod layout;
mod timelapse;
mod view;

pub use view::{bind_keys, GraphEvent, GraphView};
