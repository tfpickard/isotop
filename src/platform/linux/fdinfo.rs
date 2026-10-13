//! `/proc/<pid>/fdinfo/<fd>`, the kernel's account of one open file description: its offset and
//! the flags it was opened with. The pipe scan reads the access mode from it; per-file I/O
//! attribution (B8, #35) reads the offset.

use crate::platform::AccessMode;

/// The fields of an fdinfo file that every kind of descriptor has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FdInfo {
    /// The `flags:` line, which the kernel prints in octal: the `O_*` flags of the open file
    /// description, with `O_CLOEXEC` and the architecture's `O_LARGEFILE` among them.
    pub flags: u32,
    /// The `pos:` line, the file offset in bytes.
    pub pos: u64,
}

impl FdInfo {
    /// What the descriptor may do, from `flags & O_ACCMODE`: 0 reads, 1 writes, 2 does both. 3
    /// is not an access mode, so it gives None.
    pub fn access(self) -> Option<AccessMode> {
        match self.flags & libc::O_ACCMODE as u32 {
            0 => Some(AccessMode::Read),
            1 => Some(AccessMode::Write),
            2 => Some(AccessMode::Both),
            _ => None,
        }
    }
}

/// Parses the text of an fdinfo file (`pos:\t0`, `flags:\t02100002`, `mnt_id:` and so on). None
/// when it has no `flags:` or no `pos:` line, or either does not parse: a descriptor that closed
/// while it was read, or a kernel that formats it otherwise.
pub fn parse_fdinfo(text: &str) -> Option<FdInfo> {
    let (mut flags, mut pos) = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key {
            "flags" => flags = u32::from_str_radix(value.trim(), 8).ok(),
            "pos" => pos = value.trim().parse().ok(),
            _ => {}
        }
    }
    Some(FdInfo {
        flags: flags?,
        pos: pos?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(flags: &str) -> FdInfo {
        parse_fdinfo(&format!(
            "pos:\t0\nflags:\t{flags}\nmnt_id:\t15\nino:\t4242\n"
        ))
        .unwrap()
    }

    #[test]
    fn fdinfo_flags_give_pipe_direction() {
        // The flags are octal: 02100002 is O_RDWR | O_LARGEFILE | O_CLOEXEC on x86_64.
        let cases = [
            ("00", Some(AccessMode::Read)),
            ("02000000", Some(AccessMode::Read)),
            ("01", Some(AccessMode::Write)),
            ("0100001", Some(AccessMode::Write)),
            ("02100002", Some(AccessMode::Both)),
            ("02", Some(AccessMode::Both)),
            ("03", None),
        ];
        for (flags, access) in cases {
            assert_eq!(info(flags).access(), access, "flags {flags}");
        }
        assert_eq!(info("02100002").flags, 0o2100002);
        assert_eq!(info("02100002").pos, 0);
    }

    #[test]
    fn fdinfo_without_flags_or_offset_is_not_guessed() {
        assert_eq!(parse_fdinfo("pos:\t0\nmnt_id:\t15\n"), None);
        assert_eq!(parse_fdinfo("flags:\t02\n"), None);
        assert_eq!(
            parse_fdinfo("pos:\t0\nflags:\t09\n"),
            None,
            "9 is not octal"
        );
        assert_eq!(parse_fdinfo(""), None);
        let sought = parse_fdinfo("pos:\t8192\nflags:\t0100000\n").unwrap();
        assert_eq!(sought.pos, 8192);
    }
}
