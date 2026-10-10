//! macOS: processes from libproc and `KERN_PROCARGS2`, CPU time in Mach absolute time, CPUs and
//! memory from Mach host statistics, P/E cores from the IORegistry, the memory pressure level,
//! sockets from `proc_pidfdinfo`, and the unified log. Other users' processes are counted but
//! not measured unless isotop runs as root. macOS has no stall accounting, last CPU, clock
//! readings, run queues, cgroups or per-socket byte counters; `Sampler::missing` names them.

mod ffi;
mod journal;
mod logic;

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Instant;

use crate::model::{CoreKind, Cpu, Identity, IoBytes, Kind, Process, Unit};
use crate::platform::{Network, RawProcess};

pub use journal::{JOURNAL, journal};

/// What `Process::memory` measures here: the physical footprint, which counts compressed and
/// swapped dirty memory and leaves out clean shared pages, so it is not a resident size.
pub const MEMORY_LABEL: &str = "memory footprint";

/// `ARG_MAX` on macOS, used when `kern.argmax` cannot be read.
const ARG_MAX: usize = 1 << 20;

/// Reads processes, memory, pressure and CPUs, keeping the CPU counters between readings.
pub struct Sampler {
    /// The host port, taken once: every `mach_host_self` call adds a port reference.
    host: libc::mach_port_t,
    timebase: ffi::MachTimebaseInfo,
    page_size: u64,
    memory_total: u64,
    /// Whose processes count as the session.
    my_uid: u32,
    responsible: Option<ffi::Responsible>,
    core_kinds: HashMap<u32, CoreKind>,
    cpu_previous: Vec<[u32; 4]>,
    /// Buffers reused by every `processes` call.
    pids: Vec<c_int>,
    arguments: Vec<u8>,
    path: Vec<u8>,
    /// Path and command of each process seen in the previous call.
    described: HashMap<Identity, Description>,
    unreadable: usize,
}

/// What a process runs, which stays the same until it calls exec.
struct Description {
    /// The name it was read under; a different name means the process has exec'd since.
    name: String,
    path: String,
    command: String,
}

/// Why a process is absent from a listing.
enum Absent {
    /// Another user's process, which only root may measure.
    Unreadable,
    /// It exited, or the kernel refused for another reason.
    Gone,
}

impl Sampler {
    pub fn new() -> Self {
        let mut timebase = ffi::MachTimebaseInfo::default();
        // SAFETY: mach_timebase_info writes one struct mach_timebase_info to the pointer.
        let status = unsafe { ffi::mach_timebase_info(&mut timebase) };
        if status != libc::KERN_SUCCESS || timebase.numer == 0 || timebase.denom == 0 {
            timebase = ffi::MachTimebaseInfo { numer: 1, denom: 1 };
        }
        // SAFETY: getuid cannot fail and has no preconditions.
        let uid = unsafe { libc::getuid() };
        // Under sudo the person's own processes are still the session.
        let my_uid = std::env::var("SUDO_UID")
            .ok()
            .and_then(|text| text.parse().ok())
            .filter(|_| uid == 0)
            .unwrap_or(uid);
        let argmax = sysctl::<c_int>(c"kern.argmax")
            .and_then(|value| usize::try_from(value).ok())
            .filter(|&value| value > 0)
            .unwrap_or(ARG_MAX);
        Self {
            // SAFETY: mach_host_self has no preconditions and returns a port name.
            host: unsafe { ffi::mach_host_self() },
            timebase,
            // SAFETY: sysconf has no preconditions; _SC_PAGESIZE is positive on macOS.
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
            memory_total: sysctl::<u64>(c"hw.memsize").unwrap_or(0),
            my_uid,
            responsible: ffi::responsible(),
            core_kinds: core_kinds(),
            cpu_previous: Vec::new(),
            pids: Vec::new(),
            arguments: vec![0; argmax],
            path: vec![0; libc::PROC_PIDPATHINFO_MAXSIZE as usize],
            described: HashMap::new(),
            unreadable: 0,
        }
    }

