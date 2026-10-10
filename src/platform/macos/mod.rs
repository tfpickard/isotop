//! macOS: processes from libproc and `KERN_PROCARGS2`, CPU time in Mach absolute time, CPUs and
//! memory from Mach host statistics, P/E cores from the IORegistry, the memory pressure level,
//! sockets and open files from `proc_pidfdinfo`, and the unified log. Other users' processes are
//! counted but not measured unless isotop runs as root. macOS has no stall accounting, last CPU,
//! per-CPU clock readings, run queues, cgroups, per-socket byte counters or file lock table;
//! `Sampler::missing` names them. What it has instead is per process and per cluster:
//! `proc_pid_rusage` splits CPU time and cycles between the performance and efficiency cores and
//! counts runnable time, which give each process's share of performance-core time, its waiting
//! threads and each cluster's clock.

mod ffi;
mod journal;
mod logic;

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Instant;

use crate::model::{CoreKind, Cpu, Identity, IoBytes, Kind, Measured, Process, Unit};
use crate::platform::{Files, Locks, Network, RawProcess};

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
    /// The newest `proc_pid_rusage` flavor this kernel accepts.
    rusage_version: c_int,
    /// Whether CPU time can be split by cluster: the kernel gives V6 and the IORegistry named
    /// both performance and efficiency cores.
    clusters: bool,
    /// Each process's cluster counters at the previous `processes` call.
    counters: HashMap<Identity, logic::Counters>,
    /// The cluster clocks over the last interval in MHz, as [performance, efficiency].
    clocks: [f32; 2],
    /// Whether the last `processes` call read any cycles; virtual machines count none.
    cycles: bool,
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
    /// The pid version it was read under; exec changes it, even when the name stays.
    version: i32,
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
        let core_kinds = core_kinds();
        let rusage_version = rusage_version();
        let clusters = rusage_version >= ffi::RUSAGE_INFO_V6
            && [CoreKind::Performance, CoreKind::Efficiency]
                .iter()
                .all(|kind| core_kinds.values().any(|found| found == kind));
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
            core_kinds,
            cpu_previous: Vec::new(),
            rusage_version,
            clusters,
            counters: HashMap::new(),
            clocks: [0.0; 2],
            cycles: false,
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
        let counted = std::mem::take(&mut self.counters);
        self.cycles = false;
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
        self.clocks = logic::cluster_clocks(
            self.counters
                .iter()
                .filter_map(|(id, after)| counted.get(id).map(|before| (*before, *after))),
        );
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
        // A nonzero argument makes xnu search the zombie list too, so a zombie is not lost.
        let info: ffi::proc_bsdinfowithuniqid = pid_info(pid, ffi::PROC_PIDT_BSDINFOWITHUNIQID, 1)?;
        let bsd = info.pbsd;
        let version = info.p_uniqidentifier.p_idversion;
        let task: libc::proc_taskinfo = if logic::zombie(bsd.pbi_status) {
            // A zombie has no task left to report, so its memory, threads and CPU time read
            // as zero.
            // SAFETY: proc_taskinfo holds only integers, for which all-zero bits are valid.
            unsafe { std::mem::zeroed() }
        } else {
            pid_info(pid, libc::PROC_PIDTASKINFO, 0)?
        };
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
        let description = match previous
            .remove(&id)
            .filter(|d| logic::same_program((d.version, &d.name), (version, &name)))
        {
            Some(description) => description,
            None => self.describe(pid, version, name),
        };
        let usage = rusage(pid, self.rusage_version);
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
            performance_share: None,
            waiting: None,
            // The background file scan reads every table, kernel_task's included.
            files: Measured::Pending,
            locks_held: Measured::Pending,
            blocked_on: None,
        };
        let responsible = self.responsible.and_then(|responsible| {
            // SAFETY: the SPI takes any pid and returns -1 when it has no answer.
            let app = unsafe { responsible(pid) };
            u32::try_from(app).ok()
        });
        self.described.insert(id, description);
        let timebase = self.timebase;
        let nanoseconds = |mach: u64| logic::nanoseconds(mach, timebase.numer, timebase.denom);
        // CPU time from the rusage reading when there is one, so that the performance-core and
        // runnable totals below are of the same moment. PROC_PIDTASKINFO reads the same kernel
        // totals (recount_task_times in xnu's fill_taskprocinfo), in the same Mach units.
        let ticks = nanoseconds(usage.map_or(
            task.pti_total_user.saturating_add(task.pti_total_system),
            |usage| usage.ri_user_time.saturating_add(usage.ri_system_time),
        ));
        let mut performance_ticks = None;
        let mut runnable_ticks = None;
        if let Some(usage) = usage.filter(|_| self.rusage_version >= libc::RUSAGE_INFO_V4) {
            runnable_ticks = Some(nanoseconds(usage.ri_runnable_time));
            self.cycles |= usage.ri_cycles > 0;
            if self.clusters {
                let performance =
                    nanoseconds(usage.ri_user_ptime.saturating_add(usage.ri_system_ptime));
                performance_ticks = Some(performance);
                self.counters.insert(
                    id,
                    logic::Counters {
                        time: ticks,
                        performance_time: performance,
                        cycles: usage.ri_cycles,
                        performance_cycles: usage.ri_pcycles,
                    },
                );
            }
        }
        Ok((
            RawProcess {
                process,
                ticks,
                io,
                performance_ticks,
                runnable_ticks,
            },
            logic::Lineage {
                pid: id.pid,
                ppid: bsd.pbi_ppid,
                responsible,
            },
        ))
    }

    /// The executable path and command line. Both can be unreadable, for example for
    /// processes that SIP protects; the command then falls back to the name.
    fn describe(&mut self, pid: c_int, version: i32, name: String) -> Description {
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
            version,
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

    /// Per-CPU use since the previous call, and each CPU's cluster clock over the interval of
    /// the last `processes` call. macOS reports no clock or run queue per CPU.
    pub fn cpus(&mut self, _dt: f32) -> Vec<Cpu> {
        let ticks = cpu_ticks(self.host);
        let cpus = ticks
            .iter()
            .enumerate()
            .map(|(index, &now)| {
                let id = index as u32;
                let kind = self
                    .core_kinds
                    .get(&id)
                    .copied()
                    .unwrap_or(CoreKind::Unknown);
                Cpu {
                    id,
                    kind,
                    busy: self
                        .cpu_previous
                        .get(index)
                        .map_or(0.0, |&before| logic::busy(before, now)),
                    mhz: match kind {
                        CoreKind::Performance => self.clocks[0],
                        CoreKind::Efficiency => self.clocks[1],
                        CoreKind::Unknown => 0.0,
                    },
                    wait: 0.0,
                }
            })
            .collect();
        self.cpu_previous = ticks;
        cpus
    }

    /// Sources macOS does not have at all, by the names `Snapshot::missing` uses. The clock is
    /// missing only when it cannot be measured per cluster either.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = vec![
            "cpu pressure",
            "io pressure",
            "last cpu",
            "cpu clock",
            "run queue",
            "cgroups",
            "socket traffic",
            "file locks",
        ];
        if self.clocked() {
            missing.retain(|&name| name != "cpu clock");
        }
        missing
    }

    /// Sources measured per cluster, by the names `Snapshot::per_cluster` uses.
    pub fn per_cluster(&self) -> Vec<&'static str> {
        if self.clocked() {
            vec!["cpu clock"]
        } else {
            Vec::new()
        }
    }

    /// Whether the cluster clocks are measured: CPU time splits by cluster and the last
    /// `processes` call counted cycles.
    fn clocked(&self) -> bool {
        self.clusters && self.cycles
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

/// Lists the descriptors of `pid` into `descriptors`, a buffer reused between processes, up to
/// `limit` of them, and returns how many were listed and whether the table held more. An error
/// when the table cannot be listed: EPERM for another user's process, ESRCH for one that has
/// exited or is a zombie.
fn list_descriptors(
    pid: c_int,
    descriptors: &mut Vec<libc::proc_fdinfo>,
    limit: usize,
) -> io::Result<(usize, bool)> {
    let empty = libc::proc_fdinfo {
        proc_fd: 0,
        proc_fdtype: 0,
    };
    if descriptors.is_empty() {
        descriptors.resize(256, empty);
    }
    loop {
        // One entry past the limit, so that a buffer filled at the cap proves there are more.
        let capacity = descriptors.len().min(limit.saturating_add(1));
        let bytes = capacity * size_of::<libc::proc_fdinfo>();
        // libproc returns 0 both for an empty table and for an error, which alone sets errno.
        // SAFETY: __error returns this thread's errno, which is always writable.
        unsafe { *libc::__error() = 0 };
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
        let Ok(written) = usize::try_from(written).map(|written| written.min(bytes)) else {
            return Err(io::Error::last_os_error());
        };
        if written == 0 {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(0) | None => Ok((0, false)),
                Some(_) => Err(error),
            };
        }
        // The kernel fills at most the whole buffer and says nothing when it truncates.
        if written < bytes {
            return Ok((written / size_of::<libc::proc_fdinfo>(), false));
        }
        if capacity > limit {
            return Ok((limit, true));
        }
        let grown = descriptors.len() * 2;
        descriptors.resize(grown, empty);
    }
}

