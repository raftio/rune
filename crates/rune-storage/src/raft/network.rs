use std::future::Future;

use openraft::error::{InstallSnapshotError, RPCError, RaftError, RemoteError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::{BasicNode, Snapshot, Vote};

use super::types::RuneTypeConfig;

pub mod proto {
    tonic::include_proto!("rune.raft");
}

fn mk_unreachable(msg: impl std::fmt::Display) -> Unreachable {
    Unreachable::new(&openraft::AnyError::error(msg))
}

// ---------------------------------------------------------------------------
// Network factory
// ---------------------------------------------------------------------------

pub struct RuneNetworkFactory;

impl RaftNetworkFactory<RuneTypeConfig> for RuneNetworkFactory {
    type Network = RuneNetworkConn;

    async fn new_client(&mut self, _target: u64, node: &BasicNode) -> Self::Network {
        RuneNetworkConn {
            addr: node.addr.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Single peer connection
// ---------------------------------------------------------------------------

pub struct RuneNetworkConn {
    addr: String,
}

impl RuneNetworkConn {
    async fn client(
        &self,
    ) -> Result<
        proto::raft_service_client::RaftServiceClient<tonic::transport::Channel>,
        Unreachable,
    > {
        let url = format!("http://{}", self.addr);
        proto::raft_service_client::RaftServiceClient::connect(url)
            .await
            .map_err(|e| mk_unreachable(e))
    }
}

impl RaftNetwork<RuneTypeConfig> for RuneNetworkConn {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<RuneTypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        let data = serde_json::to_vec(&rpc).map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        let mut client = self.client().await.map_err(RPCError::Unreachable)?;

        let resp = client
            .append_entries(proto::RaftRequest { data })
            .await
            .map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        let result: Result<AppendEntriesResponse<u64>, RaftError<u64>> =
            serde_json::from_slice(&resp.into_inner().data)
                .map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        result.map_err(|e| RPCError::RemoteError(RemoteError::new(0, e)))
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<u64>,
        _option: RPCOption,
    ) -> Result<VoteResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        let data = serde_json::to_vec(&rpc).map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        let mut client = self.client().await.map_err(RPCError::Unreachable)?;

        let resp = client
            .vote(proto::RaftRequest { data })
            .await
            .map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        let result: Result<VoteResponse<u64>, RaftError<u64>> =
            serde_json::from_slice(&resp.into_inner().data)
                .map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        result.map_err(|e| RPCError::RemoteError(RemoteError::new(0, e)))
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote<u64>,
        snapshot: Snapshot<RuneTypeConfig>,
        _cancel: impl Future<Output = openraft::error::ReplicationClosed> + Send + 'static,
        _option: RPCOption,
    ) -> Result<
        openraft::raft::SnapshotResponse<u64>,
        openraft::error::StreamingError<RuneTypeConfig, openraft::error::Fatal<u64>>,
    > {
        use std::io::Read;

        let mut data_buf = Vec::new();
        let mut cursor = *snapshot.snapshot;
        cursor.read_to_end(&mut data_buf).map_err(|e| {
            openraft::error::StreamingError::Unreachable(mk_unreachable(e))
        })?;

        let install_req: InstallSnapshotRequest<RuneTypeConfig> = InstallSnapshotRequest {
            vote,
            meta: snapshot.meta.clone(),
            offset: 0,
            data: data_buf,
            done: true,
        };

        let req_data = serde_json::to_vec(&install_req).map_err(|e| {
            openraft::error::StreamingError::Unreachable(mk_unreachable(e))
        })?;

        let mut client = self.client().await.map_err(|e| {
            openraft::error::StreamingError::Unreachable(e)
        })?;

        let resp = client
            .install_snapshot(proto::RaftRequest { data: req_data })
            .await
            .map_err(|e| {
                openraft::error::StreamingError::Unreachable(mk_unreachable(e))
            })?;

        let result: Result<InstallSnapshotResponse<u64>, RaftError<u64, InstallSnapshotError>> =
            serde_json::from_slice(&resp.into_inner().data).map_err(|e| {
                openraft::error::StreamingError::Unreachable(mk_unreachable(e))
            })?;

        match result {
            Ok(resp) => Ok(openraft::raft::SnapshotResponse { vote: resp.vote }),
            Err(RaftError::APIError(_)) => Ok(openraft::raft::SnapshotResponse { vote }),
            Err(RaftError::Fatal(f)) => Err(openraft::error::StreamingError::StorageError(
                openraft::StorageError::IO {
                    source: openraft::StorageIOError::write(&openraft::AnyError::error(f)),
                },
            )),
        }
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<RuneTypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<u64>,
        RPCError<u64, BasicNode, RaftError<u64, InstallSnapshotError>>,
    > {
        let data = serde_json::to_vec(&rpc).map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        let mut client = self.client().await.map_err(RPCError::Unreachable)?;

        let resp = client
            .install_snapshot(proto::RaftRequest { data })
            .await
            .map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        let result: Result<InstallSnapshotResponse<u64>, RaftError<u64, InstallSnapshotError>> =
            serde_json::from_slice(&resp.into_inner().data)
                .map_err(|e| RPCError::Unreachable(mk_unreachable(e)))?;

        result.map_err(|e| RPCError::RemoteError(RemoteError::new(0, e)))
    }
}
