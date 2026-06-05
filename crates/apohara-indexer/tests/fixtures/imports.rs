// Basic use statements
use std::collections::HashMap;
use std::fmt::Debug;
use crate::module::Struct;
use self::inner::function;

// Use with alias
use std::io::{Read, Write};

// Glob use
use serde::*;

// Re-export (pub use)
pub use crate::reexport::Item;

// External module declaration
mod external_module;

// Inline module
mod inline_module {
    pub fn inner_function() {}
}

// pub mod
pub mod public_module {
    pub struct PublicStruct;
}