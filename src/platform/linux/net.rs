//! Socket links between processes: loopback TCP pairs, Unix-socket peers, and TCP connections
//! that leave the machine. Only sockets of processes whose file descriptors we may read are seen.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::model::Queue;
use crate::platform::{Loopback, Network, Remote};

pub fn sample() -> Network {
    let owners = socket_owners();
    let mut network = Network::default();
    let mut link = |a: u32, b: u32| {
        if a != b {
            *network.links.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    };
    let mut tcp = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = fs::read_to_string(path) {
            tcp.extend(text.lines().skip(1).filter_map(parse_tcp));
        }
    }
    let endpoints: HashMap<(&str, &str), u64> = tcp
        .iter()
        .map(|c| ((c.local.as_str(), c.remote.as_str()), c.inode))
        .collect();
    // Inodes of local connections that have an owner, with the inode of the other end.
    let mut local = HashMap::new();
    for connection in &tcp {
        let Some(&pid) = owners.get(&connection.inode) else {
            continue;
        };
        match endpoints.get(&(connection.remote.as_str(), connection.local.as_str())) {
            // Both directions of a local connection are listed; count the pair once.
            Some(peer) => {
                local.insert(connection.inode, *peer);
                if connection.local < connection.remote
                    && let Some(&other) = owners.get(peer)
                {
                    link(pid, other);
                }
            }
            None => *network.outside.entry(pid).or_default() += 1,
        }
    }
    let mut unix = HashSet::new();
    let sockets = unix_peers().unwrap_or_default();
    for socket in &sockets {
        if socket.inode < socket.peer
            && let (Some(&a), Some(&b)) = (owners.get(&socket.inode), owners.get(&socket.peer))
        {
            link(a, b);
            if a != b {
                unix.insert((a.min(b), a.max(b)));
            }
        }
    }
    for family in [libc::AF_INET, libc::AF_INET6] {
        for socket in tcp_sockets(family as u8).unwrap_or_default() {
            if let Some(peer) = local.get(&socket.inode) {
                if let (Some(&pid), Some(&other)) = (owners.get(&socket.inode), owners.get(peer))
                    && pid != other
                {
                    network.loopback.push(Loopback {
                        inode: socket.inode,
                        pid,
                        peer: other,
                        received: socket.received,
                    });
                }
                continue;
            }
            if let Some(&pid) = owners.get(&socket.inode) {
                network.remotes.push(Remote { pid, ..socket });
            }
        }
    }
    network.unix = unix;
    network.unix_queues = unix_queues(&sockets, &owners);
    network
}

/// Socket inode to owning pid, from the `socket:[inode]` links in /proc/<pid>/fd.
fn socket_owners() -> HashMap<u64, u32> {
    let mut owners = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return owners;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(descriptors) = fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for descriptor in descriptors.flatten() {
            if let Ok(target) = fs::read_link(descriptor.path())
                && let Some(inode) = target
                    .to_str()
                    .and_then(|t| t.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok())
            {
                owners.entry(inode).or_insert(pid);
            }
        }
    }
    owners
}

#[derive(Debug, PartialEq)]
struct Tcp {
    local: String,
    remote: String,
    inode: u64,
}

/// One /proc/net/tcp{,6} row; only established connections with a known inode.
fn parse_tcp(line: &str) -> Option<Tcp> {
    const ESTABLISHED: &str = "01";
    let fields: Vec<&str> = line.split_whitespace().collect();
    let inode = fields.get(9)?.parse().ok().filter(|&inode| inode != 0)?;
    (*fields.get(3)? == ESTABLISHED).then(|| Tcp {
        local: fields[1].to_owned(),
        remote: fields[2].to_owned(),
        inode,
    })
}