    /// CPU times are in nanoseconds.
    pub fn hz(&self) -> f32 {
        1e9
    }

    /// Every process this user may measure (every process for root). Other users' processes
    /// are counted in `unreadable` instead; their CPU and memory are never guessed.
    pub fn processes(&mut self) -> io::Result<Vec<RawProcess>> {
        list_pids(&mut self.pids)?;
        self.unreadable = 0;
        let mut previous = std::mem::take(&mut self.described);
        let mut found = Vec::with_capacity(self.pids.len());
        let mut lineages = Vec::with_capacity(self.pids.len());
        for index in 0..self.pids.len() {
            match self.process(self.pids[index], &mut previous) {
                Ok((raw, lineage)) => {
                    found.push(raw);
                    lineages.push(lineage);
                }
                Err(Absent::Unreadable) => self.unreadable += 1,
                Err(Absent::Gone) => {}
            }
        }
        for (raw, parent) in found.iter_mut().zip(logic::parents(&lineages)) {
            raw.process.parent = parent;
        }
        Ok(found)
    }

    /// One process; `previous` holds the descriptions of the last call, and this call's are
    /// left in `self.described`.
    fn process(
        &mut self,
        pid: c_int,
        previous: &mut HashMap<Identity, Description>,
    ) -> Result<(RawProcess, logic::Lineage), Absent> {
        let bsd: libc::proc_bsdinfo = pid_info(pid, libc::PROC_PIDTBSDINFO)?;
        let task: libc::proc_taskinfo = pid_info(pid, libc::PROC_PIDTASKINFO)?;
        let id = Identity {
            pid: pid as u32,
            start: bsd
                .pbi_start_tvsec
                .saturating_mul(1_000_000)
                .saturating_add(bsd.pbi_start_tvusec),
        };
        let name = match c_text(&bsd.pbi_name) {
            name if name.is_empty() => c_text(&bsd.pbi_comm),
            name => name,
        };
        let description = match previous.remove(&id).filter(|d| d.name == name) {
            Some(description) => description,
            None => self.describe(pid, name),
        };
        let usage = rusage(pid);
        let memory = usage.map_or(task.pti_resident_size, |usage| usage.ri_phys_footprint);
        let io = usage.map(|usage| IoBytes {
            read: usage.ri_diskio_bytesread,
            write: usage.ri_diskio_byteswritten,
        });
        let kind = logic::classify(
            id.pid,
            bsd.pbi_uid,
            self.my_uid,
            &description.path,
            &description.name,
        );
        let group = logic::group(&description.path, &description.name);
        let process = Process {
            id,
            parent: bsd.pbi_ppid,
            name: description.name.clone(),
            command: description.command.clone(),
            cgroup: if kind == Kind::Kernel {
                String::new()
            } else {
                logic::cgroup_path(kind, &group)
            },
            group,
            kind,
            state: logic::state(bsd.pbi_status, task.pti_numrunning),
            cpu: 0.0,
            memory,
            io_rate: None,
            read_rate: None,
            write_rate: None,
            written: io.map(|io| io.write),
            priority: logic::priority_from_mach(task.pti_priority),
            nice: bsd.pbi_nice,
            threads: task.pti_threadnum.max(0) as u32,
            gpu_memory: 0,
            core: 0,
            cpu_time: 0.0,
        };
        let responsible = self.responsible.and_then(|responsible| {
            // SAFETY: the SPI takes any pid and returns -1 when it has no answer.
            let app = unsafe { responsible(pid) };
            u32::try_from(app).ok()
        });
        self.described.insert(id, description);
        let ticks = logic::nanoseconds(
            task.pti_total_user.saturating_add(task.pti_total_system),
            self.timebase.numer,
            self.timebase.denom,
        );
        Ok((
            RawProcess { process, ticks, io },
            logic::Lineage {
                pid: id.pid,
                ppid: bsd.pbi_ppid,
                responsible,
            },
        ))
    }

