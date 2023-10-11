#![allow(non_upper_case_globals)]

#[cfg(feature = "log")]
#[macro_use]
extern crate log;

#[cfg(not(feature = "log"))]
#[macro_use]
extern crate tracing;

pub mod api;
pub mod transport;
