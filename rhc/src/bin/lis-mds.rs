use rhc::{
    lease::{Domain, Lease, LeaseProof},
    message::{Operation, OperationType},
    node::{NodeRole, RhcNode},
    storage::InMemoryStorage,
    time::HybridClock,
    NodeId, Result,
};
use std::{
    env,
    net::SocketAddr,
    sync::Arc,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdsStatus {
    pub node_id: String,
    pub role: String,
    pub level: u8,
    pub peer_count: usize,
    pub lease_count: usize,
    pub uptime_seconds: u64,
    pub is_leader: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataRequest {
    pub path: String,
    pub operation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataResponse {
    pub success: bool,
    pub data: Option<Vec<u8>>,
    pub lease_holder: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MdsState {
    pub rhc_node: Arc<RhcNode>,
    pub start_time: std::time::Instant,
}

impl MdsState {
    pub fn new(rhc_node: Arc<RhcNode>) -> Self {
        Self {
            rhc_node,
            start_time: std::time::Instant::now(),
        }
    }
}

async fn get_status(State(state): State<MdsState>) -> Json<MdsStatus> {
    let peer_count = state.rhc_node.peer_count();
    let uptime = state.start_time.elapsed().as_secs();
    
    Json(MdsStatus {
        node_id: format!("{:?}", state.rhc_node.id),
        role: "RHC MDS".to_string(),
        level: state.rhc_node.level,
        peer_count,
        lease_count: 0, // TODO: Count active leases
        uptime_seconds: uptime,
        is_leader: true, // TODO: Check if this node is the current BFT leader
    })
}

async fn handle_metadata(
    State(state): State<MdsState>,
    Json(request): Json<MetadataRequest>,
) -> std::result::Result<Json<MetadataResponse>, StatusCode> {
    info!("Metadata request: {} on {}", request.operation, request.path);
    
    match request.operation.as_str() {
        "lookup" => {
            // Try to read from RHC storage
            match state.rhc_node.storage().get(&request.path).await {
                Ok(Some(data)) => {
                    Ok(Json(MetadataResponse {
                        success: true,
                        data: Some(data),
                        lease_holder: Some(format!("{:?}", state.rhc_node.id)),
                        error: None,
                    }))
                }
                Ok(None) => {
                    Ok(Json(MetadataResponse {
                        success: false,
                        data: None,
                        lease_holder: None,
                        error: Some("File not found".to_string()),
                    }))
                }
                Err(e) => {
                    error!("Storage error: {}", e);
                    Ok(Json(MetadataResponse {
                        success: false,
                        data: None,
                        lease_holder: None,
                        error: Some(format!("Storage error: {}", e)),
                    }))
                }
            }
        }
        "create" => {
            // Create operation via RHC consensus
            let operation_data = bincode::serialize(&(request.path.clone(), Vec::<u8>::new()))
                .unwrap_or_else(|_| Vec::new());
                
            let operation = Operation {
                id: uuid::Uuid::new_v4(),
                op_type: OperationType::Write,
                data: operation_data,
                lease_proof: LeaseProof {
                    lease: Lease {
                        id: uuid::Uuid::new_v4(),
                        domain: Domain::new("mds".to_string(), None, 0),
                        holder: state.rhc_node.id,
                        start_time: HybridClock::new().now(),
                        duration: chrono::Duration::minutes(10),
                        parent_lease: None,
                        signature: rhc::crypto::Signature::default(),
                    },
                    chain: vec![],
                },
                timestamp: HybridClock::new().now(),
            };
            
            match state.rhc_node.storage().apply_operation(&operation).await {
                Ok(_) => {
                    info!("Created file: {}", request.path);
                    Ok(Json(MetadataResponse {
                        success: true,
                        data: Some(Vec::new()),
                        lease_holder: Some(format!("{:?}", state.rhc_node.id)),
                        error: None,
                    }))
                }
                Err(e) => {
                    error!("Failed to create {}: {}", request.path, e);
                    Ok(Json(MetadataResponse {
                        success: false,
                        data: None,
                        lease_holder: None,
                        error: Some(format!("Create failed: {}", e)),
                    }))
                }
            }
        }
        _ => {
            Ok(Json(MetadataResponse {
                success: false,
                data: None,
                lease_holder: None,
                error: Some(format!("Unknown operation: {}", request.operation)),
            }))
        }
    }
}

async fn list_peers(State(state): State<MdsState>) -> Json<Vec<String>> {
    let peers: Vec<String> = state.rhc_node.peer_ids().iter()
        .map(|id| format!("{:?}", id))
        .collect();
    Json(peers)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    // Get bind address from environment
    let bind_addr = env::var("LIS_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:7000".to_string());
    
    info!("LIS MDS (Metadata Server) starting...");
    info!("Bind address: {}", bind_addr);
    
    // Create RHC node - ALL nodes participate in consensus
    let node_id = NodeId::new();
    let rhc_node = Arc::new(RhcNode::new(
        NodeRole::Hybrid, // All nodes are hybrid - can be local leader AND global arbitrator
        0, // No artificial hierarchy levels - all nodes are peers
        Arc::new(InMemoryStorage::new()),
        None,
    ));
    
    // Parse and connect to other MDS nodes for HA
    if let Ok(peer_addrs) = env::var("LIS_MDS_PEERS") {
        info!("Connecting to MDS peers: {}", peer_addrs);
        for addr in peer_addrs.split(',') {
            let addr = addr.trim();
            let peer_id = NodeId::new(); // TODO: Proper peer ID exchange
            rhc_node.add_peer(peer_id, 0); // All nodes are peers
            info!("Added peer: {} -> {:?}", addr, peer_id);
        }
    }
    
    // Start RHC node
    rhc_node.start().await?;
    info!("RHC MDS node {:?} started", node_id);
    
    // Create MDS state
    let state = MdsState::new(rhc_node);
    
    // Build API routes
    let app = Router::new()
        .route("/status", get(get_status))
        .route("/metadata", post(handle_metadata))
        .route("/peers", get(list_peers))
        .with_state(state);
    
    // Bind to address
    let addr: SocketAddr = bind_addr.parse()
        .map_err(|e| rhc::Error::Other(anyhow::anyhow!("Invalid bind address: {}", e)))?;
    
    let listener = TcpListener::bind(&addr).await
        .map_err(|e| rhc::Error::Other(anyhow::anyhow!("Failed to bind: {}", e)))?;
    
    info!("LIS MDS listening on {}", addr);
    info!("API endpoints:");
    info!("  GET  /status   - MDS status");
    info!("  POST /metadata - Metadata operations");
    info!("  GET  /peers    - Connected peers");
    
    // Start server
    axum::serve(listener, app).await
        .map_err(|e| rhc::Error::Other(anyhow::anyhow!("Server error: {}", e)))?;
    
    Ok(())
}