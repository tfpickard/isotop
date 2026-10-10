//! Declarations libc 0.2.190 lacks or marks deprecated: Mach time and ports, the
//! `vm_statistics64` layout of the public xnu header, the responsibility SPI, IOKit and
//! CoreFoundation for the CPU cluster types, and `socket_fdinfo` and `vnode_fdinfowithpath` from
//! xnu `bsd/sys/proc_info.h`.
//!
//! The C structs keep their header names and every field, so the layouts can be checked against
//! the header line by line; isotop reads only some of the fields.

#![allow(non_camel_case_types, non_upper_case_globals, dead_code)]

use std::ffi::{c_char, c_int, c_void};
use std::mem::{align_of, offset_of, size_of};

// Mach. libc declares these but deprecates them in favour of the mach2 crate; they live in
// libSystem, which every program links.

/// `struct mach_timebase_info` (mach/mach_time.h): Mach absolute time times numer / denom is
/// nanoseconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MachTimebaseInfo {
    pub numer: u32,
    pub denom: u32,
}

unsafe extern "C" {
    pub fn mach_timebase_info(info: *mut MachTimebaseInfo) -> libc::kern_return_t;
    /// Returns a send right to the host port; each call adds a reference, so call it once.
    pub fn mach_host_self() -> libc::mach_port_t;
    /// What the C macro `mach_task_self()` reads.
    static mach_task_self_: libc::mach_port_t;
}

/// This task's port, for `vm_deallocate`.
pub fn mach_task_self() -> libc::mach_port_t {
    // SAFETY: libSystem sets mach_task_self_ before any Rust code runs and never changes it.
    unsafe { mach_task_self_ }
}

/// `struct vm_statistics64` exactly as the public xnu header (osfmk/mach/vm_statistics.h)
/// defines it: 160 bytes, `aligned(8)`. libc's copy carries fields from a newer SDK past
/// `swapped_count` that the header does not have.
#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, Default)]
pub struct VmStatistics64 {
    pub free_count: u32,
    pub active_count: u32,
    pub inactive_count: u32,
    pub wire_count: u32,
    pub zero_fill_count: u64,
    pub reactivations: u64,
    pub pageins: u64,
    pub pageouts: u64,
    pub faults: u64,
    pub cow_faults: u64,
    pub lookups: u64,
    pub hits: u64,
    pub purges: u64,
    pub purgeable_count: u32,
    pub speculative_count: u32,
    pub decompressions: u64,
    pub compressions: u64,
    pub swapins: u64,
    pub swapouts: u64,
    pub compressor_page_count: u32,
    pub throttled_count: u32,
    pub external_page_count: u32,
    pub internal_page_count: u32,
    pub total_uncompressed_pages_in_compressor: u64,
    pub swapped_count: u64,
}

/// `HOST_VM_INFO64_COUNT`: the struct's size in `integer_t`s.
pub const HOST_VM_INFO64_COUNT: libc::mach_msg_type_number_t =
    (size_of::<VmStatistics64>() / size_of::<libc::integer_t>()) as u32;

const _: () = assert!(size_of::<VmStatistics64>() == 160);
const _: () = assert!(align_of::<VmStatistics64>() == 8);
const _: () = assert!(HOST_VM_INFO64_COUNT == 40);
const _: () = assert!(offset_of!(VmStatistics64, purgeable_count) == 88);
const _: () = assert!(offset_of!(VmStatistics64, decompressions) == 96);
const _: () = assert!(offset_of!(VmStatistics64, compressor_page_count) == 128);
const _: () = assert!(offset_of!(VmStatistics64, swapped_count) == 152);

/// `RUSAGE_INFO_V6` flavor for `proc_pid_rusage` (bsd/sys/resource.h); libc stops at V4.
pub const RUSAGE_INFO_V6: c_int = 6;