/// One `proc_pidinfo` flavor of a process; `argument` is the flavor's own argument.
fn pid_info<T: Copy>(pid: c_int, flavor: c_int, argument: u64) -> Result<T, Absent> {
    let mut value = MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as c_int;
    // SAFETY: the buffer is valid for writes of `size` bytes.
    let written =
        unsafe { libc::proc_pidinfo(pid, flavor, argument, value.as_mut_ptr().cast(), size) };
    if written == size {
        // SAFETY: T is one of the plain-integer proc_info structs of libc or `ffi`, which every
        // bit pattern inhabits, and the kernel filled all of it.
        Ok(unsafe { value.assume_init() })
    } else if written <= 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) {
        Err(Absent::Unreadable)
    } else {
        Err(Absent::Gone)
    }
}

/// Resource usage of a process in the given flavor, V2 or later; the fields that flavor lacks
/// read 0. None when refused: another user's process, or one that has exited.
fn rusage(pid: c_int, version: c_int) -> Option<ffi::RusageInfoV6> {
    let mut usage = MaybeUninit::<ffi::RusageInfoV6>::zeroed();
    // SAFETY: the kernel copies out the struct of the requested flavor, which is at most
    // rusage_info_v6, and every older flavor is a prefix of RusageInfoV6 (asserted in ffi.rs),
    // so the buffer is large enough. The parameter is declared as rusage_info_t * but takes the
    // struct's address.
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            version,
            usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    // SAFETY: the struct holds only integers, so the zeroed or filled bytes are valid.
    (status == 0).then(|| unsafe { usage.assume_init() })
}

