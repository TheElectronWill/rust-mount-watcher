//! Parse /proc/mounts.

use std::{
    fmt::Display,
    fs::File,
    io::{Read, Seek},
};

use thiserror::Error;

pub const PROC_MOUNTS_PATH: &str = "/proc/mounts";

/// A mounted filesystem.
///
/// See `man fstab` for a detailed description of the fields.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct LinuxMount {
    /// The special device, filesystem or swap to be mounted.
    pub spec: String,
    /// The mount point for the filesystem, or `"none"` for swap.
    pub mount_point: String,
    /// The type of the filesystem.
    pub fs_type: String,
    /// Mount options associated with the filesystem.
    pub mount_options: Vec<String>,
    /// Used by `dump` to determine which filesystems need to be dumped. Defaults to zero if absent.
    pub dump_fs_freq: u32,
    /// Used by `fsck` to determine the order of the checks. Defaults to zero if absent.
    pub fsck_fs_passno: u32,
}

/// Error while parsing `/proc/mounts`.
#[derive(Debug, Error)]
#[error("invalid {invalid_part} in mount line: {input}")]
pub struct ParseError {
    pub(crate) input: String,
    pub invalid_part: InvalidPart,
}

/// Error while reading/parsing `/proc/mounts`.
#[derive(Debug, Error)]
pub enum ReadError {
    #[error("failed to parse {PROC_MOUNTS_PATH}")]
    Parse(#[from] ParseError),
    #[error("failed to read {PROC_MOUNTS_PATH}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy)]
pub enum InvalidPart {
    Spec,
    MountPoint,
    FsType,
    MountOptions,
    DumpFsFreq,
    FsckFsPassno,
}

impl Display for InvalidPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            InvalidPart::Spec => "spec (fs_spec)",
            InvalidPart::MountPoint => "mount point (fs_file)",
            InvalidPart::FsType => "fs type (fs_vfstype)",
            InvalidPart::MountOptions => "mount options (fs_mntops)",
            InvalidPart::DumpFsFreq => "dump fs freq (fs_freq)",
            InvalidPart::FsckFsPassno => "fsck fs passno (fs_passno)",
        };
        f.write_str(s)
    }
}

impl LinuxMount {
    /// Attempts to parse one line of `/proc/mounts`.
    /// Returns `None` if it fails.
    pub fn parse(line: &str) -> Result<Self, InvalidPart> {
        let mut fields = line.split_ascii_whitespace().into_iter();
        let spec = fields.next().map(decode_escapes).ok_or(InvalidPart::Spec)?;
        let mount_point = fields
            .next()
            .map(decode_escapes)
            .ok_or(InvalidPart::MountPoint)?;
        let fs_type = fields
            .next()
            .map(decode_escapes)
            .ok_or(InvalidPart::FsType)?;
        let mount_options = fields
            .next()
            .ok_or(InvalidPart::MountOptions)?
            .split(',')
            .map(decode_escapes)
            .collect();
        let dump_fs_freq = fields
            .next()
            .map(str::parse)
            .unwrap_or(Ok(0))
            .map_err(|_| InvalidPart::DumpFsFreq)?;
        let fsck_fs_passno = fields
            .next()
            .map(str::parse)
            .unwrap_or(Ok(0))
            .map_err(|_| InvalidPart::FsckFsPassno)?;
        Ok(Self {
            spec,
            mount_point,
            fs_type,
            mount_options,
            dump_fs_freq,
            fsck_fs_passno,
        })
    }
}