/// `struct rusage_info_v6` exactly as xnu bsd/sys/resource.h defines it: the 16-byte uuid, 47
/// named uint64_t fields and `ri_reserved[9]`, so 16 + 56 * 8 = 464 bytes. Each earlier
/// version is a prefix of it (v2 ends before `ri_cpu_time_qos_default`, v4 before `ri_flags`),
/// so a zeroed v6 buffer can receive any older flavor and its later fields stay zero.
///
/// Units, from xnu: times (`ri_user_time`, `ri_system_time`, `ri_user_ptime`,
/// `ri_system_ptime`, `ri_runnable_time`) are Mach absolute time, which `mach_timebase_info`
/// converts to nanoseconds; `ri_cycles` and `ri_pcycles` are CPU cycles. `ri_runnable_time`
/// counts the whole time threads were runnable, running included (osfmk/kern/sched_prim.c
/// starts the timer when a thread is made runnable and stops it only when the thread blocks),
/// and `ri_*ptime`/`ri_p*` cover only time on performance cores.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RusageInfoV6 {
    pub ri_uuid: [u8; 16],
    pub ri_user_time: u64,
    pub ri_system_time: u64,
    pub ri_pkg_idle_wkups: u64,
    pub ri_interrupt_wkups: u64,
    pub ri_pageins: u64,
    pub ri_wired_size: u64,
    pub ri_resident_size: u64,
    pub ri_phys_footprint: u64,
    pub ri_proc_start_abstime: u64,
    pub ri_proc_exit_abstime: u64,
    pub ri_child_user_time: u64,
    pub ri_child_system_time: u64,
    pub ri_child_pkg_idle_wkups: u64,
    pub ri_child_interrupt_wkups: u64,
    pub ri_child_pageins: u64,
    pub ri_child_elapsed_abstime: u64,
    pub ri_diskio_bytesread: u64,
    pub ri_diskio_byteswritten: u64,
    pub ri_cpu_time_qos_default: u64,
    pub ri_cpu_time_qos_maintenance: u64,
    pub ri_cpu_time_qos_background: u64,
    pub ri_cpu_time_qos_utility: u64,
    pub ri_cpu_time_qos_legacy: u64,
    pub ri_cpu_time_qos_user_initiated: u64,
    pub ri_cpu_time_qos_user_interactive: u64,
    pub ri_billed_system_time: u64,
    pub ri_serviced_system_time: u64,
    pub ri_logical_writes: u64,
    pub ri_lifetime_max_phys_footprint: u64,
    pub ri_instructions: u64,
    pub ri_cycles: u64,
    pub ri_billed_energy: u64,
    pub ri_serviced_energy: u64,
    pub ri_interval_max_phys_footprint: u64,
    pub ri_runnable_time: u64,
    pub ri_flags: u64,
    pub ri_user_ptime: u64,
    pub ri_system_ptime: u64,
    pub ri_pinstructions: u64,
    pub ri_pcycles: u64,
    pub ri_energy_nj: u64,
    pub ri_penergy_nj: u64,
    pub ri_secure_time_in_system: u64,
    pub ri_secure_ptime_in_system: u64,
    pub ri_neural_footprint: u64,
    pub ri_lifetime_max_neural_footprint: u64,
    pub ri_interval_max_neural_footprint: u64,
    pub ri_reserved: [u64; 9],
}

const _: () = assert!(size_of::<RusageInfoV6>() == 16 + (47 + 9) * 8);
const _: () = assert!(size_of::<RusageInfoV6>() == 464);
const _: () = assert!(align_of::<RusageInfoV6>() == 8);
const _: () = assert!(offset_of!(RusageInfoV6, ri_user_time) == 16);
const _: () = assert!(offset_of!(RusageInfoV6, ri_phys_footprint) == 72);
const _: () = assert!(offset_of!(RusageInfoV6, ri_diskio_bytesread) == 144);
const _: () = assert!(offset_of!(RusageInfoV6, ri_cycles) == 256);
const _: () = assert!(offset_of!(RusageInfoV6, ri_runnable_time) == 288);
const _: () = assert!(offset_of!(RusageInfoV6, ri_user_ptime) == 304);
const _: () = assert!(offset_of!(RusageInfoV6, ri_pcycles) == 328);
const _: () = assert!(offset_of!(RusageInfoV6, ri_reserved) == 392);
// The older flavors are prefixes: libc's v2 and v4 end where the fields they lack begin.
const _: () =
    assert!(size_of::<libc::rusage_info_v2>() == offset_of!(RusageInfoV6, ri_cpu_time_qos_default));
