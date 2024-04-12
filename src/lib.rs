/// trait for replication
/// unfortunately this thing has to take `self` because it usually consumes the thing
use futures_lite::{AsyncRead, AsyncWrite, Future, StreamExt};
use std::{fmt::Debug, marker::Unpin};
use thiserror::Error;
use tracing::info;

use random_access_storage::RandomAccess;

use hypercore::Hypercore;
use hypercore_protocol::{Duplex, Event, ProtocolBuilder};

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ReplicatorError {
    #[error("There was an error in the opening the protocol in handshake")]
    // TODO add key and show it
    HypercoreError(#[from] std::io::Error),
}

pub trait Replicator {
    fn replicate<S>(
        self,
        stream: S,
        is_initiator: bool,
    ) -> impl Future<Output = Result<(), ReplicatorError>> + Send
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static;
}

impl<T: RandomAccess + Debug + Send> Replicator for Hypercore<T> {
    fn replicate<S>(
        self,
        stream: S,
        is_initiator: bool,
    ) -> impl Future<Output = Result<(), ReplicatorError>> + Send
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let key = self.key_pair().public.to_bytes().clone();
        async_std::task::spawn(async move {
            let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
            while let Some(event) = protocol.next().await {
                let event = event.unwrap();
                info!("protocol event {:?}", event);
                match event {
                    Event::Handshake(_m) => {
                        info!("got handshake! {_m:?}");
                        if is_initiator {
                            protocol.open(key).await?;
                        }
                    }
                    _ => todo!(),
                }
            }
            Ok(())
        })
    }
}