    /// The executable path and command line. Both can be unreadable, for example for
    /// processes that SIP protects; the command then falls back to the name.
    fn describe(&mut self, pid: c_int, name: String) -> Description {
        // SAFETY: the buffer is valid for writes of its full length, which lies between
        // PROC_PIDPATHINFO_SIZE and PROC_PIDPATHINFO_MAXSIZE as proc_pidpath requires.
        let length = unsafe {
            libc::proc_pidpath(pid, self.path.as_mut_ptr().cast(), self.path.len() as u32)
        };
        let mut path = usize::try_from(length)
            .ok()
            .and_then(|length| self.path.get(..length))
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut size = self.arguments.len();
        // SAFETY: the name has three elements as passed, the buffer is valid for writes of
        // `size` bytes, and nothing is written to the kernel.
        let status = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                self.arguments.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        let mut command = String::new();
        if status == 0
            && let Some((executable, argv)) =
                logic::parse_procargs(&self.arguments[..size.min(self.arguments.len())])
        {
            if path.is_empty() {
                path = executable;
            }
            command = argv.join(" ").trim().to_owned();
        }
        if command.is_empty() {
            command = name.clone();
        }
        Description {
            name,
            path,
            command,
        }
    }

    /// Total memory, and the bytes the kernel can hand out without paging out application
    /// memory (free and inactive pages); 0 when unreadable.
    pub fn memory(&self) -> (u64, u64) {
        let mut statistics = ffi::VmStatistics64::default();
        let mut count = ffi::HOST_VM_INFO64_COUNT;
        // SAFETY: the buffer holds `count` integer_t values, which is what the kernel may
        // write for this count, and it writes back the count it filled.
        let status = unsafe {
            libc::host_statistics64(
                self.host,
                libc::HOST_VM_INFO64,
                (&mut statistics as *mut ffi::VmStatistics64).cast(),
                &mut count,
            )
        };
        let available = if status == libc::KERN_SUCCESS {
            logic::available(
                statistics.free_count,
                statistics.inactive_count,
                self.page_size,
            )
            .min(self.memory_total)
        } else {
            0
        };
        (self.memory_total, available)
    }

    /// The memory pressure level in the memory slot, and whether the level could be read.
    /// macOS has no CPU or I/O stall accounting.
    pub fn pressure(&self) -> ([f32; 3], bool) {
        match sysctl::<c_int>(c"kern.memorystatus_vm_pressure_level") {
            Some(level) => ([0.0, logic::pressure_from_level(level), 0.0], true),
            None => ([0.0; 3], false),
        }
    }

    /// Per-CPU use since the previous call. macOS reports no clock or run queue per CPU.
    pub fn cpus(&mut self, _dt: f32) -> Vec<Cpu> {
        let ticks = cpu_ticks(self.host);
        let cpus = ticks
            .iter()
            .enumerate()
            .map(|(index, &now)| {
                let id = index as u32;
                Cpu {
                    id,
                    kind: self
                        .core_kinds
                        .get(&id)
                        .copied()
                        .unwrap_or(CoreKind::Unknown),
                    busy: self
                        .cpu_previous
                        .get(index)
                        .map_or(0.0, |&before| logic::busy(before, now)),
                    mhz: 0.0,
                    wait: 0.0,
                }
            })
            .collect();
        self.cpu_previous = ticks;
        cpus
    }

    /// Sources macOS does not have at all, by the names `Snapshot::missing` uses.
    pub fn missing(&self) -> Vec<&'static str> {
        vec![
            "cpu pressure",
            "io pressure",
            "last cpu",
            "cpu clock",
            "run queue",
            "cgroups",
            "socket traffic",
        ]
    }

    /// Processes the last `processes` call listed but could not measure, because they belong
    /// to another user.
    pub fn unreadable(&self) -> usize {
        self.unreadable
    }
}