/// Sends one sock_diag dump request and hands each reply message's payload to `each`.
fn diag(request: &[u8], mut each: impl FnMut(&[u8])) -> io::Result<()> {
    const SOCK_DIAG_BY_FAMILY: u16 = 20;
    // SAFETY: socket() has no memory-safety preconditions; the result is checked below.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_SOCK_DIAG,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a freshly created descriptor that nothing else owns.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut message = vec![0_u8; 16];
    message.extend_from_slice(request);
    let length = message.len() as u32;
    message[0..4].copy_from_slice(&length.to_ne_bytes());
    message[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    let flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
    message[6..8].copy_from_slice(&flags.to_ne_bytes());
    message[8..12].copy_from_slice(&1_u32.to_ne_bytes());
    // SAFETY: an all-zero sockaddr_nl is valid; the family is set before use.
    let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    kernel.nl_family = libc::AF_NETLINK as u16;
    // SAFETY: message and kernel outlive the call and their lengths are passed exactly.
    let sent = unsafe {
        libc::sendto(
            socket.as_raw_fd(),
            message.as_ptr().cast(),
            message.len(),
            0,
            (&kernel as *const libc::sockaddr_nl).cast(),
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0_u8; 1 << 16];
    loop {
        // SAFETY: the buffer is valid for writes of its full length.
        let received = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
            )
        };
        if received <= 0 {
            return Err(io::Error::last_os_error());
        }
        match messages(&buffer[..received as usize], &mut each) {
            Some(true) => return Ok(()),
            Some(false) => {}
            None => return Err(io::Error::other("malformed sock_diag reply")),
        }
    }
}

/// Splits one netlink datagram into message payloads. Returns Some(true) at NLMSG_DONE.
fn messages(buffer: &[u8], each: &mut impl FnMut(&[u8])) -> Option<bool> {
    let mut offset = 0;
    while offset + 16 <= buffer.len() {
        let length = u32_at(buffer, offset)? as usize;
        let kind = u16_at(buffer, offset + 4)? as i32;
        if length < 16 || offset + length > buffer.len() {
            return None;
        }
        match kind {
            libc::NLMSG_DONE => return Some(true),
            libc::NLMSG_ERROR => return None,
            _ => each(&buffer[offset + 16..offset + length]),
        }
        offset += (length + 3) & !3;
    }
    Some(false)
}

/// Netlink attributes after a fixed header: (type, payload) pairs.
pub(super) fn attributes(message: &[u8], start: usize) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = start;
    std::iter::from_fn(move || {
        let size = u16_at(message, at)? as usize;
        let kind = u16_at(message, at + 2)?;
        if size < 4 {
            return None;
        }
        let payload = message.get(at + 4..(at + size).min(message.len()))?;
        at += (size + 3) & !3;
        Some((kind, payload))
    })
}

pub(super) fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

pub(super) fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub(super) fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// One connected Unix-domain socket from sock_diag.
#[derive(Debug, PartialEq)]
struct UnixSocket {
    inode: u64,
    /// The inode of the socket at the other end.
    peer: u64,
    /// What waits unread in this socket's receive queue; None for a listening socket, whose queue
    /// length is its backlog, and for a kernel that reports no queue length.
    queue: Option<Queue>,
}

