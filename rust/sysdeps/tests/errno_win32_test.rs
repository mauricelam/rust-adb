//! Windows-specific errno tests
//! Ported from `original/sysdeps/win32/errno_test.cpp`.

#![cfg(windows)]

use sysdeps::errno::{errno_to_wire, errno_from_wire};
use windows_sys::Win32::Networking::WinSock::*;
use windows_sys::Win32::Foundation::*;

#[test]
fn test_errno_to_wire() {
    assert_eq!(errno_to_wire(WSAEACCES as i32), 13);
    assert_eq!(errno_to_wire(ERROR_ALREADY_EXISTS as i32), 17);
    assert_eq!(errno_to_wire(WSAEFAULT as i32), 14);
    assert_eq!(errno_to_wire(WSAEINVAL as i32), 22);
    assert_eq!(errno_to_wire(ERROR_FILE_NOT_FOUND as i32), 2);
    assert_eq!(errno_to_wire(ERROR_NOT_ENOUGH_MEMORY as i32), 12);
    assert_eq!(errno_to_wire(ERROR_DISK_FULL as i32), 28);
    assert_eq!(errno_to_wire(ERROR_DIRECTORY as i32), 20);
    assert_eq!(errno_to_wire(ERROR_ACCESS_DENIED as i32), 1);
}

#[test]
fn test_errno_from_wire() {
    assert_eq!(errno_from_wire(13), WSAEACCES as i32);
    assert_eq!(errno_from_wire(17), ERROR_ALREADY_EXISTS as i32);
    assert_eq!(errno_from_wire(14), WSAEFAULT as i32);
    assert_eq!(errno_from_wire(22), WSAEINVAL as i32);
    assert_eq!(errno_from_wire(2), ERROR_FILE_NOT_FOUND as i32);
    assert_eq!(errno_from_wire(12), ERROR_NOT_ENOUGH_MEMORY as i32);
    assert_eq!(errno_from_wire(28), ERROR_DISK_FULL as i32);
    assert_eq!(errno_from_wire(20), ERROR_DIRECTORY as i32);
    assert_eq!(errno_from_wire(1), ERROR_ACCESS_DENIED as i32);
}