/// Fills `pids` with every pid on the system, kernel_task (0) included.
fn list_pids(pids: &mut Vec<c_int>) -> io::Result<()> {
    // SAFETY: a null buffer asks only for the number of pids.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    pids.resize(count as usize + 64, 0);
    loop {
        let bytes = (pids.len() * size_of::<c_int>()) as c_int;
        // SAFETY: the buffer is valid for writes of `bytes` bytes.
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        // A full buffer may have cut the list short.
        if (count as usize) < pids.len() {
            pids.truncate(count as usize);
            return Ok(());
        }
        let grown = pids.len() * 2;
        pids.resize(grown, 0);
    }
}

/// One `proc_pidinfo` flavor of a process.
fn pid_info<T: Copy>(pid: c_int, flavor: c_int) -> Result<T, Absent> {
    let mut value = MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as c_int;
    // SAFETY: the buffer is valid for writes of `size` bytes.
    let written = unsafe { libc::proc_pidinfo(pid, flavor, 0, value.as_mut_ptr().cast(), size) };
    if written == size {
        // SAFETY: T is one of libc's plain-integer proc_info structs, which every bit pattern
        // inhabits, and the kernel filled all of it.
        Ok(unsafe { value.assume_init() })
    } else if written <= 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) {
        Err(Absent::Unreadable)
    } else {
        Err(Absent::Gone)
    }
}

/// Resource usage with the physical footprint and disk I/O; None when refused.
fn rusage(pid: c_int) -> Option<libc::rusage_info_v2> {
    let mut usage = MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: for RUSAGE_INFO_V2 the kernel writes one rusage_info_v2 to the buffer; the
    // parameter is declared as rusage_info_t * but takes the struct's address.
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    // SAFETY: the struct holds only integers, so the zeroed or filled bytes are valid.
    (status == 0).then(|| unsafe { usage.assume_init() })
}

/// A sysctl value of exactly the size of T; macOS refuses a buffer of the wrong size.
fn sysctl<T: Copy + Default>(name: &CStr) -> Option<T> {
    let mut value = T::default();
    let mut size = size_of::<T>();
    // SAFETY: the name is NUL-terminated, the buffer is valid for `size` bytes, and nothing is
    // written to the kernel.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut T).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && size == size_of::<T>()).then_some(value)
}

/// Text from a fixed-size C char array, up to its first NUL.
fn c_text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Cumulative `[user, system, idle, nice]` scheduler ticks of each CPU, by CPU number.
fn cpu_ticks(host: libc::mach_port_t) -> Vec<[u32; 4]> {
    let mut count: libc::natural_t = 0;
    let mut info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut length: libc::mach_msg_type_number_t = 0;
    // SAFETY: the three out-pointers are valid; on success the kernel maps an array of
    // `length` integer_t values into this task and stores its address in `info`.
    let status = unsafe {
        libc::host_processor_info(
            host,
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut count,
            &mut info,
            &mut length,
        )
    };
    if status != libc::KERN_SUCCESS || info.is_null() {
        return Vec::new();
    }
    // SAFETY: the kernel returned `length` initialized integer_t values at `info`, which stay
    // mapped until the vm_deallocate below.
    let values = unsafe { std::slice::from_raw_parts(info, length as usize) };
    let stride = libc::CPU_STATE_MAX as usize;
    let ticks = values
        .chunks_exact(stride)
        .take(count as usize)
        .map(|cpu| {
            [
                libc::CPU_STATE_USER,
                libc::CPU_STATE_SYSTEM,
                libc::CPU_STATE_IDLE,
                libc::CPU_STATE_NICE,
            ]
            .map(|state| cpu[state as usize] as u32)
        })
        .collect();
    // SAFETY: `info` is the out-of-line array host_processor_info mapped into this task, of
    // `length` integer_t values, and nothing refers to it after this.
    unsafe {
        libc::vm_deallocate(
            ffi::mach_task_self(),
            info as libc::vm_address_t,
            length as usize * size_of::<libc::integer_t>(),
        );
    }
    ticks
}