const _: () = assert!(size_of::<libc::rusage_info_v4>() == offset_of!(RusageInfoV6, ri_flags));

// The responsibility SPI: `pid_t responsibility_get_pid_responsible_for_pid(pid_t)`, as
// Chromium declares it. It is private, so it is looked up at run time and never linked.

/// Returns the pid responsible for a process, or -1 on failure.
pub type Responsible = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

/// The responsibility SPI, when this macOS exports it.
pub fn responsible() -> Option<Responsible> {
    // SAFETY: RTLD_DEFAULT searches the loaded images and the name is NUL-terminated; dlsym
    // only looks the symbol up.
    let symbol = unsafe {
        libc::dlsym(
            libc::RTLD_DEFAULT,
            c"responsibility_get_pid_responsible_for_pid".as_ptr(),
        )
    };
    if symbol.is_null() {
        return None;
    }
    // SAFETY: the symbol is the C function above, which takes and returns one pid_t, so the
    // pointer has exactly the type Responsible describes.
    Some(unsafe { std::mem::transmute::<*mut c_void, Responsible>(symbol) })
}

// IOKit and CoreFoundation, for `cluster-type` and `logical-cpu-id` under IODeviceTree:/cpus.
// Signatures from IOKitUser IOKitLib.h and CoreFoundation's CFBase.h, CFNumber.h and CFData.h.

pub type CFTypeRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFTypeID = usize;
pub type CFIndex = isize;
pub type CFNumberType = CFIndex;
pub type CFStringEncoding = u32;
pub type io_object_t = libc::mach_port_t;

pub const kCFStringEncodingUTF8: CFStringEncoding = 0x0800_0100;
pub const kCFNumberSInt64Type: CFNumberType = 4;
/// `kIOMainPortDefault`, which IOKitLib.h defines as `MACH_PORT_NULL`.
pub const kIOMainPortDefault: libc::mach_port_t = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Returns 0 when the path names no entry.
    pub fn IORegistryEntryFromPath(
        main_port: libc::mach_port_t,
        path: *const c_char,
    ) -> io_object_t;
    pub fn IORegistryEntryGetChildIterator(
        entry: io_object_t,
        plane: *const c_char,
        iterator: *mut io_object_t,
    ) -> libc::kern_return_t;
    /// Returns 0 when the iteration is done.
    pub fn IOIteratorNext(iterator: io_object_t) -> io_object_t;
    /// Returns a retained property, or null when the entry has none.
    pub fn IORegistryEntrySearchCFProperty(
        entry: io_object_t,
        plane: *const c_char,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> CFTypeRef;
    pub fn IOObjectRelease(object: io_object_t) -> libc::kern_return_t;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFStringCreateWithCString(
        allocator: CFAllocatorRef,
        text: *const c_char,
        encoding: CFStringEncoding,
    ) -> CFStringRef;
    pub fn CFRelease(object: CFTypeRef);
    pub fn CFGetTypeID(object: CFTypeRef) -> CFTypeID;
    pub fn CFNumberGetTypeID() -> CFTypeID;
    pub fn CFDataGetTypeID() -> CFTypeID;
    /// Returns nonzero when the value converted without loss.
    pub fn CFNumberGetValue(number: CFTypeRef, kind: CFNumberType, value: *mut c_void) -> u8;
    pub fn CFDataGetLength(data: CFTypeRef) -> CFIndex;
    pub fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
}

// proc_pidinfo flavor PROC_PIDT_BSDINFOWITHUNIQID, from xnu bsd/sys/proc_info.h: the BSD info
// and the process's unique identifiers in one call, under the same permission check as
// PROC_PIDTBSDINFO. Newer headers name the int32 after p_idversion p_orig_ppidversion and older
// ones p_reserve2; the layout is the same, 56 bytes with p_idversion at offset 32.

pub const PROC_PIDT_BSDINFOWITHUNIQID: c_int = 18;