fn decode_escapes(field: &str) -> String {
    if !field.contains('\\') {
        return field.to_string();
    };

    // handle escape sequences
    let mut res = String::with_capacity(field.len() - 1);
    let mut chars = field.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            res.push(ch);
            continue;
        }

        match chars.next() {
            None => {
                // alone backslash, let it slip
                res.push('\\');
            }
            Some('\\') => res.push('\\'),
            Some(ch2) => {
                //  expect a sequence \xyz
                match (ch2, chars.next(), chars.next()) {
                    ('0', Some('4'), Some('0')) => res.push(' '),
                    ('0', Some('1'), Some('1')) => res.push('\t'),
                    ('0', Some('1'), Some('2')) => res.push('\n'),
                    ('1', Some('3'), Some('4')) => res.push('\\'),
                    (x, y, z) => {
                        // Unknown escape sequence.
                        // The glibc implementation accepts this, so we do the same.
                        res.push('\\');
                        res.push(x);
                        if let Some(y) = y {
                            res.push(y);
                        }
                        if let Some(z) = z {
                            res.push(z);
                        }
                    }
                };
            }
        };
    }
    res
}

/// Returns the filesystems that are currently mounted.
pub fn list_current_mounts() -> Result<Vec<LinuxMount>, ReadError> {
    let mut file = File::open(PROC_MOUNTS_PATH)?;
    read_proc_mounts(&mut file)
}

/// Reads `/proc/mounts` from the beginning and parses its content.
pub(crate) fn read_proc_mounts(file: &mut File) -> Result<Vec<LinuxMount>, ReadError> {
    let mut content = String::with_capacity(4096);
    file.rewind()?;
    file.read_to_string(&mut content)?;
    let mut mounts = Vec::with_capacity(64);
    parse_proc_mounts(&content, &mut mounts)?;
    Ok(mounts)
}

