use std::sync::Arc;

use openraft::raft::{
    AppendEntriesRequest, InstallSnapshotRequest, VoteRequest,
};
use openraft::Raft;
use tonic::{Request, Response, Status};

use super::network::proto;
use super::network::proto::raft_service_server::RaftService;
use super::types::RuneTypeConfig;

/// gRPC server that handles incoming Raft RPCs from peer nodes.
pub struct RaftGrpcServer {
    raft: Arc<Raft<RuneTypeConfig>>,
}

impl RaftGrpcServer {
    pub fn new(raft: Arc<Raft<RuneTypeConfig>>) -> Self {
        Self { raft }
    }

    pub fn into_service(
        self,
    ) -> proto::raft_service_server::RaftServiceServer<Self> {
        proto::raft_service_server::RaftServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl RaftService for RaftGrpcServer {
    async fn append_entries(
        &self,
        request: Request<proto::RaftRequest>,
    ) -> Result<Response<proto::RaftResponse>, Status> {
        let req: AppendEntriesRequest<RuneTypeConfig> =
            serde_json::from_slice(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let result = self.raft.append_entries(req).await;

        let data = serde_json::to_vec(&result)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(proto::RaftResponse { data }))
    }

    async fn vote(
        &self,
        request: Request<proto::RaftRequest>,
    ) -> Result<Response<proto::RaftResponse>, Status> {
        let req: VoteRequest<u64> =
            serde_json::from_slice(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let result = self.raft.vote(req).await;

        let data = serde_json::to_vec(&result)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(proto::RaftResponse { data }))
    }

    async fn install_snapshot(
        &self,
        request: Request<proto::RaftRequest>,
    ) -> Result<Response<proto::RaftResponse>, Status> {
        let req: InstallSnapshotRequest<RuneTypeConfig> =
            serde_json::from_slice(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let result = self.raft.install_snapshot(req).await;

        let data = serde_json::to_vec(&result)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(proto::RaftResponse { data }))
    }
}