/// An IOKit object, released when dropped.
struct IoObject(ffi::io_object_t);

impl Drop for IoObject {
    fn drop(&mut self) {
        // SAFETY: the object came from IOKit with a reference this wrapper owns.
        unsafe { ffi::IOObjectRelease(self.0) };
    }
}

/// A retained CoreFoundation object, released when dropped.
struct CfObject(ffi::CFTypeRef);

impl CfObject {
    fn string(text: &CStr) -> Option<Self> {
        // SAFETY: the text is NUL-terminated UTF-8 and a null allocator means the default.
        let string = unsafe {
            ffi::CFStringCreateWithCString(
                std::ptr::null(),
                text.as_ptr(),
                ffi::kCFStringEncodingUTF8,
            )
        };
        // Wrapped lazily: a CfObject releases what it holds, and CFRelease traps on NULL.
        (!string.is_null()).then(|| Self(string))
    }

    /// A number property: a CFNumber, or the little-endian bytes of a device-tree CFData.
    fn number(&self) -> Option<u64> {
        // SAFETY: self.0 is a valid CF object, and CFGetTypeID and the type id functions
        // only read it.
        let (kind, number) = unsafe { (ffi::CFGetTypeID(self.0), ffi::CFNumberGetTypeID()) };
        if kind != number {
            return logic::little_endian(self.bytes()?);
        }
        let mut value: i64 = 0;
        // SAFETY: self.0 is a CFNumber and the destination holds one SInt64.
        let exact = unsafe {
            ffi::CFNumberGetValue(
                self.0,
                ffi::kCFNumberSInt64Type,
                (&mut value as *mut i64).cast::<c_void>(),
            )
        };
        if exact == 0 {
            return None;
        }
        u64::try_from(value).ok()
    }

    /// The bytes of a CFData, borrowed for as long as the object lives.
    fn bytes(&self) -> Option<&[u8]> {
        // SAFETY: self.0 is a valid CF object; the type is checked before the CFData calls,
        // whose pointer stays valid and unchanged while the immutable object is retained.
        unsafe {
            if ffi::CFGetTypeID(self.0) != ffi::CFDataGetTypeID() {
                return None;
            }
            let length = usize::try_from(ffi::CFDataGetLength(self.0)).ok()?;
            let pointer = ffi::CFDataGetBytePtr(self.0);
            if pointer.is_null() {
                return (length == 0).then_some(&[]);
            }
            Some(std::slice::from_raw_parts(pointer, length))
        }
    }
}

impl Drop for CfObject {
    fn drop(&mut self) {
        // SAFETY: the object was created or copied (retained) for this wrapper.
        unsafe { ffi::CFRelease(self.0) };
    }
}

