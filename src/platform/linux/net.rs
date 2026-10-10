//! Socket links between processes: loopback TCP pairs, Unix-socket peers, and TCP connections
//! that leave the machine. Only sockets of processes whose file descriptors we may read are seen.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

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
    for (inode, peer) in unix_peers().unwrap_or_default() {
        if inode < peer
            && let (Some(&a), Some(&b)) = (owners.get(&inode), owners.get(&peer))
        {
            link(a, b);
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
fn attributes(message: &[u8], start: usize) -> impl Iterator<Item = (u16, &[u8])> {
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

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Connected Unix-socket (inode, peer inode) pairs.
fn unix_peers() -> io::Result<Vec<(u64, u64)>> {
    const UDIAG_SHOW_PEER: u32 = 0x4;
    let mut request = [0_u8; 24];
    request[0] = libc::AF_UNIX as u8;
    request[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
    request[12..16].copy_from_slice(&UDIAG_SHOW_PEER.to_ne_bytes());
    let mut peers = Vec::new();
    diag(&request, |message| peers.extend(unix_peer(message)))?;
    Ok(peers)
}

/// unix_diag_msg: family, type, state, pad, inode (u32), cookie (2 x u32); then attributes.
fn unix_peer(message: &[u8]) -> Option<(u64, u64)> {
    const UNIX_DIAG_PEER: u16 = 2;
    let inode = u32_at(message, 4)?;
    let peer = attributes(message, 16)
        .find(|&(kind, _)| kind == UNIX_DIAG_PEER)
        .and_then(|(_, payload)| u32_at(payload, 0))?;
    Some((inode as u64, peer as u64))
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
    let info = attributes(message, 72)
        .find(|&(kind, _)| kind == INET_DIAG_INFO)
        .map(|(_, payload)| payload);
    let (rtt, sent, received) = info.map_or((0, 0, 0), |info| {
        (
            u32_at(info, 68).unwrap_or(0),
            u64_at(info, 120).unwrap_or(0),
            u64_at(info, 128).unwrap_or(0),
        )
    });
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
        let mut collect = |m: &[u8]| peers.extend(unix_peer(m));
        assert_eq!(messages(&datagram, &mut collect), Some(false));
        assert_eq!(messages(&done, &mut collect), Some(true));
        assert_eq!(peers, vec![(7, 9)]);
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
}