/// The newest `proc_pid_rusage` flavor this kernel accepts, of V6 (performance-core time and
/// cycles), V4 (cycles and runnable time) and V2. It asks about this process, which always
/// exists and may always read itself, so a refusal can only be about the flavor: xnu checks
/// the pid (ESRCH) and the permission (EPERM) before the flavor (EINVAL). Asking once here
/// means a per-process refusal later is never taken for an unsupported flavor.
fn rusage_version() -> c_int {
    // SAFETY: getpid cannot fail and has no preconditions.
    let me = unsafe { libc::getpid() };
    [ffi::RUSAGE_INFO_V6, libc::RUSAGE_INFO_V4]
        .into_iter()
        .find(|&version| rusage(me, version).is_some())
        .unwrap_or(libc::RUSAGE_INFO_V2)
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
        let Ok((count, _)) = list_descriptors(pid, descriptors, usize::MAX) else {
            return;
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

/// How much one open-file scan may read, with the same numbers as on Linux. Listing a table is
/// one `proc_pidinfo` call; examining a vnode descriptor is one `proc_pidfdinfo` call, which
/// stats the file and builds its path, much as Linux's readlink and stat do.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    /// Vnode descriptors examined per process; past them the open count is estimated.
    examined: usize,
    /// Descriptors listed per process; the listing stops there.
    listed: usize,
    /// Descriptors listed per scan. A process is only scanned while a whole `listed` share
    /// remains, so none is judged from a sliver of its table.
    scan: usize,
}

const BOUNDS: Bounds = Bounds {
    examined: 4096,
    listed: 16384,
    scan: 65536,
};

/// The open-file scan across processes, which resumes where the previous scan ran out of budget.
pub struct FileScan {
    bounds: Bounds,
    /// The last pid scanned, so the next scan starts after it.
    after: u32,
    /// The last result for each pid: None for an unreadable table.
    known: HashMap<u32, Option<Files>>,
    /// Buffers reused by every scan.
    pids: Vec<c_int>,
    descriptors: Vec<libc::proc_fdinfo>,
}

impl FileScan {
    pub fn new() -> Self {
        Self {
            bounds: BOUNDS,
            after: 0,
            known: HashMap::new(),
            pids: Vec::new(),
            descriptors: Vec::new(),
        }
    }

    /// Open files per pid; None for a table that could not be read. A pid the scan has not
    /// reached yet, this time or ever, is absent unless it keeps a previous result (a pid
    /// reused within those few seconds briefly shows its predecessor's files); so is one that
    /// exited during the scan.
    pub fn sample(&mut self) -> HashMap<u32, Option<Files>> {
        if list_pids(&mut self.pids).is_err() {
            self.known.clear();
            return HashMap::new();
        }
        let mut pids: Vec<u32> = self
            .pids
            .iter()
            .filter_map(|&pid| u32::try_from(pid).ok())
            .collect();
        pids.sort_unstable();
        pids.dedup();
        let start = pids.partition_point(|&pid| pid <= self.after);
        let mut budget = self.bounds.scan;
        let mut found = HashMap::with_capacity(pids.len());
        for &pid in pids[start..].iter().chain(&pids[..start]) {
            if budget < self.bounds.listed {
                break;
            }
            self.after = pid;
            match open_files(pid as c_int, self.bounds, &mut self.descriptors) {
                Ok((files, listed)) => {
                    budget -= listed;
                    found.insert(pid, Some(files));
                }
                Err(Absent::Gone) => {}
                Err(Absent::Unreadable) => {
                    found.insert(pid, None);
                }
            }
        }
        for pid in pids {
            if let Some(previous) = self.known.get(&pid) {
                found.entry(pid).or_insert_with(|| previous.clone());
            }
        }
        self.known = found.clone();
        found
    }
}

/// The open files of one process, and how many descriptors were listed. Unreadable for another
/// user's process without root, as on Linux; Gone for one that has exited.
fn open_files(
    pid: c_int,
    bounds: Bounds,
    descriptors: &mut Vec<libc::proc_fdinfo>,
) -> Result<(Files, usize), Absent> {
    let (listed, truncated) = match list_descriptors(pid, descriptors, bounds.listed) {
        Ok(listing) => listing,
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
            // xnu lists nothing for a zombie and answers ESRCH as for a process that is gone,
            // but a zombie's table was closed when it exited: it holds no files. A nonzero
            // argument makes PROC_PIDTBSDINFO search the zombie list.
            return match pid_info::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO, 1) {
                Ok(bsd) if logic::zombie(bsd.pbi_status) => Ok((Files::default(), 0)),
                _ => Err(Absent::Gone),
            };
        }
        Err(_) => return Err(Absent::Unreadable),
    };
    let mut examined = Vec::new();
    let mut beyond = 0;
    // Only vnodes can be files; sockets, pipes, kqueues and shared memory are told apart by
    // the listing itself, so they cost nothing and are not counted as examined.
    for descriptor in descriptors[..listed]
        .iter()
        .filter(|descriptor| descriptor.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
    {
        if examined.len() < bounds.examined {
            examined.push(vnode_descriptor(pid, descriptor.proc_fd));
        } else {
            beyond += 1;
        }
    }
    Ok((logic::table(examined, beyond, truncated), listed))
}

