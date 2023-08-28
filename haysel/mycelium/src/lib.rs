//! data sent over IPC should be serialized with json

#[macro_use]
extern crate serde;
#[macro_use]
extern crate thiserror;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
pub use squirrel;
pub use squirrel::api::station;
use squirrel::api::station::{
    capabilities::{Channel, ChannelData, ChannelID, KnownChannels},
    identity::{KnownStations, StationID},
};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum IPCError {
    #[error("IO Operation failed {0}")]
    IO(#[from] io::Error),
    #[error("Serialize/Deserialize failed {0}")]
    Serde(#[from] serde_json::Error),
}

/// Write a IPC packet to a stream.
///
/// Receive packet with `ipc_recv`
pub async fn ipc_send<T: Serialize>(
    socket: &mut (impl AsyncWriteExt + Unpin),
    packet: &T,
) -> Result<(), IPCError> {
    let serialized = serde_json::to_vec(packet)?;
    let len_bytes = (serialized.len() as u64).to_be_bytes();
    socket.write_all(&len_bytes).await?;
    socket.write_all(&serialized).await?;
    Ok(())
}

/// Reads an IPC packet from `socket`
///
/// this will only work if *every previous packet received was correct*
/// or if the stream was 'reset', as in no bytes from previous packets are left over
pub async fn ipc_recv<T: DeserializeOwned>(
    socket: &mut (impl AsyncReadExt + Unpin),
) -> Result<T, IPCError> {
    let mut buf = [0u8; 8]; //u64
    socket.read_exact(&mut buf).await?;
    let amnt = u64::from_be_bytes(buf);
    let mut buf = vec![0u8; amnt as _];
    socket.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPCMsg {
    pub kind: IPCMsgKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IPCMsgKind {
    /// Initialization packet, sent with current information about stuff
    Haiii {
        stations: KnownStations,
        channels: KnownChannels,
    },
    FreshHotData {
        from: StationID,
        recorded_at: DateTime<Utc>,
        by_channel: HashMap<ChannelID, ChannelData>,
    },
    NewStation {
        id: StationID,
    },
    NewChannel {
        id: ChannelID,
        ch: Channel,
    },
    StationNewChannel {
        station: StationID,
        channel: ChannelID,
    },
}
