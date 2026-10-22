use crate::transport::TransportSelector;

/// Selection errors during device option argument parsing.
///
/// Ports option parsing errors from C++ `commandline.cpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    SRequiresArgument,
    TRequiresArgument,
    InvalidTransportId,
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionError::SRequiresArgument => write!(f, "-s requires an argument"),
            SelectionError::TRequiresArgument => write!(f, "-t requires an argument"),
            SelectionError::InvalidTransportId => write!(f, "invalid transport id"),
        }
    }
}

impl std::error::Error for SelectionError {}

/// Represents parsed device selection options (`-s`, `-t`, `-d`, `-e`, and `ANDROID_SERIAL` fallback).
///
/// Ports device selection and option parsing logic from `original/client/commandline.cpp`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceSelection {
    pub serial: Option<String>,
    pub transport_id: Option<u64>,
    pub usb: bool,
    pub local: bool,
    pub selector: TransportSelector,
}

impl DeviceSelection {
    /// Creates a `DeviceSelection` from command-line arguments and an environment lookup function.
    ///
    /// The environment lookup `env` is passed as a function or closure `Fn(&str) -> Option<String>`.
    /// The real environment is never read directly.
    ///
    /// Returns `(DeviceSelection, remaining_args)`.
    pub fn from_args_and_env<I, S, E>(
        args: I,
        env: E,
    ) -> Result<(Self, Vec<String>), SelectionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
        E: Fn(&str) -> Option<String>,
    {
        let mut serial = None;
        let mut transport_id = None;
        let mut usb = false;
        let mut local = false;
        let mut specified_d_e_s = false;

        let mut args_iter = args.into_iter().map(|s| s.as_ref().to_string());
        let mut remaining = Vec::new();

        while let Some(arg) = args_iter.next() {
            if arg == "-d" {
                usb = true;
                local = false;
                specified_d_e_s = true;
            } else if arg == "-e" {
                local = true;
                usb = false;
                specified_d_e_s = true;
            } else if let Some(rest) = arg.strip_prefix("-s") {
                specified_d_e_s = true;
                if !rest.is_empty() {
                    if rest.chars().next().unwrap().is_ascii_digit() {
                        serial = Some(rest.to_string());
                    } else {
                        return Err(SelectionError::SRequiresArgument);
                    }
                } else if let Some(next_arg) = args_iter.next() {
                    serial = Some(next_arg);
                } else {
                    return Err(SelectionError::SRequiresArgument);
                }
            } else if let Some(rest) = arg.strip_prefix("-t") {
                let id_str = if !rest.is_empty() {
                    if rest.chars().next().unwrap().is_ascii_digit() {
                        rest.to_string()
                    } else {
                        return Err(SelectionError::TRequiresArgument);
                    }
                } else if let Some(next_arg) = args_iter.next() {
                    next_arg
                } else {
                    return Err(SelectionError::TRequiresArgument);
                };

                if id_str.is_empty() || !id_str.chars().all(|c| c.is_ascii_digit()) {
                    return Err(SelectionError::InvalidTransportId);
                }
                let id = id_str
                    .parse::<u64>()
                    .map_err(|_| SelectionError::InvalidTransportId)?;
                transport_id = Some(id);
            } else {
                remaining.push(arg);
                break;
            }
        }

        for arg in args_iter {
            remaining.push(arg);
        }

        if !specified_d_e_s {
            if let Some(env_serial) = env("ANDROID_SERIAL") {
                if !env_serial.is_empty() {
                    serial = Some(env_serial);
                }
            }
        }

        let selector = if let Some(id) = transport_id {
            TransportSelector::Id(id)
        } else if let Some(ref s) = serial {
            TransportSelector::Serial(s.clone())
        } else if usb {
            TransportSelector::Usb
        } else if local {
            TransportSelector::Local
        } else {
            TransportSelector::Any
        };

        Ok((
            Self {
                serial,
                transport_id,
                usb,
                local,
                selector,
            },
            remaining,
        ))
    }
}