/// What vnode descriptor `fd` of `pid` holds: Other when it has closed or been reused for
/// something else since the listing, None when it is still open but could not be read.
fn vnode_descriptor(pid: c_int, fd: i32) -> Option<logic::Descriptor> {
    let info = match vnode_info(pid, fd) {
        Ok(info) => info,
        // xnu answers EBADF for a descriptor that is no longer open or no longer a vnode.
        Err(error) if error.raw_os_error() == Some(libc::EBADF) => {
            return Some(logic::Descriptor::Other);
        }
        // A forcibly unmounted or revoked vnode, a network filesystem whose server fails the
        // stat, or a MAC policy: the descriptor may hold a file that cannot be seen.
        Err(_) => return None,
    };
    let stat = info.pvi.vi_stat;
    Some(logic::descriptor(logic::VnodeStat {
        device: stat.vst_dev,
        mode: stat.vst_mode,
        links: stat.vst_nlink,
        inode: stat.vst_ino,
        size: stat.vst_size,
    }))
}

/// An open vnode's `vnode_fdinfo`, only when the kernel wrote exactly the size of ours, which
/// proves the layouts agree. The kernel stats the vnode the descriptor holds, so a file
/// deleted while open is still found, with a link count of 0. macOS has no equivalent of
/// Linux's AT_STATX_DONT_SYNC: the filesystem is asked for the attributes, which a network
/// filesystem answers from its attribute cache while that is fresh.
fn vnode_info(pid: c_int, fd: i32) -> io::Result<ffi::vnode_fdinfo> {
    let mut info = MaybeUninit::<ffi::vnode_fdinfo>::zeroed();
    let size = size_of::<ffi::vnode_fdinfo>() as c_int;
    // SAFETY: the buffer is valid for writes of `size` bytes.
    let written = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            ffi::PROC_PIDFDVNODEINFO,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written <= 0 {
        // libproc returns 0 only when the call failed, which set errno.
        return Err(io::Error::last_os_error());
    }
    if written != size {
        return Err(io::Error::other(
            "vnode_fdinfo size differs from the kernel's",
        ));
    }
    // SAFETY: vnode_fdinfo holds only integers, and the kernel filled all of it.
    Ok(unsafe { info.assume_init() })
}