/// Parses the content of `/proc/mounts` and stores the result in `buf`.
pub(crate) fn parse_proc_mounts(
    content: &str,
    buf: &mut Vec<LinuxMount>,
) -> Result<(), ParseError> {
    for line in content.lines() {
        let line = line.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if !line.is_empty() && !line.starts_with('#') {
            let m = LinuxMount::parse(line).map_err(|invalid_part| ParseError {
                input: line.to_owned(),
                invalid_part,
            })?;
            buf.push(m);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use crate::mount::decode_escapes;

    use super::{parse_proc_mounts, LinuxMount};

    fn vec_str(values: &[&str]) -> Vec<String> {
        values.into_iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn decoding_sequences() {
        assert_eq!(
            decode_escapes("abcdef 14 ,;=0"),
            String::from("abcdef 14 ,;=0")
        );
        assert_eq!(
            decode_escapes(r#"C:\134\040test"#),
            String::from(r#"C:\ test"#)
        );
    }

    #[test]
    fn decoding_sequences_unicode() {
        assert_eq!(decode_escapes(r#"C:\134…µ\\"#), String::from(r#"C:\…µ\"#));
    }

    #[test]
    fn decoding_sequences_unknown() {
        assert_eq!(decode_escapes(r#"path=C:\;"#), String::from(r#"path=C:\;"#));
        assert_eq!(decode_escapes(r#"\abcd"#), String::from(r#"\abcd"#));
    }

    #[test]
    fn parsing() {
        let content = "
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
tmpfs /run tmpfs rw,nosuid,nodev,noexec,relatime,size=1599352k,mode=755,inode64 1 2
cgroup2 /sys/fs/cgroup cgroup2 rw,nosuid,nodev,noexec,relatime,nsdelegate,memory_recursiveprot 0 0
/dev/nvme0n1p1 /boot/efi vfat rw,relatime,errors=remount-ro 0 0";
        let mut mounts = Vec::new();
        parse_proc_mounts(&content, &mut mounts).unwrap();

        let expected = vec![
            LinuxMount {
                spec: String::from("sysfs"),
                mount_point: String::from("/sys"),
                fs_type: String::from("sysfs"),
                mount_options: vec_str(&["rw", "nosuid", "nodev", "noexec", "relatime"]),
                dump_fs_freq: 0,
                fsck_fs_passno: 0,
            },
            LinuxMount {
                spec: String::from("tmpfs"),
                mount_point: String::from("/run"),
                fs_type: String::from("tmpfs"),
                mount_options: vec_str(&[
                    "rw",
                    "nosuid",
                    "nodev",
                    "noexec",
                    "relatime",
                    "size=1599352k",
                    "mode=755",
                    "inode64",
                ]),
                dump_fs_freq: 1,
                fsck_fs_passno: 2,
            },
            LinuxMount {
                spec: String::from("cgroup2"),
                mount_point: String::from("/sys/fs/cgroup"),
                fs_type: String::from("cgroup2"),
                mount_options: vec_str(&[
                    "rw",
                    "nosuid",
                    "nodev",
                    "noexec",
                    "relatime",
                    "nsdelegate",
                    "memory_recursiveprot",
                ]),
                dump_fs_freq: 0,
                fsck_fs_passno: 0,
            },
            LinuxMount {
                spec: String::from("/dev/nvme0n1p1"),
                mount_point: String::from("/boot/efi"),
                fs_type: String::from("vfat"),
                mount_options: vec_str(&["rw", "relatime", "errors=remount-ro"]),
                dump_fs_freq: 0,
                fsck_fs_passno: 0,
            },
        ];
        assert_eq!(expected, mounts);
    }

    #[test]
    fn parsing_with_defaults() {
        let content = "
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime
tmpfs /run tmpfs rw,nosuid,nodev,noexec 1
        ";

        let mut mounts = Vec::new();
        parse_proc_mounts(&content, &mut mounts).unwrap();

        let expected = vec![
            LinuxMount {
                spec: String::from("sysfs"),
                mount_point: String::from("/sys"),
                fs_type: String::from("sysfs"),
                mount_options: vec_str(&["rw", "nosuid", "nodev", "noexec", "relatime"]),
                dump_fs_freq: 0,
                fsck_fs_passno: 0,
            },
            LinuxMount {
                spec: String::from("tmpfs"),
                mount_point: String::from("/run"),
                fs_type: String::from("tmpfs"),
                mount_options: vec_str(&["rw", "nosuid", "nodev", "noexec"]),
                dump_fs_freq: 1,
                fsck_fs_passno: 0,
            },
        ];
        assert_eq!(expected, mounts);
    }

    /// Tests parsing with escape sequences in the mount lines, like what happens in WSL.
    #[test]
    fn parsing_with_escape_sequences() {
        let content = r#"
C:\134 /mnt/c 9p rw,noatime,aname=drvfs;path=C:\;uid=1000;gid=1000;symlinkroot=/mnt/,cache=5,access=client 0 0
C:\134Program\040Files\134Docker\134Docker\134resources /Docker/host 9p rw,noatime,aname=drvfs;path=C:\134Program\040Files\134…\134resources;symlinkroot=/mnt/,cache=5 0 0
        "#;

        let mut mounts = Vec::new();
        parse_proc_mounts(&content, &mut mounts).unwrap();

        let expected = vec![
            LinuxMount {
                spec: String::from("C:\\"),
                mount_point: String::from("/mnt/c"),
                fs_type: String::from("9p"),
                mount_options: vec_str(&[
                    "rw",
                    "noatime",
                    "aname=drvfs;path=C:\\;uid=1000;gid=1000;symlinkroot=/mnt/",
                    "cache=5",
                    "access=client",
                ]),
                dump_fs_freq: 0,
                fsck_fs_passno: 0,
            },
            LinuxMount {
                spec: String::from("C:\\Program Files\\Docker\\Docker\\resources"),
                mount_point: String::from("/Docker/host"),
                fs_type: String::from("9p"),
                mount_options: vec_str(&[
                    "rw",
                    "noatime",
                    "aname=drvfs;path=C:\\Program Files\\…\\resources;symlinkroot=/mnt/",
                    "cache=5",
                ]),
                dump_fs_freq: 0,
                fsck_fs_passno: 0,
            },
        ];
        assert_eq!(expected, mounts);
    }

    #[test]
    fn parsing_error() {
        let mut mounts = Vec::new();
        parse_proc_mounts("badbad", &mut mounts).unwrap_err();
        parse_proc_mounts("croup2 /sys/fs/cgroup", &mut mounts).unwrap_err();
    }

    #[test]
    fn parsing_comments() {
        let mut mounts = Vec::new();
        parse_proc_mounts("\n# badbad\n", &mut mounts).unwrap();
    }
}