/// Connected Unix sockets with their peers and receive queues. The request asks for
/// `UDIAG_SHOW_PEER` and `UDIAG_SHOW_RQLEN` (include/uapi/linux/unix_diag.h); the queue length
/// rides in the same dump, so it costs no further syscall.
fn unix_peers() -> io::Result<Vec<UnixSocket>> {
    const UDIAG_SHOW_PEER: u32 = 0x4;
    const UDIAG_SHOW_RQLEN: u32 = 0x10;
    let mut request = [0_u8; 24];
    request[0] = libc::AF_UNIX as u8;
    request[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
    request[12..16].copy_from_slice(&(UDIAG_SHOW_PEER | UDIAG_SHOW_RQLEN).to_ne_bytes());
    let mut sockets = Vec::new();
    diag(&request, |message| sockets.extend(unix_socket(message)))?;
    Ok(sockets)
}

/// unix_diag_msg: family, type (`sk_type`), state (`sk_state`), pad, inode (u32), cookie
/// (2 x u32); then attributes. `UNIX_DIAG_PEER` (2) is the peer's inode (u32) and
/// `UNIX_DIAG_RQLEN` (4) is `unix_diag_rqlen`: rqueue (u32) and wqueue (u32).
///
/// What rqueue means follows `unix_inq_len` (net/unix/af_unix.c), which fills it: for a stream or
/// seqpacket socket the sum of the bytes in its receive queue, and for any other type (datagram)
/// the length of the first packet only, which can say that one is waiting and no more. For a
/// listening socket `sk_diag_show_rqlen` (net/unix/diag.c) puts the backlog there instead, so a
/// listener has no queue. wqueue is the send buffer allocated, not payload, and is not read.
fn unix_socket(message: &[u8]) -> Option<UnixSocket> {
    const UNIX_DIAG_PEER: u16 = 2;
    const UNIX_DIAG_RQLEN: u16 = 4;
    const TCP_LISTEN: u8 = 10;
    let (kind, state) = (*message.get(1)?, *message.get(2)?);
    let inode = u32_at(message, 4)?;
    let mut peer = None;
    let mut rqueue = None;
    for (attribute, payload) in attributes(message, 16) {
        match attribute {
            UNIX_DIAG_PEER => peer = u32_at(payload, 0),
            UNIX_DIAG_RQLEN => rqueue = u32_at(payload, 0),
            _ => {}
        }
    }
    let queue = rqueue.filter(|_| state != TCP_LISTEN).and_then(|rqueue| {
        let rqueue = u64::from(rqueue);
        match i32::from(kind) {
            libc::SOCK_STREAM | libc::SOCK_SEQPACKET => Some(Queue {
                unread: rqueue,
                datagram_waiting: false,
            }),
            libc::SOCK_DGRAM => Some(Queue {
                unread: 0,
                datagram_waiting: rqueue > 0,
            }),
            _ => None,
        }
    });
    Some(UnixSocket {
        inode: u64::from(inode),
        peer: u64::from(peer?),
        queue,
    })
}

/// What waits unread between processes, by (reader pid, writer pid): each socket's queue is
/// what its owner has yet to read from the owner of its peer, and the sockets of one pair add
/// up. A socket whose queue is unknown, or whose owner or peer's owner is not known, or is the
/// same process, adds nothing.
fn unix_queues(sockets: &[UnixSocket], owners: &HashMap<u64, u32>) -> HashMap<(u32, u32), Queue> {
    let mut queues: HashMap<(u32, u32), Queue> = HashMap::new();
    for socket in sockets {
        if let Some(queue) = socket.queue
            && let (Some(&reader), Some(&writer)) =
                (owners.get(&socket.inode), owners.get(&socket.peer))
            && reader != writer
        {
            queues.entry((reader, writer)).or_default().add(queue);
        }
    }
    queues
}

/// Established TCP sockets of one address family, with their kernel tcp_info statistics.
fn tcp_sockets(family: u8) -> io::Result<Vec<Remote>> {
    const INET_DIAG_INFO: u8 = 2;
    const TCP_ESTABLISHED: u32 = 1;
    let mut request = [0_u8; 56];
    request[0] = family;
    request[1] = libc::IPPROTO_TCP as u8;
    request[2] = 1 << (INET_DIAG_INFO - 1);
    request[4..8].copy_from_slice(&(1_u32 << TCP_ESTABLISHED).to_ne_bytes());
    let mut sockets = Vec::new();
    diag(&request, |message| sockets.extend(tcp_socket(message)))?;
    Ok(sockets)
}

/// inet_diag_msg: family, state, timer, retrans; the socket id (ports, addresses, interface,
/// cookie) at 4..52; expires, queues and uid; the inode at 68; then attributes from 72.
fn tcp_socket(message: &[u8]) -> Option<Remote> {
    const INET_DIAG_INFO: u16 = 2;
    let family = *message.first()? as i32;
    let port = u16::from_be_bytes(message.get(6..8)?.try_into().ok()?);
    let raw: [u8; 16] = message.get(24..40)?.try_into().ok()?;
    let address = if family == libc::AF_INET {
        IpAddr::V4(Ipv4Addr::new(raw[0], raw[1], raw[2], raw[3]))
    } else {
        let v6 = Ipv6Addr::from(raw);
        v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4)
    };
    let inode = u32_at(message, 68)? as u64;
    // tcp_info: rtt (microseconds) at 68, bytes_acked at 120 and bytes_received at 128.
    // A socket without readable statistics is left out rather than shown with zero counters,
    // which the next scan would turn into a huge rate.
    let info = attributes(message, 72)
        .find(|&(kind, _)| kind == INET_DIAG_INFO)
        .map(|(_, payload)| payload)?;
    let (rtt, sent, received) = (u32_at(info, 68)?, u64_at(info, 120)?, u64_at(info, 128)?);
    (inode != 0).then_some(Remote {
        pid: 0,
        inode,
        address,
        port,
        rtt,
        sent,
        received,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_rows_keep_only_established_connections() {
        let row = "   3: 0100007F:1F90 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 424242 1 0000000000000000 20 4 30 10 -1";
        assert_eq!(
            parse_tcp(row),
            Some(Tcp {
                local: "0100007F:1F90".into(),
                remote: "0100007F:C350".into(),
                inode: 424242,
            })
        );
        let listening = row.replacen(" 01 ", " 0A ", 1);
        assert_eq!(parse_tcp(&listening), None);
    }

    #[test]
    fn unix_diag_messages_yield_peer_pairs() {
        let mut datagram = Vec::new();
        let mut message = vec![0_u8; 16];
        message[0] = libc::AF_UNIX as u8;
        message[4..8].copy_from_slice(&7_u32.to_ne_bytes());
        message.extend_from_slice(&8_u16.to_ne_bytes());
        message.extend_from_slice(&2_u16.to_ne_bytes());
        message.extend_from_slice(&9_u32.to_ne_bytes());
        datagram.extend_from_slice(&(16 + message.len() as u32).to_ne_bytes());
        datagram.extend_from_slice(&20_u16.to_ne_bytes());
        datagram.extend_from_slice(&[0; 10]);
        datagram.extend_from_slice(&message);
        let mut done = vec![0_u8; 16];
        done[0..4].copy_from_slice(&16_u32.to_ne_bytes());
        done[4..6].copy_from_slice(&(libc::NLMSG_DONE as u16).to_ne_bytes());
        let mut peers = Vec::new();
        let mut collect = |m: &[u8]| peers.extend(unix_socket(m));
        assert_eq!(messages(&datagram, &mut collect), Some(false));
        assert_eq!(messages(&done, &mut collect), Some(true));
        assert_eq!(
            peers,
            vec![UnixSocket {
                inode: 7,
                peer: 9,
                queue: None,
            }],
            "no queue attribute, no queue"
        );
    }

    /// A unix_diag_msg of `kind` and `state` for socket `inode`, with a peer attribute and, when
    /// `rqlen` is given, a `UNIX_DIAG_RQLEN` of (rqueue, wqueue).
    fn unix_message(
        kind: i32,
        state: u8,
        inode: u32,
        peer: u32,
        rqlen: Option<(u32, u32)>,
    ) -> Vec<u8> {
        let mut message = vec![0_u8; 16];
        message[0] = libc::AF_UNIX as u8;
        message[1] = kind as u8;
        message[2] = state;
        message[4..8].copy_from_slice(&inode.to_ne_bytes());
        message.extend_from_slice(&8_u16.to_ne_bytes());
        message.extend_from_slice(&2_u16.to_ne_bytes());
        message.extend_from_slice(&peer.to_ne_bytes());
        if let Some((rqueue, wqueue)) = rqlen {
            message.extend_from_slice(&12_u16.to_ne_bytes());
            message.extend_from_slice(&4_u16.to_ne_bytes());
            message.extend_from_slice(&rqueue.to_ne_bytes());
            message.extend_from_slice(&wqueue.to_ne_bytes());
        }
        message
    }

    const ESTABLISHED: u8 = 1;
    const LISTENING: u8 = 10;

    #[test]
    fn unix_diag_messages_yield_receive_queues_for_stream_sockets() {
        let sockets: Vec<UnixSocket> = [
            // Stream and seqpacket: rqueue is the unread bytes, wqueue is not read.
            unix_message(libc::SOCK_STREAM, ESTABLISHED, 7, 9, Some((4096, 212_992))),
            unix_message(libc::SOCK_SEQPACKET, ESTABLISHED, 9, 7, Some((300, 0))),
            unix_message(libc::SOCK_STREAM, ESTABLISHED, 11, 13, Some((0, 0))),
            // The kernel did not report a length, so none is invented.
            unix_message(libc::SOCK_STREAM, ESTABLISHED, 13, 11, None),
        ]
        .iter()
        .filter_map(|message| unix_socket(message))
        .collect();
        let queue = |unread| {
            Some(Queue {
                unread,
                datagram_waiting: false,
            })
        };
        assert_eq!(
            sockets
                .iter()
                .map(|socket| socket.queue)
                .collect::<Vec<_>>(),
            [queue(4096), queue(300), queue(0), None]
        );
        // Socket 7 belongs to pid 100 and its peer 9 to pid 200, so pid 100 has 4096 bytes to read
        // from pid 200 and pid 200 has 300 to read from pid 100: sockets 11 and 13 are both pid
        // 100's, and a process is no link to itself.
        let owners = HashMap::from([(7, 100), (9, 200), (11, 100), (13, 100)]);
        assert_eq!(
            unix_queues(&sockets, &owners),
            HashMap::from([
                (
                    (100, 200),
                    Queue {
                        unread: 4096,
                        datagram_waiting: false
                    }
                ),
                (
                    (200, 100),
                    Queue {
                        unread: 300,
                        datagram_waiting: false
                    }
                ),
            ])
        );
        // Two sockets of one pair add up.
        let more = [
            unix_socket(&unix_message(
                libc::SOCK_STREAM,
                ESTABLISHED,
                21,
                23,
                Some((10, 0)),
            ))
            .unwrap(),
            unix_socket(&unix_message(
                libc::SOCK_STREAM,
                ESTABLISHED,
                25,
                27,
                Some((32, 0)),
            ))
            .unwrap(),
        ];
        let owners = HashMap::from([(21, 1), (23, 2), (25, 1), (27, 2)]);
        assert_eq!(
            unix_queues(&more, &owners)[&(1, 2)],
            Queue {
                unread: 42,
                datagram_waiting: false
            }
        );
    }

    #[test]
    fn the_kernel_reports_stream_bytes_and_a_waiting_datagram_for_real_sockets() {
        use std::io::Write;
        use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(&[7; 3000]).unwrap();
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        sender.send(&[1; 100]).unwrap();
        let path = std::env::temp_dir().join(format!("isotop-queue-{}.sock", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let _waiting = UnixStream::connect(&path).unwrap();
        let sockets = match unix_peers() {
            Ok(sockets) => sockets,
            // qemu-user, which runs the x86_64 test binaries on a development machine, has no
            // sock_diag; a kernel without it has no links either.
            Err(error) if error.raw_os_error() == Some(libc::EPROTONOSUPPORT) => {
                eprintln!("skipped: no sock_diag here ({error})");
                return;
            }
            Err(error) => panic!("sock_diag failed: {error}"),
        };
        let queue = |fd: i32| {
            // SAFETY: an all-zero stat is valid, and fstat fills it for the open descriptor.
            let inode = unsafe {
                let mut status: libc::stat = std::mem::zeroed();
                assert_eq!(libc::fstat(fd, &mut status), 0);
                status.st_ino
            };
            sockets.iter().find(|socket| socket.inode == inode)
        };
        let bytes = |unread| {
            Some(Queue {
                unread,
                datagram_waiting: false,
            })
        };
        assert_eq!(queue(reader.as_raw_fd()).unwrap().queue, bytes(3000));
        assert_eq!(queue(writer.as_raw_fd()).unwrap().queue, bytes(0));
        assert_eq!(
            queue(receiver.as_raw_fd()).unwrap().queue,
            Some(Queue {
                unread: 0,
                datagram_waiting: true
            }),
            "100 bytes are queued, and the kernel says only that a packet waits"
        );
        assert_eq!(
            queue(sender.as_raw_fd()).unwrap().queue,
            Some(Queue::default())
        );
        assert!(
            queue(listener.as_raw_fd()).is_none(),
            "a listener has no peer, so it is never a pair"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn listening_sockets_are_not_reported_as_queues() {
        // A listener's rqueue is its backlog of connections to accept, not bytes.
        let listener = unix_message(libc::SOCK_STREAM, LISTENING, 7, 9, Some((3, 128)));
        let socket = unix_socket(&listener).unwrap();
        assert_eq!(socket.queue, None);
        let owners = HashMap::from([(7, 100), (9, 200)]);
        assert!(unix_queues(&[socket], &owners).is_empty());
    }

    #[test]
    fn a_datagram_queue_reports_a_waiting_packet_not_a_byte_count() {
        // For a datagram socket rqueue is the size of the first packet: 1500 here says that a
        // datagram waits, not that 1500 bytes do.
        let waiting = unix_socket(&unix_message(
            libc::SOCK_DGRAM,
            ESTABLISHED,
            7,
            9,
            Some((1500, 0)),
        ))
        .unwrap();
        let empty = unix_socket(&unix_message(
            libc::SOCK_DGRAM,
            ESTABLISHED,
            9,
            7,
            Some((0, 0)),
        ))
        .unwrap();
        assert_eq!(
            waiting.queue,
            Some(Queue {
                unread: 0,
                datagram_waiting: true
            })
        );
        assert_eq!(empty.queue, Some(Queue::default()));
        let owners = HashMap::from([(7, 100), (9, 200)]);
        let queues = unix_queues(&[waiting, empty], &owners);
        assert_eq!(
            queues[&(100, 200)],
            Queue {
                unread: 0,
                datagram_waiting: true
            }
        );
        assert_eq!(
            queues[&(200, 100)],
            Queue::default(),
            "measured idle, not absent"
        );
    }

    #[test]
    fn tcp_diag_messages_carry_address_rtt_and_bytes() {
        let mut message = vec![0_u8; 72];
        message[0] = libc::AF_INET as u8;
        message[6..8].copy_from_slice(&443_u16.to_be_bytes());
        message[24..28].copy_from_slice(&[93, 184, 216, 34]);
        message[68..72].copy_from_slice(&77_u32.to_ne_bytes());
        let mut info = vec![0_u8; 136];
        info[68..72].copy_from_slice(&25_000_u32.to_ne_bytes());
        info[120..128].copy_from_slice(&1000_u64.to_ne_bytes());
        info[128..136].copy_from_slice(&5000_u64.to_ne_bytes());
        message.extend_from_slice(&(4 + info.len() as u16).to_ne_bytes());
        message.extend_from_slice(&2_u16.to_ne_bytes());
        message.extend_from_slice(&info);
        let remote = tcp_socket(&message).unwrap();
        assert_eq!(remote.address, IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)));
        assert_eq!((remote.port, remote.inode, remote.rtt), (443, 77, 25_000));
        assert_eq!((remote.sent, remote.received), (1000, 5000));
    }

    #[test]
    fn a_tcp_socket_without_readable_statistics_is_left_out_not_counted_as_zero() {
        let mut message = vec![0_u8; 72];
        message[0] = libc::AF_INET as u8;
        message[68..72].copy_from_slice(&77_u32.to_ne_bytes());
        assert_eq!(tcp_socket(&message), None, "no tcp_info attribute");
        // A tcp_info from a kernel too old to count bytes ends before the byte counters.
        let info = vec![0_u8; 100];
        message.extend_from_slice(&(4 + info.len() as u16).to_ne_bytes());
        message.extend_from_slice(&2_u16.to_ne_bytes());
        message.extend_from_slice(&info);
        assert_eq!(tcp_socket(&message), None);
    }
}
