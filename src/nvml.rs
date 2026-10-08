//! Per-process NVIDIA GPU memory through NVML, loaded at run time so isotop runs without it.
//! Laptops runtime-suspend an idle discrete GPU; a suspended GPU has no processes, and querying
//! it would wake it, so NVML is only initialised while a device is already powered up.

use std::collections::HashMap;
use std::ffi::{CStr, c_int, c_uint, c_void};
use std::fs;

/// Reported when the driver cannot attribute memory to a process (NVML_VALUE_NOT_AVAILABLE).
const NOT_AVAILABLE: u64 = u64::MAX;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessInfo {
    pid: c_uint,
    used_gpu_memory: u64,
    gpu_instance_id: c_uint,
    compute_instance_id: c_uint,
}

type Status = c_int;
type Processes = unsafe extern "C" fn(*mut c_void, *mut c_uint, *mut ProcessInfo) -> Status;

pub struct Nvml {
    library: *mut c_void,
    init: unsafe extern "C" fn() -> Status,
    shutdown: unsafe extern "C" fn() -> Status,
    count: unsafe extern "C" fn(*mut c_uint) -> Status,
    handle: unsafe extern "C" fn(c_uint, *mut *mut c_void) -> Status,
    queries: [Processes; 2],
}

impl Nvml {
    pub fn load() -> Option<Self> {
        // SAFETY: dlopen receives a NUL-terminated name; a null result is handled.
        let library = unsafe { libc::dlopen(c"libnvidia-ml.so.1".as_ptr(), libc::RTLD_NOW) };
        if library.is_null() {
            return None;
        }
        let symbol = |name: &CStr| {
            // SAFETY: library is a live dlopen handle and name is NUL-terminated.
            let address = unsafe { libc::dlsym(library, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        };
        let bound = (|| {
            // SAFETY: the addresses are NVML entry points with exactly these C signatures.
            unsafe {
                Some(Self {
                    library,
                    init: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> Status>(
                        symbol(c"nvmlInit_v2")?,
                    ),
                    shutdown: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> Status>(
                        symbol(c"nvmlShutdown")?,
                    ),
                    count: std::mem::transmute::<
                        *mut c_void,
                        unsafe extern "C" fn(*mut c_uint) -> Status,
                    >(symbol(c"nvmlDeviceGetCount_v2")?),
                    handle: std::mem::transmute::<
                        *mut c_void,
                        unsafe extern "C" fn(c_uint, *mut *mut c_void) -> Status,
                    >(symbol(c"nvmlDeviceGetHandleByIndex_v2")?),
                    queries: [
                        std::mem::transmute::<*mut c_void, Processes>(symbol(
                            c"nvmlDeviceGetGraphicsRunningProcesses_v3",
                        )?),
                        std::mem::transmute::<*mut c_void, Processes>(symbol(
                            c"nvmlDeviceGetComputeRunningProcesses_v3",
                        )?),
                    ],
                })
            }
        })();
        if bound.is_none() {
            // SAFETY: the handle came from dlopen above and is not used afterwards.
            unsafe { libc::dlclose(library) };
        }
        bound
    }

    /// GPU memory per pid; 1 byte marks a process whose usage the driver does not report.
    pub fn sample(&self) -> HashMap<u32, u64> {
        let mut usage = HashMap::new();
        if !awake() {
            return usage;
        }
        // SAFETY: init/shutdown bracket every other NVML call; device handles never outlive them,
        // and each query receives a buffer with room for `count` entries.
        unsafe {
            if (self.init)() != 0 {
                return usage;
            }
            let mut total = 0;
            if (self.count)(&mut total) == 0 {
                for index in 0..total {
                    let mut device = std::ptr::null_mut();
                    if (self.handle)(index, &mut device) != 0 {
                        continue;
                    }
                    for query in self.queries {
                        let mut infos = vec![ProcessInfo::default(); 512];
                        let mut count = infos.len() as c_uint;
                        if query(device, &mut count, infos.as_mut_ptr()) != 0 {
                            continue;
                        }
                        for info in &infos[..(count as usize).min(infos.len())] {
                            let bytes = match info.used_gpu_memory {
                                NOT_AVAILABLE => 1,
                                bytes => bytes.max(1),
                            };
                            *usage.entry(info.pid).or_default() += bytes;
                        }
                    }
                }
            }
            (self.shutdown)();
        }
        usage
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: the handle came from dlopen, and no NVML session stays open between samples.
        unsafe { libc::dlclose(self.library) };
    }
}

/// True when some NVIDIA display controller is not runtime-suspended.
fn awake() -> bool {
    let Ok(devices) = fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };
    devices.flatten().any(|device| {
        let path = device.path();
        let read = |name: &str| fs::read_to_string(path.join(name)).unwrap_or_default();
        read("vendor").trim() == "0x10de"
            && read("class").trim().starts_with("0x03")
            && read("power/runtime_status").trim() != "suspended"
    })
}