/// File locks. macOS keeps no table of them that can be read: `fcntl(F_GETLK)` only tests a
/// range of a file the caller has open itself, so no process can be named as a holder or a
/// waiter. `Sampler::missing` names "file locks".
pub fn locks() -> Option<Locks> {
    None
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
        let missing = sampler.missing();
        assert!(
            ["last cpu", "run queue", "cgroups"]
                .iter()
                .all(|name| missing.contains(name))
        );
        assert_eq!(
            missing.contains(&"cpu clock"),
            !sampler.per_cluster().contains(&"cpu clock")
        );
        assert!(processes.len() + sampler.unreadable() <= sampler.pids.len());
    }

    #[test]
    fn this_process_ran_no_longer_on_performance_cores_than_in_all() {
        let mut sampler = Sampler::new();
        let deadline = Instant::now() + Duration::from_millis(200);
        let mut spin = 0_u64;
        while Instant::now() < deadline {
            spin = std::hint::black_box(spin.wrapping_mul(31).wrapping_add(7));
        }
        let me = std::process::id();
        let processes = sampler.processes().unwrap();
        let raw = processes
            .iter()
            .find(|raw| raw.process.id.pid == me)
            .expect("this process is listed");
        assert!(sampler.rusage_version >= libc::RUSAGE_INFO_V4);
        let runnable = raw
            .runnable_ticks
            .expect("V4 and later count runnable time");
        assert!(runnable > 0, "runnable {runnable}");
        if sampler.clusters {
            let performance = raw
                .performance_ticks
                .expect("V6 splits CPU time by cluster");
            assert!(
                performance <= raw.ticks,
                "{performance} ns on performance cores of {} ns",
                raw.ticks
            );
        } else {
            assert_eq!(raw.performance_ticks, None);
        }
        let cpus = sampler.cpus(0.2);
        assert!(cpus.iter().all(|cpu| cpu.mhz >= 0.0 && cpu.mhz < 10_000.0));
        assert_eq!(sampler.per_cluster().is_empty(), !sampler.clocked());
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

    /// A path in the temporary directory that no other test or run uses.
    fn temporary(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("isotop-{name}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn a_file_deleted_while_open_is_found_with_its_size() {
        use std::os::unix::fs::MetadataExt;
        let me = std::process::id() as c_int;
        let mut descriptors = Vec::new();
        let Ok((before, _)) = open_files(me, BOUNDS, &mut descriptors) else {
            panic!("this process can read its own table");
        };
        let path = temporary("deleted");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&[7; 12345]).unwrap();
        let metadata = file.metadata().unwrap();
        std::fs::remove_file(&path).unwrap();
        let ours = |files: &Files| {
            files
                .deleted
                .iter()
                .find(|f| f.inode == metadata.ino())
                .copied()
        };
        assert_eq!(ours(&before), None);

        // The kernel writes exactly the 176 bytes of vnode_fdinfo, or vnode_info would fail.
        let info = vnode_info(me, file.as_raw_fd()).expect("PROC_PIDFDVNODEINFO size differs");
        let stat = info.pvi.vi_stat;
        assert_eq!(
            u32::from(stat.vst_mode) & u32::from(libc::S_IFMT),
            u32::from(libc::S_IFREG)
        );
        assert_eq!(stat.vst_nlink, 0);
        assert_eq!(stat.vst_ino, metadata.ino());
        assert_eq!(stat.vst_size, 12345);

        let Ok((after, listed)) = open_files(me, BOUNDS, &mut descriptors) else {
            panic!("this process can read its own table");
        };
        assert!(listed > 0);
        let deleted = ours(&after).expect("the deleted file is still open");
        assert_eq!(deleted.size, 12345);
        assert_eq!(deleted.device, u64::from(metadata.dev() as u32));
        assert!(after.open >= 1 && !after.partial);

        drop(file);
        let Ok((closed, _)) = open_files(me, BOUNDS, &mut descriptors) else {
            panic!("this process can read its own table");
        };
        assert_eq!(ours(&closed), None, "closing the file frees it");
    }

    #[test]
    fn a_descriptor_closed_or_reused_since_the_listing_is_no_file_rather_than_unread() {
        use std::os::fd::AsRawFd;
        let me = std::process::id() as c_int;
        // No table has this many slots, so the descriptor is not open: EBADF.
        assert_eq!(
            vnode_descriptor(me, i32::MAX),
            Some(logic::Descriptor::Other)
        );
        // A descriptor that is now a socket rather than a vnode also answers EBADF.
        let socket = std::os::unix::net::UnixDatagram::unbound().unwrap();
        assert_eq!(
            vnode_descriptor(me, socket.as_raw_fd()),
            Some(logic::Descriptor::Other)
        );
    }

    #[test]
    fn the_scan_reaches_this_process_and_cannot_read_other_users_without_root() {
        let path = temporary("open");
        let file = std::fs::File::create(&path).unwrap();
        let mut scan = FileScan::new();
        let me = std::process::id();
        // A scan that runs out of budget resumes where it stopped, so a few reach every pid.
        let found = (0..64)
            .find_map(|_| scan.sample().remove(&me))
            .expect("the scan reaches this process");
        let files = found.expect("this process can read its own table");
        assert!(files.open >= 1, "{files:?}");
        drop(file);
        std::fs::remove_file(&path).unwrap();
        // SAFETY: geteuid cannot fail and has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            // launchd (pid 1) runs as root.
            assert!(matches!(
                open_files(1, BOUNDS, &mut Vec::new()),
                Err(Absent::Unreadable)
            ));
        }
    }

    #[test]
    fn a_zombie_holds_no_files() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id() as c_int;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pid_info::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO, 1)
            .is_ok_and(|bsd| logic::zombie(bsd.pbi_status))
        {
            assert!(Instant::now() < deadline, "the child never became a zombie");
            std::thread::sleep(Duration::from_millis(10));
        }
        let Ok((files, listed)) = open_files(pid, BOUNDS, &mut Vec::new()) else {
            panic!("a zombie is read as holding no files");
        };
        assert_eq!((files, listed), (Files::default(), 0));
        child.wait().unwrap();
    }

    #[test]
    fn a_listing_capped_below_the_table_says_it_was_cut_short() {
        let files: Vec<std::fs::File> = (0..8)
            .map(|_| std::fs::File::open("/dev/null").unwrap())
            .collect();
        let me = std::process::id() as c_int;
        let mut descriptors = Vec::new();
        let (all, cut) = list_descriptors(me, &mut descriptors, usize::MAX).unwrap();
        assert!(all >= files.len() && !cut);
        assert_eq!(
            list_descriptors(me, &mut descriptors, 4).unwrap(),
            (4, true)
        );
        let (again, cut) = list_descriptors(me, &mut descriptors, all + 64).unwrap();
        assert!(again >= files.len() && !cut);
    }
}
