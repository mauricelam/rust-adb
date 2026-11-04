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
/// Encapsulates `TransportSelector` so contradictory combinations (like `usb && local`)
/// are impossible to represent in the type system.
/// Ports device selection and option parsing logic from `original/client/commandline.cpp`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceSelection {
    pub selector: TransportSelector,
}

impl From<TransportSelector> for DeviceSelection {
    fn from(selector: TransportSelector) -> Self {
        Self { selector }
    }
}

impl DeviceSelection {
    pub fn new(selector: TransportSelector) -> Self {
        Self { selector }
    }

    pub fn selector(&self) -> &TransportSelector {
        &self.selector
    }

    pub fn serial(&self) -> Option<&str> {
        match &self.selector {
            TransportSelector::Serial(s) => Some(s),
            _ => None,
        }
    }

    pub fn transport_id(&self) -> Option<u64> {
        match self.selector {
            TransportSelector::Id(id) => Some(id),
            _ => None,
        }
    }

    pub fn is_usb(&self) -> bool {
        matches!(self.selector, TransportSelector::Usb)
    }

    pub fn is_local(&self) -> bool {
        matches!(self.selector, TransportSelector::Local)
    }

    /// Parses a single device selection argument from an iterator if it matches `-d`, `-e`, `-s`, or `-t`.
    /// Returns `Ok(Some((selector, specified_d_e_s)))` if consumed, `Ok(None)` if not an option for device selection,
    /// or `Err(SelectionError)`.
    pub fn parse_arg_step<I>(
        arg: &str,
        args_iter: &mut I,
    ) -> Result<Option<(TransportSelector, bool)>, SelectionError>
    where
        I: Iterator<Item = String>,
    {
        if arg == "-d" {
            Ok(Some((TransportSelector::Usb, true)))
        } else if arg == "-e" {
            Ok(Some((TransportSelector::Local, true)))
        } else if let Some(rest) = arg.strip_prefix("-s") {
            let serial = if !rest.is_empty() {
                if rest.chars().next().unwrap().is_ascii_digit() {
                    rest.to_string()
                } else {
                    return Err(SelectionError::SRequiresArgument);
                }
            } else if let Some(next_arg) = args_iter.next() {
                next_arg
            } else {
                return Err(SelectionError::SRequiresArgument);
            };
            Ok(Some((TransportSelector::Serial(serial), true)))
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
            Ok(Some((TransportSelector::Id(id), true)))
        } else {
            Ok(None)
        }
    }

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
        let mut selector = None;
        let mut specified_d_e_s = false;

        let mut args_iter = args.into_iter().map(|s| s.as_ref().to_string());
        let mut remaining = Vec::new();

        while let Some(arg) = args_iter.next() {
            match Self::parse_arg_step(&arg, &mut args_iter)? {
                Some((s, d_e_s)) => {
                    selector = Some(s);
                    if d_e_s {
                        specified_d_e_s = true;
                    }
                }
                None => {
                    remaining.push(arg);
                    break;
                }
            }
        }

        for arg in args_iter {
            remaining.push(arg);
        }

        let effective_selector = if let Some(s) = selector {
            s
        } else if !specified_d_e_s {
            if let Some(env_serial) = env("ANDROID_SERIAL") {
                if !env_serial.is_empty() {
                    TransportSelector::Serial(env_serial)
                } else {
                    TransportSelector::Any
                }
            } else {
                TransportSelector::Any
            }
        } else {
            TransportSelector::Any
        };

        Ok((
            Self {
                selector: effective_selector,
            },
            remaining,
        ))
    }
}
