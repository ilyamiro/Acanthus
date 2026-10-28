pub(crate) use std::collections::HashMap as Map;

pub(crate) type IndexMap<K, V> = indexmap::IndexMap<K, V>;

pub mod color;
pub mod contrast;
pub mod dislike;
pub mod dynamic_color;
pub mod error;
pub mod generate;
pub mod hct;
pub mod palette;
pub mod quantize;
pub mod scheme;
pub mod score;
pub mod temperature;
pub mod utils;

pub use error::Error;