/// `struct proc_uniqidentifierinfo`. `p_idversion` is the pid version, which every exec
/// changes while the pid and start time stay.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct proc_uniqidentifierinfo {
    pub p_uuid: [u8; 16],
    pub p_uniqueid: u64,
    pub p_puniqueid: u64,
    pub p_idversion: i32,
    pub p_orig_ppidversion: i32,
    pub p_reserve2: u64,
    pub p_reserve3: u64,
}

/// `struct proc_bsdinfowithuniqid`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct proc_bsdinfowithuniqid {
    pub pbsd: libc::proc_bsdinfo,
    pub p_uniqidentifier: proc_uniqidentifierinfo,
}

const _: () = assert!(size_of::<proc_uniqidentifierinfo>() == 56);
const _: () = assert!(offset_of!(proc_uniqidentifierinfo, p_idversion) == 32);
const _: () = assert!(size_of::<libc::proc_bsdinfo>() == 136);
const _: () = assert!(size_of::<proc_bsdinfowithuniqid>() == 192);
const _: () = assert!(align_of::<proc_bsdinfowithuniqid>() == 8);

// struct socket_fdinfo and its members, from xnu bsd/sys/proc_info.h. The text of these structs
// is identical from xnu-7195 (macOS 11) to main. Every field is a fixed-width scalar and nothing
// is packed, so repr(C) reproduces the C layout; the assertions below pin it.

/// `PROC_PIDFDSOCKETINFO` flavor for `proc_pidfdinfo`.
pub const PROC_PIDFDSOCKETINFO: c_int = 3;

pub const SOCKINFO_GENERIC: i32 = 0;
pub const SOCKINFO_IN: i32 = 1;
pub const SOCKINFO_TCP: i32 = 2;
pub const SOCKINFO_UN: i32 = 3;
pub const SOCKINFO_NDRV: i32 = 4;
pub const SOCKINFO_KERN_EVENT: i32 = 5;
pub const SOCKINFO_KERN_CTL: i32 = 6;
pub const SOCKINFO_VSOCK: i32 = 7;

pub const INI_IPV4: u8 = 0x1;
pub const INI_IPV6: u8 = 0x2;

pub const TSI_T_NTIMERS: usize = 4;