/// Performance or efficiency for each logical CPU, from the `cluster-type` and
/// `logical-cpu-id` properties of the IODeviceTree:/cpus children. Any CPU these do not cover
/// stays Unknown; the order of the ids is not assumed.
fn core_kinds() -> HashMap<u32, CoreKind> {
    let mut kinds = HashMap::new();
    let (Some(id_key), Some(type_key)) = (
        CfObject::string(c"logical-cpu-id"),
        CfObject::string(c"cluster-type"),
    ) else {
        return kinds;
    };
    // SAFETY: the path is NUL-terminated; a 0 result means no such entry.
    let cpus = unsafe {
        ffi::IORegistryEntryFromPath(ffi::kIOMainPortDefault, c"IODeviceTree:/cpus".as_ptr())
    };
    if cpus == 0 {
        return kinds;
    }
    let cpus = IoObject(cpus);
    let mut iterator = 0;
    // SAFETY: the entry is valid, the plane name is NUL-terminated, and the iterator is
    // written only on success.
    let status = unsafe {
        ffi::IORegistryEntryGetChildIterator(cpus.0, c"IODeviceTree".as_ptr(), &mut iterator)
    };
    if status != libc::KERN_SUCCESS || iterator == 0 {
        return kinds;
    }
    let iterator = IoObject(iterator);
    let property = |entry: &IoObject, key: &CfObject| {
        // SAFETY: the entry and key are valid; options 0 reads this entry only, and the result
        // is retained or null.
        let value = unsafe {
            ffi::IORegistryEntrySearchCFProperty(
                entry.0,
                c"IODeviceTree".as_ptr(),
                key.0,
                std::ptr::null(),
                0,
            )
        };
        // Wrapped lazily, as in CfObject::string: a missing property is NULL.
        (!value.is_null()).then(|| CfObject(value))
    };
    loop {
        // SAFETY: the iterator is valid; it returns retained children and 0 at the end.
        let child = unsafe { ffi::IOIteratorNext(iterator.0) };
        if child == 0 {
            break;
        }
        let child = IoObject(child);
        let id = property(&child, &id_key)
            .and_then(|value| value.number())
            .and_then(|id| u32::try_from(id).ok());
        let kind = property(&child, &type_key)
            .and_then(|value| value.bytes()?.first().copied())
            .and_then(logic::core_kind);
        if let (Some(id), Some(kind)) = (id, kind) {
            kinds.insert(id, kind);
        }
    }
    kinds
}

/// Socket links between processes, from the sockets of every process this user may read.
pub fn network() -> Network {
    let mut pids = Vec::new();
    if list_pids(&mut pids).is_err() {
        return Network::default();
    }
    let mut sockets = Sockets::default();
    let mut descriptors = Vec::new();
    for &pid in &pids {
        sockets.scan(pid, &mut descriptors);
    }
    logic::network(&sockets.unix, &sockets.tcp, &own_addresses())
}

/// The TCP and Unix sockets of the scanned processes.
#[derive(Default)]
struct Sockets {
    tcp: Vec<logic::TcpSocket>,
    unix: Vec<logic::UnixSocket>,
}

impl Sockets {
    /// Adds the sockets of one process; `descriptors` is a buffer reused between processes.
    fn scan(&mut self, pid: c_int, descriptors: &mut Vec<libc::proc_fdinfo>) {
        let empty = libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        };
        if descriptors.is_empty() {
            descriptors.resize(256, empty);
        }
        let count = loop {
            let bytes = descriptors.len() * size_of::<libc::proc_fdinfo>();
            // SAFETY: the buffer is valid for writes of `bytes` bytes.
            let written = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDLISTFDS,
                    0,
                    descriptors.as_mut_ptr().cast(),
                    bytes as c_int,
                )
            };
            let Ok(written) = usize::try_from(written) else {
                return;
            };
            if written == 0 {
                return;
            }
            // The kernel fills at most the whole buffer and says nothing when it truncates.
            if written < bytes {
                break written / size_of::<libc::proc_fdinfo>();
            }
            let grown = descriptors.len() * 2;
            descriptors.resize(grown, empty);
        };
        for descriptor in &descriptors[..count] {
            if descriptor.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32
                && let Some(info) = socket_info(pid, descriptor.proc_fd)
            {
                self.add(pid as u32, &info.psi);
            }
        }
    }

    fn add(&mut self, pid: u32, socket: &ffi::socket_info) {
        match socket.soi_kind {
            ffi::SOCKINFO_TCP => {
                // SAFETY: soi_kind says the kernel filled the TCP member of the union, and its
                // addresses are plain bytes that any value inhabits.
                let (tcp, local, foreign) = unsafe {
                    let tcp = socket.soi_proto.pri_tcp;
                    let ini = tcp.tcpsi_ini;
                    (
                        tcp,
                        ini.insi_laddr.ina_6.u6_addr8,
                        ini.insi_faddr.ina_6.u6_addr8,
                    )
                };
                let ini = tcp.tcpsi_ini;
                self.tcp.push(logic::TcpSocket {
                    pid,
                    id: socket.soi_so,
                    local: SocketAddr::new(
                        logic::address(ini.insi_vflag, local),
                        logic::port(ini.insi_lport),
                    ),
                    foreign: SocketAddr::new(
                        logic::address(ini.insi_vflag, foreign),
                        logic::port(ini.insi_fport),
                    ),
                    state: tcp.tcpsi_state,
                });
            }
            ffi::SOCKINFO_UN => {
                // SAFETY: soi_kind says the kernel filled the Unix member of the union.
                let peer = unsafe { socket.soi_proto.pri_un.unsi_conn_pcb };
                self.unix.push(logic::UnixSocket {
                    pid,
                    pcb: socket.soi_pcb,
                    peer,
                });
            }
            _ => {}
        }
    }
}

