use libc;

/// Maps a host errno value to the ADB wire protocol value.
/// Based on the Linux asm-generic values.
pub fn errno_to_wire(errno: i32) -> i32 {
    match errno {
        libc::EACCES => 13,
        libc::EEXIST => 17,
        libc::EFAULT => 14,
        libc::EFBIG => 27,
        libc::EINTR => 4,
        libc::EINVAL => 22,
        libc::EIO => 5,
        libc::EISDIR => 21,
        libc::ELOOP => 40,
        libc::EMFILE => 24,
        libc::ENAMETOOLONG => 36,
        libc::ENFILE => 23,
        libc::ENOENT => 2,
        libc::ENOMEM => 12,
        libc::ENOSPC => 28,
        libc::ENOTDIR => 20,
        libc::EOVERFLOW => 75,
        libc::EPERM => 1,
        libc::EROFS => 30,
        libc::ETXTBSY => 26,
        _ => panic!("failed to convert errno {} to wire", errno),
    }
}

/// Maps an ADB wire protocol errno value to the host errno value.
pub fn errno_from_wire(wire_errno: i32) -> i32 {
    match wire_errno {
        13 => libc::EACCES,
        17 => libc::EEXIST,
        14 => libc::EFAULT,
        27 => libc::EFBIG,
        4 => libc::EINTR,
        22 => libc::EINVAL,
        5 => libc::EIO,
        21 => libc::EISDIR,
        40 => libc::ELOOP,
        24 => libc::EMFILE,
        36 => libc::ENAMETOOLONG,
        23 => libc::ENFILE,
        2 => libc::ENOENT,
        12 => libc::ENOMEM,
        28 => libc::ENOSPC,
        20 => libc::ENOTDIR,
        75 => libc::EOVERFLOW,
        1 => libc::EPERM,
        30 => libc::EROFS,
        26 => libc::ETXTBSY,
        _ => panic!("failed to convert wire errno {} to host", wire_errno),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libc;

    #[test]
    fn test_errno_to_wire() {
        assert_eq!(errno_to_wire(libc::EACCES), 13);
        assert_eq!(errno_to_wire(libc::ENOENT), 2);
        assert_eq!(errno_to_wire(libc::EIO), 5);
    }

    #[test]
    #[should_panic(expected = "failed to convert errno -1 to wire")]
    fn test_errno_to_wire_panic() {
        errno_to_wire(-1);
    }

    #[test]
    fn test_errno_from_wire() {
        assert_eq!(errno_from_wire(13), libc::EACCES);
        assert_eq!(errno_from_wire(2), libc::ENOENT);
        assert_eq!(errno_from_wire(5), libc::EIO);
    }

    #[test]
    #[should_panic(expected = "failed to convert wire errno -1 to host")]
    fn test_errno_from_wire_panic() {
        errno_from_wire(-1);
    }
}