/// `SOCK_MAXADDRLEN` (bsd/sys/socket.h).
pub const SOCK_MAXADDRLEN: usize = 255;
/// `IF_NAMESIZE` (bsd/net/if.h).
pub const IF_NAMESIZE: usize = 16;
/// `MAX_KCTL_NAME` (bsd/sys/kern_control.h).
pub const MAX_KCTL_NAME: usize = 96;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct proc_fileinfo {
    pub fi_openflags: u32,
    pub fi_status: u32,
    pub fi_offset: i64,
    pub fi_type: i32,
    pub fi_guardflags: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct vinfo_stat {
    pub vst_dev: u32,
    pub vst_mode: u16,
    pub vst_nlink: u16,
    pub vst_ino: u64,
    pub vst_uid: u32,
    pub vst_gid: u32,
    pub vst_atime: i64,
    pub vst_atimensec: i64,
    pub vst_mtime: i64,
    pub vst_mtimensec: i64,
    pub vst_ctime: i64,
    pub vst_ctimensec: i64,
    pub vst_birthtime: i64,
    pub vst_birthtimensec: i64,
    pub vst_size: i64,
    pub vst_blocks: i64,
    pub vst_blksize: i32,
    pub vst_flags: u32,
    pub vst_gen: u32,
    pub vst_rdev: u32,
    pub vst_qspare: [i64; 2],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct sockbuf_info {
    pub sbi_cc: u32,
    pub sbi_hiwat: u32,
    pub sbi_mbcnt: u32,
    pub sbi_mbmax: u32,
    pub sbi_lowat: u32,
    pub sbi_flags: i16,
    pub sbi_timeo: i16,
}

/// `struct in_addr`; `s_addr` is in network byte order.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct in_addr {
    pub s_addr: u32,
}

/// `struct in6_addr`: a union of u8[16], u16[8] and u32[4], so align 4.
#[repr(C)]
#[derive(Copy, Clone)]
pub union in6_addr {
    pub u6_addr8: [u8; 16],
    pub u6_addr16: [u16; 8],
    pub u6_addr32: [u32; 4],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct in4in6_addr {
    pub i46a_pad32: [u32; 3],
    pub i46a_addr4: in_addr,
}

/// The anonymous union of `insi_faddr` and `insi_laddr`.
#[repr(C)]
#[derive(Copy, Clone)]
pub union in_sockinfo_addr {
    pub ina_46: in4in6_addr,
    pub ina_6: in6_addr,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct in_sockinfo_v4 {
    pub in4_tos: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct in_sockinfo_v6 {
    pub in6_hlim: u8,
    pub in6_cksum: i32,
    pub in6_ifindex: u16,
    pub in6_hops: i16,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct in_sockinfo {
    /// Network-order `u_short` widened to `int`.
    pub insi_fport: i32,
    pub insi_lport: i32,
    pub insi_gencnt: u64,
    pub insi_flags: u32,
    pub insi_flow: u32,
    pub insi_vflag: u8,
    pub insi_ip_ttl: u8,
    pub rfu_1: u32,
    pub insi_faddr: in_sockinfo_addr,
    pub insi_laddr: in_sockinfo_addr,
    pub insi_v4: in_sockinfo_v4,
    pub insi_v6: in_sockinfo_v6,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct tcp_sockinfo {
    pub tcpsi_ini: in_sockinfo,
    pub tcpsi_state: i32,
    pub tcpsi_timer: [i32; TSI_T_NTIMERS],
    pub tcpsi_mss: i32,
    pub tcpsi_flags: u32,
    pub rfu_1: u32,
    pub tcpsi_tp: u64,
}

/// `struct sockaddr_un`: 106 bytes, align 1.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct sockaddr_un {
    pub sun_len: u8,
    pub sun_family: u8,
    pub sun_path: [u8; 104],
}

/// The anonymous union of `unsi_addr` and `unsi_caddr`.
#[repr(C)]
#[derive(Copy, Clone)]
pub union un_sockinfo_addr {
    pub ua_sun: sockaddr_un,
    pub ua_dummy: [u8; SOCK_MAXADDRLEN],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct un_sockinfo {
    pub unsi_conn_so: u64,
    pub unsi_conn_pcb: u64,
    pub unsi_addr: un_sockinfo_addr,
    pub unsi_caddr: un_sockinfo_addr,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct ndrv_info {
    pub ndrvsi_if_family: u32,
    pub ndrvsi_if_unit: u32,
    pub ndrvsi_if_name: [u8; IF_NAMESIZE],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct kern_event_info {
    pub kesi_vendor_code_filter: u32,
    pub kesi_class_filter: u32,
    pub kesi_subclass_filter: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct kern_ctl_info {
    pub kcsi_id: u32,
    pub kcsi_reg_unit: u32,
    pub kcsi_flags: u32,
    pub kcsi_recvbufsize: u32,
    pub kcsi_sendbufsize: u32,
    pub kcsi_unit: u32,
    pub kcsi_name: [u8; MAX_KCTL_NAME],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct vsock_sockinfo {
    pub local_cid: u32,
    pub local_port: u32,
    pub remote_cid: u32,
    pub remote_port: u32,
}

/// The anonymous union `soi_proto`; `soi_kind` says which member the kernel filled.
#[repr(C)]
#[derive(Copy, Clone)]
pub union socket_info_proto {
    pub pri_in: in_sockinfo,
    pub pri_tcp: tcp_sockinfo,
    pub pri_un: un_sockinfo,
    pub pri_ndrv: ndrv_info,
    pub pri_kern_event: kern_event_info,
    pub pri_kern_ctl: kern_ctl_info,
    pub pri_vsock: vsock_sockinfo,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct socket_info {
    pub soi_stat: vinfo_stat,
    pub soi_so: u64,
    pub soi_pcb: u64,
    pub soi_type: i32,
    pub soi_protocol: i32,
    pub soi_family: i32,
    pub soi_options: i16,
    pub soi_linger: i16,
    pub soi_state: i16,
    pub soi_qlen: i16,
    pub soi_incqlen: i16,
    pub soi_qlimit: i16,
    pub soi_timeo: i16,
    pub soi_error: u16,
    pub soi_oobmark: u32,
    pub soi_rcv: sockbuf_info,
    pub soi_snd: sockbuf_info,
    pub soi_kind: i32,
    pub rfu_1: u32,
    pub soi_proto: socket_info_proto,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct socket_fdinfo {
    pub pfi: proc_fileinfo,
    pub psi: socket_info,
}

// struct vnode_fdinfowithpath and its members, from the same header. libc 0.2.190 has
// vnode_info and vnode_info_path but not the fdinfo wrapper or its flavor; these are declared
// on this file's vinfo_stat and proc_fileinfo so one definition of each serves both flavors, and
// the assertions below check them against libc's copies too.

/// `PROC_PIDFDVNODEPATHINFO` flavor for `proc_pidfdinfo`: the open file's `vnode_info` and path.
pub const PROC_PIDFDVNODEPATHINFO: c_int = 2;

/// `MAXPATHLEN` (bsd/sys/param.h).
pub const MAXPATHLEN: usize = 1024;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct vnode_info {
    pub vi_stat: vinfo_stat,
    pub vi_type: c_int,
    pub vi_pad: c_int,
    pub vi_fsid: libc::fsid_t,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct vnode_info_path {
    pub vip_vi: vnode_info,
    pub vip_path: [c_char; MAXPATHLEN],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct vnode_fdinfowithpath {
    pub pfi: proc_fileinfo,
    pub pvip: vnode_info_path,
}

macro_rules! assert_layout {
    ($t:ty, $size:expr, $align:expr) => {
        const _: () = assert!(size_of::<$t>() == $size);
        const _: () = assert!(align_of::<$t>() == $align);
    };
}

macro_rules! assert_offset {
    ($t:ty, $field:ident, $offset:expr) => {
        const _: () = assert!(offset_of!($t, $field) == $offset);
    };
}

assert_layout!(proc_fileinfo, 24, 8);
assert_layout!(vinfo_stat, 136, 8);
assert_layout!(sockbuf_info, 24, 4);
assert_layout!(in_addr, 4, 4);
assert_layout!(in6_addr, 16, 4);
assert_layout!(in4in6_addr, 16, 4);
assert_layout!(in_sockinfo_addr, 16, 4);
assert_layout!(in_sockinfo_v4, 1, 1);
assert_layout!(in_sockinfo_v6, 12, 4);
assert_layout!(in_sockinfo, 80, 8);
assert_layout!(tcp_sockinfo, 120, 8);
assert_layout!(sockaddr_un, 106, 1);
assert_layout!(un_sockinfo_addr, 255, 1);
assert_layout!(un_sockinfo, 528, 8);
assert_layout!(ndrv_info, 24, 4);
assert_layout!(kern_event_info, 12, 4);
assert_layout!(kern_ctl_info, 120, 4);
assert_layout!(vsock_sockinfo, 16, 4);
assert_layout!(socket_info_proto, 528, 8);
assert_layout!(socket_info, 768, 8);
assert_layout!(socket_fdinfo, 792, 8);

assert_offset!(proc_fileinfo, fi_offset, 8);
assert_offset!(proc_fileinfo, fi_type, 16);
assert_offset!(proc_fileinfo, fi_guardflags, 20);

assert_offset!(vinfo_stat, vst_ino, 8);
assert_offset!(vinfo_stat, vst_uid, 16);
assert_offset!(vinfo_stat, vst_atime, 24);
assert_offset!(vinfo_stat, vst_size, 88);
assert_offset!(vinfo_stat, vst_blksize, 104);
assert_offset!(vinfo_stat, vst_rdev, 116);
assert_offset!(vinfo_stat, vst_qspare, 120);

assert_offset!(in_sockinfo, insi_lport, 4);
assert_offset!(in_sockinfo, insi_gencnt, 8);
assert_offset!(in_sockinfo, insi_flags, 16);
assert_offset!(in_sockinfo, insi_flow, 20);
assert_offset!(in_sockinfo, insi_vflag, 24);
assert_offset!(in_sockinfo, insi_ip_ttl, 25);
assert_offset!(in_sockinfo, rfu_1, 28);
assert_offset!(in_sockinfo, insi_faddr, 32);
assert_offset!(in_sockinfo, insi_laddr, 48);
assert_offset!(in_sockinfo, insi_v4, 64);
assert_offset!(in_sockinfo, insi_v6, 68);
assert_offset!(in_sockinfo_v6, in6_cksum, 4);
assert_offset!(in_sockinfo_v6, in6_ifindex, 8);
assert_offset!(in_sockinfo_v6, in6_hops, 10);
assert_offset!(in4in6_addr, i46a_addr4, 12);

assert_offset!(tcp_sockinfo, tcpsi_state, 80);
assert_offset!(tcp_sockinfo, tcpsi_timer, 84);
assert_offset!(tcp_sockinfo, tcpsi_mss, 100);
assert_offset!(tcp_sockinfo, tcpsi_flags, 104);
assert_offset!(tcp_sockinfo, rfu_1, 108);
assert_offset!(tcp_sockinfo, tcpsi_tp, 112);

assert_offset!(un_sockinfo, unsi_conn_pcb, 8);
assert_offset!(un_sockinfo, unsi_addr, 16);
assert_offset!(un_sockinfo, unsi_caddr, 271);

assert_offset!(socket_info, soi_so, 136);
assert_offset!(socket_info, soi_pcb, 144);
assert_offset!(socket_info, soi_type, 152);
assert_offset!(socket_info, soi_protocol, 156);
assert_offset!(socket_info, soi_family, 160);
assert_offset!(socket_info, soi_options, 164);
assert_offset!(socket_info, soi_linger, 166);
assert_offset!(socket_info, soi_state, 168);
assert_offset!(socket_info, soi_qlen, 170);
assert_offset!(socket_info, soi_incqlen, 172);
assert_offset!(socket_info, soi_qlimit, 174);
assert_offset!(socket_info, soi_timeo, 176);
assert_offset!(socket_info, soi_error, 178);
assert_offset!(socket_info, soi_oobmark, 180);
assert_offset!(socket_info, soi_rcv, 184);
assert_offset!(socket_info, soi_snd, 208);
assert_offset!(socket_info, soi_kind, 232);
assert_offset!(socket_info, rfu_1, 236);
assert_offset!(socket_info, soi_proto, 240);

assert_offset!(socket_fdinfo, psi, 24);

// vinfo_stat is 136 bytes, vnode_info adds vi_type, vi_pad and an 8-byte fsid_t (152),
// vnode_info_path a MAXPATHLEN path (1176), and the fdinfo wrapper the 24-byte proc_fileinfo
// in front (1200), which is PROC_PIDFDVNODEPATHINFO_SIZE.
assert_layout!(vnode_info, 152, 8);
assert_layout!(vnode_info_path, 1176, 8);
assert_layout!(vnode_fdinfowithpath, 1200, 8);
assert_offset!(vnode_info, vi_type, 136);
assert_offset!(vnode_info, vi_fsid, 144);
assert_offset!(vnode_info_path, vip_path, 152);
assert_offset!(vnode_fdinfowithpath, pvip, 24);
assert_offset!(vinfo_stat, vst_mode, 4);
assert_offset!(vinfo_stat, vst_nlink, 6);
const _: () = assert!(size_of::<libc::vinfo_stat>() == size_of::<vinfo_stat>());
const _: () = assert!(size_of::<libc::vnode_info>() == size_of::<vnode_info>());
const _: () = assert!(size_of::<libc::vnode_info_path>() == size_of::<vnode_info_path>());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rusage_info_v6_has_the_size_of_the_xnu_header() {
        // 16 bytes of uuid, 47 named counters and 9 reserved ones.
        assert_eq!(size_of::<RusageInfoV6>(), 464);
        assert_eq!(
            offset_of!(RusageInfoV6, ri_user_ptime),
            size_of::<libc::rusage_info_v4>() + 8
        );
    }
}
