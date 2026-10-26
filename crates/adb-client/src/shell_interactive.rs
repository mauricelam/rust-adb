use crate::client::{ClientError, ShellOptions};

/// Mode for PTY allocation requested by client options (`-t`, `-tt`, `-T`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PtyMode {
    #[default]
    Auto,
    No,
    Yes,
    Definitely,
}

/// Result of PTY allocation decision logic matching C++ `commandline.cpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyDecision {
    /// Service type argument ("pty", "raw", or "" for v1).
    pub type_arg: &'static str,
    /// Warning message to print to stderr, if any.
    pub warning: Option<String>,
    /// Whether local stdin should be set to raw mode.
    pub raw_stdin: bool,
    /// Whether Shell v2 protocol should be used.
    pub use_shell_protocol: bool,
}

/// Parses the `-e` argument string into an optional escape character.
///
/// Accepts a single character or `'none'`. Returns error on any other string.
pub fn parse_escape_char_arg(s: &str) -> Result<Option<char>, ClientError> {
    if s == "none" {
        Ok(None)
    } else if s.chars().count() == 1 {
        Ok(Some(s.chars().next().unwrap()))
    } else {
        Err(ClientError::QueryFailed(
            "-e requires a single-character argument or 'none'".to_string(),
        ))
    }
}

/// Decides PTY allocation, service type argument, local raw stdin, and warnings.
///
/// Ports logic from C++ `RemoteShell` in `original/client/commandline.cpp`.
pub fn determine_pty_allocation(
    options: &ShellOptions,
    is_interactive: bool,
    stdin_isatty: bool,
    device_supports_shell2: bool,
) -> Result<PtyDecision, ClientError> {
    let use_shell_protocol = device_supports_shell2 && !options.disable_shell_protocol;

    let effective_pty_mode = if options.no_pty {
        PtyMode::No
    } else {
        options.pty_mode
    };

    let mut warning = None;

    let requested_type = match effective_pty_mode {
        PtyMode::No => "raw",
        PtyMode::Auto => {
            if !stdin_isatty || !is_interactive {
                "raw"
            } else {
                "pty"
            }
        }
        PtyMode::Yes => {
            if !stdin_isatty {
                warning = Some(
                    "Remote PTY will not be allocated because stdin is not a terminal.\n\
                     Use multiple -t options to force remote PTY allocation.\n"
                        .to_string(),
                );
                "raw"
            } else {
                "pty"
            }
        }
        PtyMode::Definitely => "pty",
    };

    let type_arg = if !use_shell_protocol {
        if requested_type != "pty" {
            if !device_supports_shell2 {
                return Err(ClientError::QueryFailed(
                    "device only supports allocating a pty".to_string(),
                ));
            } else if options.disable_shell_protocol {
                return Err(ClientError::QueryFailed(
                    "-x only supports allocating a pty".to_string(),
                ));
            }
        }
        ""
    } else {
        requested_type
    };

    let raw_stdin = type_arg == "pty" || (type_arg.is_empty() && is_interactive);

    Ok(PtyDecision {
        type_arg,
        warning,
        raw_stdin,
        use_shell_protocol,
    })
}

/// Formats window size for `ShellPacketId::WindowSizeChange`.
///
/// Text format: `<rows>x<cols>,<xpixels>x<ypixels>\0` matching C++ `send_window_size_change`.
pub fn format_window_size(rows: u16, cols: u16, xpixel: u16, ypixel: u16) -> Vec<u8> {
    format!("{rows}x{cols},{xpixel}x{ypixel}\0").into_bytes()
}

/// Guard object that sets local terminal to raw mode and restores original attributes on drop.
#[cfg(unix)]
pub struct RawTerminalGuard {
    fd: std::os::fd::RawFd,
    orig_termios: libc::termios,
    active: bool,
}

