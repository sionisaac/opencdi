// Lets the state macros name `opencdi::` both here and in bins that include these files by path.
extern crate self as opencdi;

pub mod audio;
pub mod bios;
pub mod bios_audit;
pub mod boot;
pub mod bus;
pub mod cdic;
pub mod cdrom;
pub mod dma;
pub mod dvc;
#[cfg(feature = "host")]
pub mod frontend;
#[cfg(all(windows, feature = "host"))]
pub mod gui;
pub mod input_script;
pub mod host_input;
pub mod machine;
pub mod mcd212;
pub mod mono4;
pub mod nvram;
pub mod rtc;
pub mod os9_trace;
pub mod savestate;
pub mod scc_timer;
pub mod slave;
pub mod tap;
#[cfg(test)]
mod spec_tests;

pub use m68k::{CpuCore, CpuType, AddressBus};