/// A socket's `socket_fdinfo`, only when the kernel wrote exactly the size of ours, which
/// proves the layouts agree.
fn socket_info(pid: c_int, fd: i32) -> Option<ffi::socket_fdinfo> {
    let mut info = MaybeUninit::<ffi::socket_fdinfo>::zeroed();
    let size = size_of::<ffi::socket_fdinfo>() as c_int;
    // SAFETY: the buffer is valid for writes of `size` bytes.
    let written = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            ffi::PROC_PIDFDSOCKETINFO,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    // SAFETY: socket_fdinfo holds only integers and unions of integers, so the zeroed or
    // filled bytes are valid.
    (written == size).then(|| unsafe { info.assume_init() })
}

/// The addresses of this machine's interfaces.
fn own_addresses() -> HashSet<IpAddr> {
    let mut addresses = HashSet::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs writes the head of a list that freeifaddrs releases below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return addresses;
    }
    let mut entry = list;
    while !entry.is_null() {
        // SAFETY: entries of the list stay valid until freeifaddrs; each address points to a
        // sockaddr whose family says which larger struct it is, read unaligned.
        unsafe {
            let address = (*entry).ifa_addr;
            if !address.is_null() {
                match c_int::from((*address).sa_family) {
                    libc::AF_INET => {
                        let ip = std::ptr::read_unaligned(address.cast::<libc::sockaddr_in>());
                        addresses
                            .insert(IpAddr::V4(Ipv4Addr::from(ip.sin_addr.s_addr.to_ne_bytes())));
                    }
                    libc::AF_INET6 => {
                        let ip = std::ptr::read_unaligned(address.cast::<libc::sockaddr_in6>());
                        addresses.insert(IpAddr::V6(Ipv6Addr::from(ip.sin6_addr.s6_addr)));
                    }
                    _ => {}
                }
            }
            entry = (*entry).ifa_next;
        }
    }
    // SAFETY: the list came from getifaddrs and is not used afterwards.
    unsafe { libc::freeifaddrs(list) };
    addresses
}

/// macOS has no cgroups to account.
pub fn account(
    _paths: &HashSet<String>,
    _previous: &mut HashMap<String, (Instant, u64, u64)>,
) -> HashMap<String, Unit> {
    HashMap::new()
}

