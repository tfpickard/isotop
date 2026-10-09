//! macOS: a stub that compiles and keeps demo mode working. Live mode reports that it needs the
//! collector, and the background sources come back empty.

mod journal;

use std::collections::{HashMap, HashSet};
use std::io;
use std::time::Instant;

use crate::model::{Cpu, Unit};
use crate::platform::{Network, RawProcess};

pub use journal::journal;

/// Reads processes, memory, pressure and CPUs.
pub struct Sampler;

impl Sampler {
    pub fn new() -> Self {
        Self
    }

    /// CPU times will be in nanoseconds.
    pub fn hz(&self) -> f32 {
        1e9
    }

    pub fn processes(&mut self) -> io::Result<Vec<RawProcess>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "live mode on macOS needs the collector from the next release; run isotop --demo",
        ))
    }

    pub fn memory(&self) -> (u64, u64) {
        (0, 0)
    }

    pub fn pressure(&self) -> ([f32; 3], bool) {
        ([0.0; 3], false)
    }

    pub fn cpus(&mut self, _dt: f32) -> Vec<Cpu> {
        Vec::new()
    }
}

/// Socket links between processes; none yet.
pub fn network() -> Network {
    Network::default()
}

/// macOS has no cgroups to account.
pub fn account(
    _paths: &HashSet<String>,
    _previous: &mut HashMap<String, (Instant, u64, u64)>,
) -> HashMap<String, Unit> {
    HashMap::new()
}

/// GPU memory per process; not read on macOS yet.
pub struct Gpu;

impl Gpu {
    pub fn load() -> Option<Self> {
        None
    }

    pub fn sample(&self) -> HashMap<u32, u64> {
        HashMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_mode_refuses_with_an_unsupported_error() {
        let error = Sampler::new().processes().err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("--demo"));
    }
}
