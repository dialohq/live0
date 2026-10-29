use std::net::SocketAddr;

use crate::{Error, Manager};

/// Serves `manager` over the `proto/snapshot_manager.proto` contract until the server stops.
pub async fn serve(manager: Manager, listen: SocketAddr) -> Result<(), Error> {
    todo!()
}