/// GPU memory per process. Apple GPUs share memory with the CPU and report no per-process
/// figure here, so there is none.
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn the_collector_finds_this_process_with_its_memory_cpu_time_and_start() {
        let mut sampler = Sampler::new();
        let deadline = Instant::now() + Duration::from_millis(200);
        let mut spin = 0_u64;
        while Instant::now() < deadline {
            spin = std::hint::black_box(spin.wrapping_mul(31).wrapping_add(7));
        }
        let processes = sampler.processes().unwrap();
        let me = std::process::id();
        let raw = processes
            .iter()
            .find(|raw| raw.process.id.pid == me)
            .expect("this process is listed");
        let process = &raw.process;
        assert!(process.memory > 1 << 20, "memory {}", process.memory);
        assert!(raw.ticks > 0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        assert!(
            process.id.start <= now + 1_000_000,
            "start {}",
            process.id.start
        );
        assert!(
            now - process.id.start.min(now) < 3_600_000_000,
            "start {}",
            process.id.start
        );
        let binary = std::env::current_exe().unwrap();
        let binary = binary.file_name().unwrap().to_string_lossy();
        assert!(
            process.name.contains(binary.as_ref()),
            "{} does not name {binary}",
            process.name
        );
        assert!(process.threads >= 1);
        assert!(raw.io.is_some());
        assert_eq!(process.kind, Kind::Session);
        assert_eq!(process.cgroup, format!("/session/{binary}"));
        assert!(
            processes
                .iter()
                .all(|raw| raw.process.id.pid != 0 || raw.process.kind == Kind::Kernel)
        );
        assert_eq!(sampler.missing().len(), 7);
        assert!(processes.len() + sampler.unreadable() <= sampler.pids.len());
    }

    #[test]
    fn socket_info_has_the_size_the_kernel_reports() {
        let (mut left, right) = UnixStream::pair().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        client.write_all(b"x").unwrap();
        let mut byte = [0];
        server.read_exact(&mut byte).unwrap();
        left.write_all(b"y").unwrap();

        let me = std::process::id();
        let pid = me as c_int;
        assert!(
            socket_info(pid, left.as_raw_fd()).is_some(),
            "proc_pidfdinfo size differs"
        );
        let mut sockets = Sockets::default();
        sockets.scan(pid, &mut Vec::new());

        let unix = |fd: i32| {
            let info = socket_info(pid, fd).unwrap();
            sockets
                .unix
                .iter()
                .copied()
                .find(|socket| socket.pcb == info.psi.soi_pcb)
                .expect("the Unix socket was scanned")
        };
        let (a, b) = (unix(left.as_raw_fd()), unix(right.as_raw_fd()));
        assert_eq!((a.peer, b.peer), (b.pcb, a.pcb));
        assert_eq!(logic::pair_unix(&[a, b]), vec![(me, me)]);

        let end = |local: SocketAddr, foreign: SocketAddr| {
            sockets
                .tcp
                .iter()
                .copied()
                .find(|socket| socket.local == local && socket.foreign == foreign)
                .expect("the TCP socket was scanned")
        };
        let outgoing = end(client.local_addr().unwrap(), client.peer_addr().unwrap());
        let incoming = end(server.local_addr().unwrap(), server.peer_addr().unwrap());
        assert_eq!(
            (outgoing.state, incoming.state),
            (logic::ESTABLISHED, logic::ESTABLISHED)
        );
        let (pairs, remotes) = logic::pair_tcp(&[outgoing, incoming], &HashSet::new());
        assert_eq!(pairs, vec![(me, me)]);
        assert!(remotes.is_empty());
        assert!(!network().outside.contains_key(&me));
    }

    #[test]
    fn cpus_report_busy_shares_between_zero_and_one() {
        let mut sampler = Sampler::new();
        assert!(sampler.cpus(0.1).iter().all(|cpu| cpu.busy == 0.0));
        std::thread::sleep(Duration::from_millis(100));
        let cpus = sampler.cpus(0.1);
        assert!(!cpus.is_empty());
        assert!(cpus.iter().all(|cpu| (0.0..=1.0).contains(&cpu.busy)));
        assert!(cpus.windows(2).all(|pair| pair[0].id < pair[1].id));
        let (total, available) = sampler.memory();
        assert!(total > 0);
        assert!(
            available > 0 && available <= total,
            "{available} of {total}"
        );
    }
}
