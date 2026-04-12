use anyhow::{Context, Result};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tracing::info;

use crate::converter::IngestMessage;

#[allow(clippy::doc_overindented_list_items)]
pub mod proto {
    tonic::include_proto!("graphrag");
}

use proto::graph_rag_engine_client::GraphRagEngineClient;
use proto::{
    IngestMessage as ProtoIngestMessage, IngestRequest, ListSchemasRequest, MessageMetadata,
};

/// gRPC client for the Vegapunk GraphRag engine.
pub struct VegapunkClient {
    client: GraphRagEngineClient<
        tonic::service::interceptor::InterceptedService<Channel, AuthInterceptor>,
    >,
}

#[derive(Clone)]
struct AuthInterceptor {
    token: String,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        let val: MetadataValue<_> = format!("Bearer {}", self.token)
            .parse()
            .map_err(|_| tonic::Status::internal("invalid auth token"))?;
        req.metadata_mut().insert("authorization", val);
        Ok(req)
    }
}

impl VegapunkClient {
    /// Connect to the Vegapunk gRPC endpoint and attach the given auth token.
    pub async fn connect(endpoint: &str, auth_token: &str) -> Result<Self> {
        let channel = Channel::from_shared(format!("http://{endpoint}"))
            .context("invalid gRPC endpoint")?
            .connect()
            .await
            .context("failed to connect to Vegapunk gRPC")?;
        let interceptor = AuthInterceptor {
            token: auth_token.to_string(),
        };
        let client = GraphRagEngineClient::with_interceptor(channel, interceptor);
        Ok(Self { client })
    }

    /// Return true if a schema with the given name exists in Vegapunk.
    pub async fn check_schema_exists(&mut self, schema_name: &str) -> Result<bool> {
        let response = self
            .client
            .list_schemas(ListSchemasRequest {})
            .await
            .context("failed to list schemas")?;
        let exists = response
            .into_inner()
            .schemas
            .iter()
            .any(|s| s.name == schema_name);
        Ok(exists)
    }

    /// Query the most recent message timestamp for a channel from Vegapunk.
    ///
    /// Returns `None` if no messages have been ingested for the channel yet.
    pub async fn get_latest_message_ts(
        &mut self,
        schema: &str,
        channel_id: &str,
    ) -> Result<Option<String>> {
        let request = proto::QueryNodesRequest {
            schema: schema.to_string(),
            node_type: "Message".to_string(),
            filters: vec![proto::AttributeFilter {
                key: "channel_id".to_string(),
                op: "eq".to_string(),
                value: channel_id.to_string(),
            }],
            sort_by: Some("timestamp".to_string()),
            sort_order: Some("desc".to_string()),
            limit: Some(1),
            offset: None,
            traverse: None,
        };
        let response = self
            .client
            .query_nodes(request)
            .await
            .context("failed to query nodes for cursor recovery")?;
        let nodes = response.into_inner().nodes;
        let ts = nodes
            .first()
            .and_then(|n| n.attributes.get("timestamp").cloned());
        Ok(ts)
    }

    /// Ingest a batch of messages into Vegapunk under the given schema.
    ///
    /// Returns the number of messages ingested as reported by the server.
    pub async fn ingest(&mut self, messages: Vec<IngestMessage>, schema: &str) -> Result<i32> {
        let proto_messages: Vec<ProtoIngestMessage> = messages
            .into_iter()
            .map(|m| ProtoIngestMessage {
                id: Some(m.id),
                text: m.text,
                metadata: Some(MessageMetadata {
                    source_type: m.metadata.source_type,
                    author: m.metadata.author,
                    author_id: Some(m.metadata.author_id),
                    channel: m.metadata.channel,
                    channel_id: Some(m.metadata.channel_id),
                    thread_id: m.metadata.thread_id,
                    timestamp: m.metadata.timestamp,
                }),
            })
            .collect();

        let request = IngestRequest {
            messages: proto_messages,
            schema: schema.to_string(),
        };

        let response = self
            .client
            .ingest(request)
            .await
            .context("failed to ingest messages")?;
        info!(
            ingested_count = response.get_ref().ingested_count,
            "ingest response"
        );
        Ok(response.into_inner().ingested_count)
    }
}
