//! adb-server: Host server daemon implementation and service routing.

pub mod router;
pub mod stream;

pub use router::{
    OpenDecision, OpenHandler, OpenRequest, PacketRouter, RouterError, StreamMessage,
    StreamReceiver, StreamSender,
};
pub use stream::{StreamEntry, StreamError, StreamRegistry};

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