#[cfg(unix)]
impl RawTerminalGuard {
    pub fn new(fd: std::os::fd::RawFd) -> Option<Self> {
        if unsafe { libc::isatty(fd) } == 0 {
            return None;
        }
        let mut orig_termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut orig_termios) } != 0 {
            return None;
        }
        let mut raw_termios = orig_termios;
        unsafe { libc::cfmakeraw(&mut raw_termios) };
        raw_termios.c_cc[libc::VTIME] = 0;
        raw_termios.c_cc[libc::VMIN] = 1;

        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw_termios) } != 0 {
            return None;
        }

        Some(Self {
            fd,
            orig_termios,
            active: true,
        })
    }
}

#[cfg(unix)]
impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.orig_termios);
            }
        }
    }
}

#[cfg(not(unix))]
pub struct RawTerminalGuard;

#[cfg(not(unix))]
impl RawTerminalGuard {
    pub fn new(_fd: i32) -> Option<Self> {
        None
    }
}

/// Reads current terminal window size using `TIOCGWINSZ`.
#[cfg(unix)]
pub fn get_window_size(fd: std::os::fd::RawFd) -> Option<(u16, u16, u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 {
        Some((ws.ws_row, ws.ws_col, ws.ws_xpixel, ws.ws_ypixel))
    } else {
        None
    }
}

#[cfg(not(unix))]
pub fn get_window_size(_fd: i32) -> Option<(u16, u16, u16, u16)> {
    None
}

/// Inspects reader to find underlying TTY file descriptor if available.
#[cfg(unix)]
pub fn find_tty_fd<R: 'static>(reader: &R) -> Option<std::os::fd::RawFd> {
    use std::any::Any;
    use std::os::fd::AsRawFd;

    let any = reader as &dyn Any;

    if let Some(f) = any.downcast_ref::<tokio::fs::File>() {
        let fd = f.as_raw_fd();
        return if unsafe { libc::isatty(fd) } != 0 {
            Some(fd)
        } else {
            None
        };
    }

    if let Some(f) = any.downcast_ref::<std::fs::File>() {
        let fd = f.as_raw_fd();
        return if unsafe { libc::isatty(fd) } != 0 {
            Some(fd)
        } else {
            None
        };
    }

    if any.is::<tokio::io::Stdin>() || any.is::<std::io::Stdin>() {
        return if unsafe { libc::isatty(libc::STDIN_FILENO) } != 0 {
            Some(libc::STDIN_FILENO)
        } else {
            None
        };
    }

    None
}

#[cfg(not(unix))]
pub fn find_tty_fd<R: 'static>(_reader: &R) -> Option<i32> {
    None
}

/// Escape character state machine state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EscapeState {
    #[default]
    StartOfLine,
    MidFlow,
    InEscape,
}

/// Outcome of processing input bytes through the escape state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeResult {
    /// Bytes to emit to the remote shell.
    Output(Vec<u8>),
    /// `~.` disconnect sequence received at start of line.
    Disconnect,
}

/// Escape sequence state machine implementation.
///
/// Ports escape sequence state machine from C++ `stdin_read_thread_loop` in `original/client/commandline.cpp`.
#[derive(Debug, Clone)]
pub struct EscapeStateMachine {
    escape_char: Option<u8>,
    state: EscapeState,
}

impl EscapeStateMachine {
    pub fn new(escape_char: Option<char>) -> Self {
        Self {
            escape_char: escape_char.map(|c| c as u8),
            state: EscapeState::StartOfLine,
        }
    }

    pub fn state(&self) -> EscapeState {
        self.state
    }

    pub fn process_byte(&mut self, b: u8) -> EscapeResult {
        let esc = match self.escape_char {
            Some(e) => e,
            None => return EscapeResult::Output(vec![b]),
        };

        if b == esc {
            if self.state == EscapeState::StartOfLine {
                self.state = EscapeState::InEscape;
                EscapeResult::Output(vec![])
            } else {
                self.state = EscapeState::MidFlow;
                EscapeResult::Output(vec![b])
            }
        } else {
            if self.state == EscapeState::InEscape {
                if b == b'.' {
                    return EscapeResult::Disconnect;
                } else {
                    let out = vec![esc, b];
                    self.state = if b == b'\n' || b == b'\r' {
                        EscapeState::StartOfLine
                    } else {
                        EscapeState::MidFlow
                    };
                    return EscapeResult::Output(out);
                }
            }
            self.state = if b == b'\n' || b == b'\r' {
                EscapeState::StartOfLine
            } else {
                EscapeState::MidFlow
            };
            EscapeResult::Output(vec![b])
        }
    }

    pub fn process_chunk(&mut self, input: &[u8]) -> EscapeResult {
        let mut out = Vec::new();
        for &b in input {
            match self.process_byte(b) {
                EscapeResult::Output(bytes) => out.extend(bytes),
                EscapeResult::Disconnect => return EscapeResult::Disconnect,
            }
        }
        EscapeResult::Output(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_escape_char_arg() {
        assert_eq!(parse_escape_char_arg("none").unwrap(), None);
        assert_eq!(parse_escape_char_arg("~").unwrap(), Some('~'));
        assert_eq!(parse_escape_char_arg("@").unwrap(), Some('@'));
        assert!(parse_escape_char_arg("invalid").is_err());
        assert!(parse_escape_char_arg("").is_err());
    }

    #[test]
    fn test_format_window_size() {
        let bytes = format_window_size(24, 80, 0, 0);
        assert_eq!(bytes, b"24x80,0x0\0");

        let bytes_pixels = format_window_size(40, 120, 800, 600);
        assert_eq!(bytes_pixels, b"40x120,800x600\0");
    }

    #[test]
    fn test_pty_selection_table() {
        // Auto, interactive, tty, shell2 -> pty, v2
        let opt = ShellOptions::default();
        let dec = determine_pty_allocation(&opt, true, true, true).unwrap();
        assert_eq!(dec.type_arg, "pty");
        assert_eq!(dec.warning, None);
        assert!(dec.raw_stdin);
        assert!(dec.use_shell_protocol);

        // Auto, non-interactive, tty, shell2 -> raw, v2
        let dec = determine_pty_allocation(&opt, false, true, true).unwrap();
        assert_eq!(dec.type_arg, "raw");
        assert_eq!(dec.warning, None);
        assert!(!dec.raw_stdin);

        // Auto, interactive, non-tty, shell2 -> raw, v2
        let dec = determine_pty_allocation(&opt, true, false, true).unwrap();
        assert_eq!(dec.type_arg, "raw");
        assert_eq!(dec.warning, None);
        assert!(!dec.raw_stdin);

        // Yes (-t), interactive, non-tty, shell2 -> raw, warning printed
        let opt_t = ShellOptions {
            pty_mode: PtyMode::Yes,
            ..Default::default()
        };
        let dec = determine_pty_allocation(&opt_t, true, false, true).unwrap();
        assert_eq!(dec.type_arg, "raw");
        assert!(dec
            .warning
            .as_ref()
            .unwrap()
            .contains("Remote PTY will not be allocated"));
        assert!(!dec.raw_stdin);

        // Definitely (-tt), interactive, non-tty, shell2 -> pty, no warning
        let opt_tt = ShellOptions {
            pty_mode: PtyMode::Definitely,
            ..Default::default()
        };
        let dec = determine_pty_allocation(&opt_tt, true, false, true).unwrap();
        assert_eq!(dec.type_arg, "pty");
        assert_eq!(dec.warning, None);
        assert!(dec.raw_stdin);

        // No (-T), interactive, tty, shell2 -> raw
        let opt_no = ShellOptions {
            pty_mode: PtyMode::No,
            ..Default::default()
        };
        let dec = determine_pty_allocation(&opt_no, true, true, true).unwrap();
        assert_eq!(dec.type_arg, "raw");
        assert_eq!(dec.warning, None);
        assert!(!dec.raw_stdin);

        // no_pty field set -> raw
        let opt_no_pty = ShellOptions {
            no_pty: true,
            ..Default::default()
        };
        let dec = determine_pty_allocation(&opt_no_pty, true, true, true).unwrap();
        assert_eq!(dec.type_arg, "raw");

        // v1 (no shell2 on device), Auto, interactive, tty -> type_arg = "", raw_stdin = true
        let dec = determine_pty_allocation(&opt, true, true, false).unwrap();
        assert_eq!(dec.type_arg, "");
        assert!(dec.raw_stdin);
        assert!(!dec.use_shell_protocol);

        // v1 (no shell2 on device), non-interactive (requests raw) -> device only supports allocating a pty
        let err = determine_pty_allocation(&opt, false, true, false).unwrap_err();
        assert_eq!(err.to_string(), "device only supports allocating a pty");

        // -x (disable shell protocol), non-interactive -> -x only supports allocating a pty
        let opt_x = ShellOptions {
            disable_shell_protocol: true,
            ..Default::default()
        };
        let err = determine_pty_allocation(&opt_x, false, true, true).unwrap_err();
        assert_eq!(err.to_string(), "-x only supports allocating a pty");
    }

    #[test]
    fn test_escape_state_machine_transitions() {
        let mut sm = EscapeStateMachine::new(Some('~'));

        // StartOfLine swallows ~
        assert_eq!(sm.process_byte(b'~'), EscapeResult::Output(vec![]));
        assert_eq!(sm.state(), EscapeState::InEscape);

        // InEscape followed by . disconnects
        assert_eq!(sm.process_byte(b'.'), EscapeResult::Disconnect);

        // Reset state machine
        let mut sm = EscapeStateMachine::new(Some('~'));
        assert_eq!(sm.process_byte(b'~'), EscapeResult::Output(vec![]));
        // InEscape followed by ~ emits ~ (since ~ escapes ~) and state becomes MidFlow
        assert_eq!(sm.process_byte(b'~'), EscapeResult::Output(vec![b'~']));
        assert_eq!(sm.state(), EscapeState::MidFlow);

        // MidFlow ~ emits ~ and stays MidFlow
        assert_eq!(sm.process_byte(b'~'), EscapeResult::Output(vec![b'~']));
        assert_eq!(sm.state(), EscapeState::MidFlow);

        // In Escape followed by x emits ~x
        let mut sm = EscapeStateMachine::new(Some('~'));
        assert_eq!(sm.process_byte(b'~'), EscapeResult::Output(vec![]));
        assert_eq!(
            sm.process_byte(b'x'),
            EscapeResult::Output(vec![b'~', b'x'])
        );
        assert_eq!(sm.state(), EscapeState::MidFlow);

        // Newline puts state back to StartOfLine
        assert_eq!(sm.process_byte(b'\n'), EscapeResult::Output(vec![b'\n']));
        assert_eq!(sm.state(), EscapeState::StartOfLine);

        // Now ~ at StartOfLine is swallowed again
        assert_eq!(sm.process_byte(b'~'), EscapeResult::Output(vec![]));
        assert_eq!(sm.state(), EscapeState::InEscape);
        assert_eq!(sm.process_byte(b'.'), EscapeResult::Disconnect);

        // Carriage return \r puts state back to StartOfLine
        let mut sm = EscapeStateMachine::new(Some('~'));
        assert_eq!(sm.process_chunk(b"foo\r~."), EscapeResult::Disconnect);

        // MidLine ~ followed by . does NOT disconnect
        let mut sm = EscapeStateMachine::new(Some('~'));
        let res = sm.process_chunk(b"hello ~.");
        assert_eq!(res, EscapeResult::Output(b"hello ~.".to_vec()));

        // Disabled escape character
        let mut sm_none = EscapeStateMachine::new(None);
        assert_eq!(
            sm_none.process_chunk(b"~."),
            EscapeResult::Output(b"~.".to_vec())
        );
    }
}
