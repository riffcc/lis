use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    fs,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
    task::{Context, Poll},
};

use color_eyre::{Result, eyre::eyre};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use fuser::{FileAttr, FileType, Filesystem, Request, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry};
use libp2p::{
    core::{
        muxing::{StreamMuxerBox, StreamMuxerExt},
        transport::Boxed,
    },
    identity,
    kad::{
        store::MemoryStore,
        Behaviour as Kademlia,
    },
    noise,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp,
    yamux,
    Multiaddr,
    PeerId,
    SwarmBuilder,
    identify,
    Transport,
    dns,
};
use nix::mount::{self};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
    Frame,
};
use redb::TableDefinition;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use toml;
use uuid::Uuid;
use futures::{AsyncReadExt, AsyncWriteExt};
use std::pin::Pin;
use base64;
use base64::Engine;
use if_addrs;

const DOCUMENTS: TableDefinition<&str, &[u8]> = TableDefinition::new("documents");
const ROOT_DOC_KEY: &str = "root";
const NODE_TIMEOUT_SECS: u64 = 60;
const DEFAULT_PORT: u16 = 34163;
const RELAY_TOPIC: &str = "/lis/relay/v1";
const BOOTSTRAP_NODES: [&str; 6] = [
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
    "/ip4/104.131.131.82/tcp/4001/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ",
    "/ip4/104.131.131.82/udp/4001/quic/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
    "/dnsaddr/bootstrap.libp2p.io/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
];

// Document types for MerkleDAG structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RootDoc {
    inode_map: DocumentId,
    top_level_directory: DocumentId,
}

#[derive(Debug, Serialize, Deserialize)]
struct InodeMapDoc {
    inode_to_doc: HashMap<Uuid, DocumentId>,
    doc_to_inode: HashMap<DocumentId, Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirectoryDoc {
    metadata: DocumentId,
    children: DocumentId,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChildrenDoc {
    entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
enum DirectoryEntry {
    Folder {
        name: String,
        directory_doc: DocumentId,
    },
    File {
        name: String,
        file_doc: DocumentId,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct FileDoc {
    metadata: DocumentId,
    chunks: Vec<DocumentId>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MetadataDoc {
    name: String,
    doc_type: DocType,
    size: u64,
    inode_uuid: Uuid,
    modified: u64,
    hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
enum DocType {
    Directory,
    File,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
struct DocumentId(String);

impl DocumentId {
    fn new(content: &[u8]) -> Self {
        let hash = blake3::hash(content);
        DocumentId(hex::encode(hash.as_bytes()))
    }
}

// Enum to represent CLI commands
#[derive(Debug, PartialEq)]
enum CliCommand {
    Interactive { config: Option<String> },
    Help,
    Cluster { action: ClusterAction, config: Option<String> },
    Daemon { config: Option<String> },
    Mount { config: Option<String> },
    Unmount { config: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    Normal,
    Editing,
}

#[derive(Clone)]
struct SwarmWrapper(Arc<Mutex<Swarm<LisNetworkBehaviour>>>);

impl std::fmt::Debug for SwarmWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmWrapper")
            .field("swarm", &"<Swarm>")
            .finish()
    }
}

impl SwarmWrapper {
    fn new(swarm: Swarm<LisNetworkBehaviour>) -> Self {
        Self(Arc::new(Mutex::new(swarm)))
    }
}

#[derive(Clone)]
struct TransportWrapper(Arc<Boxed<(PeerId, StreamMuxerBox)>>);

impl TransportWrapper {
    fn new(transport: Boxed<(PeerId, StreamMuxerBox)>) -> Self {
        Self(Arc::new(transport))
    }
}

/// Application state
struct AppState {
    config_path: PathBuf,
    clusters: Vec<String>,
    selected_cluster: Option<usize>,
    message: Option<String>,
    network_status: Option<String>,
    peer_id: Option<PeerId>,
    swarm: Option<Arc<Mutex<Swarm<LisNetworkBehaviour>>>>,
    input_mode: InputMode,
    input_buffer: String,
    bootstrap_peers: Option<HashSet<PeerId>>,
}

impl AppState {
    fn new(config: Option<String>) -> Result<Self> {
        let config_path = if let Some(cfg) = config {
            PathBuf::from(cfg)
        } else {
            let home = env::var("HOME").map_err(|_| eyre!("$HOME not set"))?;
            PathBuf::from(home).join(".lis").join("config.toml")
        };
        
        // Create config directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        Ok(AppState {
            config_path,
            clusters: Vec::new(),
            selected_cluster: None,
            message: None,
            network_status: None,
            peer_id: None,
            swarm: None,
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            bootstrap_peers: None,
        })
    }

    async fn init_p2p(&mut self) -> Result<()> {
        // Load or create a persistent keypair
        let key_path = self.config_path.parent().unwrap().join("peer_key");
        let local_key = if key_path.exists() {
            // Load existing keypair
            let key_bytes = fs::read(&key_path)?;
            identity::Keypair::from_protobuf_encoding(&key_bytes)?
        } else {
            // Generate and save new keypair
            let key = identity::Keypair::generate_ed25519();
            fs::write(&key_path, key.to_protobuf_encoding()?)?;
            key
        };

        let local_peer_id = PeerId::from(local_key.public());
        println!("Local peer ID: {}", local_peer_id);
        self.peer_id = Some(local_peer_id);

        // Create behaviours with more aggressive timeouts
        let store = MemoryStore::new(local_peer_id);
        let mut kad = Kademlia::new(local_peer_id, store);
        let relay = {
            let mut config = libp2p_relay::Config::default();
            config.max_circuits = 100;
            config.max_reservations = 100;
            config.max_reservations_per_peer = 10;
            config.reservation_duration = Duration::from_secs(3600); // 1 hour
            libp2p_relay::Behaviour::new(local_peer_id, config)
        };
        let dcutr = libp2p_dcutr::Behaviour::new(local_peer_id);
        let identify = identify::Behaviour::new(identify::Config::new(
            "lis/1.0".to_string(),
            local_key.public(),
        ));
        
        // Configure Kademlia for better bootstrapping
        kad.set_mode(Some(libp2p::kad::Mode::Server));
        
        // Add bootstrap nodes with better error handling
        let mut bootstrap_addresses: Vec<(PeerId, Multiaddr)> = Vec::new();
        let mut bootstrap_peer_ids: HashSet<PeerId> = HashSet::new();
        
        // Create the transport with more aggressive timeouts
        let tcp_config = tcp::Config::default()
            .nodelay(true);
        
        let yamux_config = yamux::Config::default();
        
        let transport = libp2p::dns::tokio::Transport::system(libp2p::tcp::tokio::Transport::new(tcp_config.clone()))?
            .upgrade(libp2p::core::upgrade::Version::V1Lazy)
            .authenticate(noise::Config::new(&local_key).expect("signing libp2p-noise static keypair"))
            .multiplex(yamux_config)
            .timeout(Duration::from_secs(20))
            .boxed();

        // Build the swarm with custom configuration
        let behaviour = LisNetworkBehaviour {
            kademlia: kad,
            relay,
            dcutr,
            identify,
        };

        let mut swarm = SwarmBuilder::with_existing_identity(local_key)
            .with_tokio()
            .with_tcp(
                tcp_config,
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|_| Ok(behaviour))?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(60))
                   .with_dial_concurrency_factor(std::num::NonZeroU8::new(4).unwrap())
                   .with_notify_handler_buffer_size(std::num::NonZeroUsize::new(32).unwrap())
                   .with_per_connection_event_buffer_size(64)
            })
            .build();

        // Listen on all interfaces
        println!("Attempting to listen on port {}", DEFAULT_PORT);
        match swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", DEFAULT_PORT).parse()?) {
            Ok(_) => println!("Successfully listening on port {}", DEFAULT_PORT),
            Err(e) => println!("Failed to listen on port {}: {}", DEFAULT_PORT, e),
        }
        
        // Also listen on a random port as fallback
        match swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?) {
            Ok(_) => println!("Successfully listening on random port"),
            Err(e) => println!("Failed to listen on random port: {}", e),
        }

        // Store swarm before attempting connections
        self.swarm = Some(Arc::new(Mutex::new(swarm)));
        
        // Connect to bootstrap nodes with retries
        self.connect_to_bootstrap_nodes().await?;
        
        // Wait for bootstrap with a longer timeout
        if self.wait_for_bootstrap(120).await? {
            println!("Successfully bootstrapped into the network");
        } else {
            println!("Warning: Bootstrap process did not complete, but continuing anyway");
        }
        
        Ok(())
    }

    async fn start_listening(&mut self) -> Result<()> {
        if let Some(swarm) = &self.swarm {
            let mut swarm = swarm.lock().await;
            
            // Try to listen on all non-loopback interfaces
            let interfaces = if_addrs::get_if_addrs()?;
            let mut success = false;
            
            for iface in interfaces {
                // Skip loopback interfaces unless it's the only one we have
                if !iface.is_loopback() {
                    let ip = iface.ip();
                    // Only handle IPv4 addresses for now
                    if let std::net::IpAddr::V4(ipv4) = ip {
                        let addr = format!("/ip4/{}/tcp/{}", ipv4, DEFAULT_PORT);
                        match swarm.listen_on(addr.parse()?) {
                            Ok(_) => {
                                println!("Listening on {}", addr);
                                success = true;
                            }
                            Err(e) => {
                                println!("Failed to listen on {}: {}", addr, e);
                                // Try a random port on this interface
                                let random_addr = format!("/ip4/{}/tcp/0", ipv4);
                                if let Ok(_) = swarm.listen_on(random_addr.parse()?) {
                                    println!("Listening on {} (random port)", ipv4);
                                    success = true;
                                }
                            }
                        }
                    }
                }
            }

            // If no other interfaces worked, fall back to loopback
            if !success {
                println!("No external interfaces available, falling back to loopback");
                swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;
            }
        }
        Ok(())
    }

    async fn connect_to_peer(&mut self, addr: Multiaddr) -> Result<()> {
        if let Some(swarm) = &self.swarm {
            let mut swarm = swarm.lock().await;
            swarm.dial(addr)?;
        }
        Ok(())
    }

    async fn handle_swarm_event(&mut self, event: SwarmEvent<LisNetworkBehaviourEvent>) -> Result<()> {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("📡 Listening on {}", address);
                self.network_status = Some(format!("Listening on {}", address));
            }
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                let is_bootstrap = self.bootstrap_peers.as_ref()
                    .map(|peers| peers.contains(&peer_id))
                    .unwrap_or(false);
                
                if is_bootstrap {
                    println!("🌟 Connected to bootstrap peer: {} at {}", peer_id, endpoint.get_remote_address());
                } else {
                    println!("✅ Connected to peer: {} at {}", peer_id, endpoint.get_remote_address());
                }

                // Store the peer's address in our routing table
                if let Some(swarm) = &self.swarm {
                    let mut swarm = swarm.lock().await;
                    let addr = endpoint.get_remote_address();
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.to_owned());
                    
                    // Start a bootstrap process when we connect to a bootstrap node
                    if is_bootstrap {
                        println!("Starting DHT bootstrap after connecting to bootstrap peer...");
                        if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
                            println!("Failed to start bootstrap: {}", e);
                        }
                    }
                }
                self.network_status = Some(format!("Connected to {}{}", 
                    if is_bootstrap { "bootstrap peer " } else { "" },
                    peer_id
                ));
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                let is_bootstrap = self.bootstrap_peers.as_ref()
                    .map(|peers| peers.contains(&peer_id))
                    .unwrap_or(false);
                
                if is_bootstrap {
                    println!("❌ Disconnected from bootstrap peer: {} (cause: {:?})", peer_id, cause);
                } else {
                    println!("❌ Disconnected from peer: {} (cause: {:?})", peer_id, cause);
                }
                self.network_status = Some(format!("Disconnected from {}{}", 
                    if is_bootstrap { "bootstrap peer " } else { "" },
                    peer_id
                ));
            }
            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Identify(identify::Event::Received { peer_id, ref info, .. })) => {
                println!("🔍 Identified peer {} running {}", peer_id, info.protocol_version);
                if let Some(addr) = info.listen_addrs.first() {
                    println!("  📍 Peer {} is listening on {}", peer_id, addr);
                    // Add the address to Kademlia
                    if let Some(swarm) = &self.swarm {
                        let mut swarm = swarm.lock().await;
                        swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                    }
                }
            }
            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Kademlia(ref event)) => {
                match event {
                    libp2p::kad::Event::OutboundQueryProgressed { result, stats, .. } => {
                        match result {
                            libp2p::kad::QueryResult::Bootstrap(_) => {
                                println!("🔄 Bootstrap progress: {} peers in {}ms", 
                                    stats.num_successes(),
                                    stats.duration().map_or(0, |d| d.as_millis())
                                );
                            }
                            libp2p::kad::QueryResult::GetClosestPeers(Ok(ok)) => {
                                println!("👥 Found {} close peers", ok.peers.len());
                                if let Some(swarm) = &self.swarm {
                                    let mut swarm = swarm.lock().await;
                                    let local_peer_id = *swarm.local_peer_id();
                                    let connected = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                    for peer_info in &ok.peers {
                                        let peer_id = peer_info.peer_id;
                                        if peer_id != local_peer_id && !connected.contains(&peer_id) {
                                            println!("Attempting to connect to peer: {}", peer_id);
                                            let _ = swarm.dial(peer_id);
                                        }
                                    }
                                }
                            }
                            libp2p::kad::QueryResult::GetProviders(Ok(ok)) => {
                                match ok {
                                    libp2p::kad::GetProvidersOk::FoundProviders { providers, .. } => {
                                        if let Some(swarm) = &self.swarm {
                                            let mut swarm = swarm.lock().await;
                                            let local_peer_id = *swarm.local_peer_id();
                                            let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                            for provider in providers {
                                                println!("Found provider: {}", provider);
                                                let provider_id = provider.clone();
                                                if provider_id != local_peer_id && !connected_peers.contains(&provider) {
                                                    println!("Attempting to connect to provider: {}", provider);
                                                    let _ = swarm.dial(provider_id);
                                                }
                                            }
                                        }
                                    }
                                    libp2p::kad::GetProvidersOk::FinishedWithNoAdditionalRecord { closest_peers } => {
                                        println!("No providers found, but got {} closest peers", closest_peers.len());
                                        if let Some(swarm) = &self.swarm {
                                            let mut swarm = swarm.lock().await;
                                            let local_peer_id = *swarm.local_peer_id();
                                            let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                            for peer in closest_peers {
                                                let peer_id = peer.clone();
                                                if peer_id != local_peer_id && !connected_peers.contains(&peer) {
                                                    println!("Attempting to connect to closest peer: {}", peer);
                                                    let _ = swarm.dial(peer_id);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Relay(e)) => {
                println!("🔄 Relay event: {:?}", e);
            }
            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Dcutr(ref e)) => {
                println!("🕳️ Hole punching event: {:?}", e);
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                println!("⚠️ Failed to connect to peer {:?}: {}", peer_id, error);
            }
            _ => {}
        }
        Ok(())
    }

    fn load_clusters(&mut self) -> Result<()> {
        let clusters_dir = self.config_path.parent().unwrap().join("clusters");
        if clusters_dir.exists() {
            self.clusters = fs::read_dir(&clusters_dir)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect();
        }
        Ok(())
    }

    fn create_cluster(&mut self, name: &str) -> Result<()> {
        let clusters_dir = self.config_path.parent().unwrap().join("clusters").join(name);
        fs::create_dir_all(&clusters_dir)?;
        
        // Create cluster config
        let config_path = clusters_dir.join("config.toml");
        fs::write(&config_path, format!("name = \"{}\"\nreplication = 2\n", name))?;
        
        // Create cluster database
        let db_path = clusters_dir.join("cluster.db");
        fs::write(&db_path, "")?; // Just create an empty file for now
        
        self.message = Some(format!("Created cluster: {}", name));
        self.load_clusters()?;
        Ok(())
    }

    fn get_inode(&self, _path: &Path) -> Result<u64> {
        // TODO: Implement proper inode mapping
        Ok(1)
    }

    fn get_document(&self, _inode: u64) -> Result<Vec<u8>> {
        // TODO: Implement document retrieval
        Ok(Vec::new())
    }

    fn generate_share_ticket(&self, cluster: &str) -> Result<String> {
        let clusters_dir = self.config_path.parent().unwrap().join("clusters");
        let cluster_path = clusters_dir.join(cluster);
        
        if !cluster_path.exists() {
            return Err(eyre!("Cluster '{}' not found", cluster));
        }

        // Create the ticket data
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs();
        
        let ticket_data = format!("{}:{}:{}", cluster, self.peer_id.unwrap(), timestamp);
        
        // Store the ticket in the cluster's tickets file
        let tickets_file = cluster_path.join("tickets.toml");
        let tickets_content = if tickets_file.exists() {
            fs::read_to_string(&tickets_file)?
        } else {
            String::new()
        };

        let mut tickets: toml::Table = toml::from_str(&tickets_content).unwrap_or_default();
        let mut ticket_info = toml::Table::new();
        ticket_info.insert("peer_id".into(), toml::Value::String(self.peer_id.unwrap().to_string()));
        ticket_info.insert("timestamp".into(), toml::Value::Integer(timestamp as i64));
        ticket_info.insert("data".into(), toml::Value::String(ticket_data.clone()));
        
        // Use base64 to make it more compact and avoid any special characters
        let encoded_ticket = base64::engine::general_purpose::STANDARD.encode(ticket_data);
        tickets.insert(encoded_ticket.clone(), toml::Value::Table(ticket_info));

        fs::write(tickets_file, toml::to_string(&tickets)?)?;
        Ok(encoded_ticket)
    }

    async fn join_cluster(&mut self, cluster: &str, ticket: &str) -> Result<()> {
        let mut connected = false;
        let clusters_dir = self.config_path.parent().unwrap().join("clusters");
        
        // Create local cluster directory first
        let local_cluster_path = clusters_dir.join(cluster);
        fs::create_dir_all(&local_cluster_path)?;

        // Create initial cluster config
        let config_path = local_cluster_path.join("config.toml");
        fs::write(&config_path, format!("name = \"{}\"\nreplication = 2\n", cluster))?;

        // Store the ticket for verification
        let tickets_file = local_cluster_path.join("tickets.toml");
        let mut tickets = toml::Table::new();
        let mut ticket_info = toml::Table::new();
        ticket_info.insert("ticket".into(), toml::Value::String(ticket.to_string()));
        tickets.insert("join_ticket".into(), toml::Value::Table(ticket_info));
        fs::write(&tickets_file, toml::to_string(&tickets)?)?;

        // Start listening and attempt to connect via DHT
        self.start_listening().await?;

        // Decode and parse the ticket
        let ticket_bytes = base64::engine::general_purpose::STANDARD.decode(ticket)
            .map_err(|_| eyre!("Invalid ticket encoding"))?;
        let ticket_data = String::from_utf8(ticket_bytes)?;

        let ticket_parts: Vec<&str> = ticket_data.split(':').collect();
        if ticket_parts.len() != 3 {
            return Err(eyre!("Invalid ticket format"));
        }

        let ticket_cluster = ticket_parts[0];
        if ticket_cluster != cluster {
            return Err(eyre!("Ticket is for cluster '{}', not '{}'", ticket_cluster, cluster));
        }

        let host_peer_id: PeerId = ticket_parts[1].parse()
            .map_err(|_| eyre!("Invalid peer ID in ticket"))?;

        println!("Attempting to connect to host peer: {}", host_peer_id);
        
        if let Some(swarm) = &self.swarm {
            // First add any bootstrap nodes
            {
                let mut swarm = swarm.lock().await;
                println!("Bootstrapping with known nodes...");
                for node in BOOTSTRAP_NODES.iter() {
                    if let Ok(addr) = node.parse::<Multiaddr>() {
                        // Extract the peer ID from the multiaddr
                        if let Some(peer_id) = addr.iter().find_map(|p| {
                            if let libp2p::multiaddr::Protocol::P2p(hash) = p {
                                Some(PeerId::from(hash))
                            } else {
                                None
                            }
                        }) {
                            println!("Adding bootstrap node: {} ({})", addr, peer_id);
                            // Add the address without the peer ID component for dialing
                            let mut dial_addr = addr.clone();
                            dial_addr.pop(); // Remove the /p2p/... component
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, dial_addr);
                        }
                    }
                }
                
                // Start a bootstrap process
                println!("Starting DHT bootstrap...");
                swarm.behaviour_mut().kademlia.bootstrap()?;
            }
            
            // Wait for bootstrap to complete (up to 30 seconds)
            let mut attempts = 0;
            let mut bootstrap_complete = false;
            while !bootstrap_complete && attempts < 30 {
                {
                    let mut swarm = swarm.lock().await;
                    if let Poll::Ready(Some(event)) = futures::Stream::poll_next(Pin::new(&mut *swarm), &mut Context::from_waker(futures::task::noop_waker_ref())) {
                        match event {
                            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Kademlia(
                                libp2p::kad::Event::OutboundQueryProgressed { result: libp2p::kad::QueryResult::Bootstrap(_), .. }
                            )) => {
                                println!("Bootstrap completed successfully");
                                bootstrap_complete = true;
                            }
                            _ => {}
                        }
                    }
                }
                if !bootstrap_complete {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    attempts += 1;
                    if attempts % 5 == 0 {
                        println!("Waiting for bootstrap to complete... ({}/30)", attempts);
                    }
                }
            }
            
            if !bootstrap_complete {
                println!("Warning: Bootstrap did not complete in time, continuing anyway...");
            }
            
            // Now try to find and connect to the host
            {
                let mut swarm = swarm.lock().await;
                // First announce ourselves as a provider for this cluster
                let topic = format!("lis-cluster:{}", cluster);
                println!("Announcing ourselves as provider for topic: {}", topic);
                swarm.behaviour_mut().kademlia.start_providing(topic.as_bytes().to_vec().into())?;
                
                // Then search for other providers
                println!("Searching for cluster providers with topic: {}", topic);
                swarm.behaviour_mut().kademlia.get_providers(topic.as_bytes().to_vec().into());
                
                // Also try direct connection to the host
                println!("Searching for cluster host with peer ID: {}", host_peer_id);
                swarm.behaviour_mut().kademlia.get_closest_peers(host_peer_id);
                
                // Try all known ports for the host
                println!("Attempting to connect to host on known ports...");
                for port in [DEFAULT_PORT, 0] {
                    // Try IPv4 interfaces
                    let interfaces = if_addrs::get_if_addrs()?;
                    for iface in interfaces {
                        if !iface.is_loopback() {
                            if let std::net::IpAddr::V4(ipv4) = iface.ip() {
                                let addr = format!("/ip4/{}/tcp/{}", ipv4, port);
                                if let Ok(multiaddr) = addr.parse::<Multiaddr>() {
                                    swarm.behaviour_mut().kademlia.add_address(&host_peer_id, multiaddr.clone());
                                    println!("Added potential host address: {}", addr);
                                    let _ = swarm.dial(host_peer_id);
                                }
                            }
                        }
                    }
                    
                    // Also try loopback
                    let addr = format!("/ip4/127.0.0.1/tcp/{}", port);
                    if let Ok(multiaddr) = addr.parse::<Multiaddr>() {
                        swarm.behaviour_mut().kademlia.add_address(&host_peer_id, multiaddr.clone());
                        println!("Added potential host address: {}", addr);
                        let _ = swarm.dial(host_peer_id);
                    }
                }
            }

            // Wait for connection to be established
            let mut attempts = 0;
            while attempts < 30 && !connected {
                {
                    let mut swarm = swarm.lock().await;
                    if let Poll::Ready(Some(event)) = futures::Stream::poll_next(Pin::new(&mut *swarm), &mut Context::from_waker(futures::task::noop_waker_ref())) {
                        match event {
                            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } if peer_id == host_peer_id => {
                                println!("Successfully connected to cluster host!");
                                // Store the working address
                                let addr = endpoint.get_remote_address();
                                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.to_owned());
                                println!("Stored working address: {}", addr);
                                
                                // Start providing the cluster topic to maintain visibility
                                let topic = format!("lis-cluster:{}", cluster);
                                let _ = swarm.behaviour_mut().kademlia.start_providing(topic.as_bytes().to_vec().into());
                                
                                connected = true;
                                break;
                            }
                            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Kademlia(ref kad_event)) => {
                                match kad_event {
                                    libp2p::kad::Event::OutboundQueryProgressed { result, stats, .. } => {
                                        match result {
                                            libp2p::kad::QueryResult::Bootstrap(_) => {
                                                println!("🔄 Bootstrap progress: {} peers in {}ms", 
                                                    stats.num_successes(),
                                                    stats.duration().map_or(0, |d| d.as_millis())
                                                );
                                            }
                                            libp2p::kad::QueryResult::GetClosestPeers(Ok(ok)) => {
                                                println!("👥 Found {} close peers", ok.peers.len());
                                                if let Some(swarm) = &self.swarm {
                                                    let mut swarm = swarm.lock().await;
                                                    let local_peer_id = *swarm.local_peer_id();
                                                    let connected = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                                    for peer_info in &ok.peers {
                                                        let peer_id = peer_info.peer_id;
                                                        if peer_id != local_peer_id && !connected.contains(&peer_id) {
                                                            println!("Attempting to connect to peer: {}", peer_id);
                                                            let _ = swarm.dial(peer_id);
                                                        }
                                                    }
                                                }
                                            }
                                            libp2p::kad::QueryResult::GetProviders(Ok(ok)) => {
                                                match ok {
                                                    libp2p::kad::GetProvidersOk::FoundProviders { providers, .. } => {
                                                        if let Some(swarm) = &self.swarm {
                                                            let mut swarm = swarm.lock().await;
                                                            let local_peer_id = *swarm.local_peer_id();
                                                            let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                                            for provider in providers {
                                                                println!("Found provider: {}", provider);
                                                                let provider_id = provider.clone();
                                                                if provider_id != local_peer_id && !connected_peers.contains(&provider) {
                                                                    println!("Attempting to connect to provider: {}", provider);
                                                                    let _ = swarm.dial(provider_id);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    libp2p::kad::GetProvidersOk::FinishedWithNoAdditionalRecord { closest_peers } => {
                                                        println!("No providers found, but got {} closest peers", closest_peers.len());
                                                        if let Some(swarm) = &self.swarm {
                                                            let mut swarm = swarm.lock().await;
                                                            let local_peer_id = *swarm.local_peer_id();
                                                            let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                                            for peer in closest_peers {
                                                                let peer_id = peer.clone();
                                                                if peer_id != local_peer_id && !connected_peers.contains(&peer) {
                                                                    println!("Attempting to connect to closest peer: {}", peer);
                                                                    let _ = swarm.dial(peer_id);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                attempts += 1;
                if attempts % 5 == 0 {
                    println!("Still trying to connect... ({}/30)", attempts);
                    let mut swarm = swarm.lock().await;
                    // Retry all discovery methods periodically
                    let topic = format!("lis-cluster:{}", cluster);
                    swarm.behaviour_mut().kademlia.get_providers(topic.as_bytes().to_vec().into());
                    swarm.behaviour_mut().kademlia.get_closest_peers(host_peer_id);
                }
            }
        }

        if !connected {
            return Err(eyre!("Failed to connect to host peer after 30 seconds"));
        }

        // Add this cluster to our list
        self.load_clusters()?;

        // Store the ticket data in a file for persistence
        let peers_file = clusters_dir.join(cluster).join("known_peers.toml");
        let mut known_peers = if peers_file.exists() {
            toml::from_str(&fs::read_to_string(&peers_file)?).unwrap_or_default()
        } else {
            toml::Table::new()
        };
        
        // Add the host peer to known peers
        let mut peer_info = toml::Table::new();
        peer_info.insert("peer_id".into(), toml::Value::String(host_peer_id.to_string()));
        peer_info.insert("timestamp".into(), toml::Value::Integer(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64));
        known_peers.insert(host_peer_id.to_string(), toml::Value::Table(peer_info));
        
        fs::write(&peers_file, toml::to_string(&known_peers)?)?;

        Ok(())
    }

    async fn connect_to_bootstrap_nodes(&mut self) -> Result<()> {
        if let Some(swarm) = &self.swarm {
            let mut swarm = swarm.lock().await;
            
            // Store bootstrap peers for tracking
            let mut bootstrap_peer_ids: HashSet<PeerId> = HashSet::new();
            
            for node in BOOTSTRAP_NODES.iter() {
                println!("Processing bootstrap node: {}", node);
                match node.parse::<Multiaddr>() {
                    Ok(addr) => {
                        if let Some(peer_id) = addr.iter().find_map(|p| {
                            if let libp2p::multiaddr::Protocol::P2p(hash) = p {
                                Some(PeerId::from(hash))
                            } else {
                                None
                            }
                        }) {
                            bootstrap_peer_ids.insert(peer_id);
                            
                            // Extract base address without peer ID and port
                            let mut base_addr = Multiaddr::empty();
                            let mut has_tcp = false;
                            let mut has_quic = false;
                            
                            for proto in addr.iter() {
                                match proto {
                                    libp2p::multiaddr::Protocol::Ip4(ip) => {
                                        base_addr.push(libp2p::multiaddr::Protocol::Ip4(ip));
                                        println!("Found IPv4 address: {}", ip);
                                    }
                                    libp2p::multiaddr::Protocol::Ip6(ip) => {
                                        base_addr.push(libp2p::multiaddr::Protocol::Ip6(ip));
                                        println!("Found IPv6 address: {}", ip);
                                    }
                                    libp2p::multiaddr::Protocol::Dns(host) => {
                                        base_addr.push(libp2p::multiaddr::Protocol::Dns(host.clone()));
                                        println!("Found DNS address: {}", host);
                                    }
                                    libp2p::multiaddr::Protocol::Dnsaddr(host) => {
                                        base_addr.push(libp2p::multiaddr::Protocol::Dns(host.clone()));
                                        println!("Found DNSAddr (converting to DNS): {}", host);
                                    }
                                    libp2p::multiaddr::Protocol::Tcp(port) => {
                                        has_tcp = true;
                                        base_addr.push(libp2p::multiaddr::Protocol::Tcp(port));
                                    }
                                    libp2p::multiaddr::Protocol::QuicV1 => {
                                        has_quic = true;
                                    }
                                    _ => {}
                                }
                            }

                            // Create dial addresses for all supported protocols
                            let mut dial_addrs = Vec::new();
                            
                            // For DNS addresses, use the original address
                            if base_addr.iter().any(|p| matches!(p, libp2p::multiaddr::Protocol::Dns(_) | libp2p::multiaddr::Protocol::Dnsaddr(_))) {
                                dial_addrs.push(addr.clone());
                            } else {
                                // For IP addresses, try standard ports
                                let mut tcp_addr = base_addr.clone();
                                tcp_addr.push(libp2p::multiaddr::Protocol::Tcp(4001));
                                tcp_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id.into()));
                                dial_addrs.push(tcp_addr);
                            }
                            
                            // Try original port if specified
                            if has_tcp {
                                dial_addrs.push(addr.clone());
                            }
                            
                            // Try QUIC if supported
                            if has_quic {
                                let mut quic_addr = base_addr.clone();
                                quic_addr.push(libp2p::multiaddr::Protocol::Udp(4001));
                                quic_addr.push(libp2p::multiaddr::Protocol::QuicV1);
                                quic_addr.push(libp2p::multiaddr::Protocol::P2p(peer_id.into()));
                                dial_addrs.push(quic_addr);
                            }
                            
                            // Add all addresses to Kademlia
                            for addr in &dial_addrs {
                                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                            }
                            
                            // Try dialing each address
                            for dial_addr in dial_addrs {
                                println!("Attempting to dial bootstrap node {} at {}", peer_id, dial_addr);
                                match swarm.dial(dial_addr.clone()) {
                                    Ok(_) => {
                                        println!("Successfully initiated connection to {} at {}", peer_id, dial_addr);
                                    }
                                    Err(e) => println!("Failed to dial {} at {}: {}", peer_id, dial_addr, e),
                                }
                            }
                            
                            // Small delay between connection attempts
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        } else {
                            println!("No peer ID found in multiaddr: {}", addr);
                        }
                    }
                    Err(e) => println!("Failed to parse bootstrap node address {}: {}", node, e),
                }
            }
            
            self.bootstrap_peers = Some(bootstrap_peer_ids);
            
            // Wait for initial connections
            println!("Waiting for initial connections...");
            let mut attempts = 0;
            while attempts < 30 {
                let connected = swarm.connected_peers().count();
                if connected > 0 {
                    println!("Successfully connected to {} peers", connected);
                    break;
                }
                
                // Process any pending events
                while let Poll::Ready(Some(event)) = futures::Stream::poll_next(Pin::new(&mut *swarm), &mut Context::from_waker(futures::task::noop_waker_ref())) {
                    match event {
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            println!("🌟 Connected to peer: {}", peer_id);
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                            println!("⚠️ Connection error to {:?}: {}", peer_id, error);
                        }
                        _ => {}
                    }
                }
                
                attempts += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
                if attempts % 5 == 0 {
                    println!("Still waiting for connections... ({}/30s)", attempts);
                }
            }
            
            // Start the bootstrap process if we have any connections
            if swarm.connected_peers().count() > 0 {
                println!("Starting DHT bootstrap...");
                swarm.behaviour_mut().kademlia.bootstrap()?;
            } else {
                println!("Warning: No connections established, bootstrap may fail");
            }
        }
        Ok(())
    }

    async fn wait_for_bootstrap(&mut self, timeout_secs: u64) -> Result<bool> {
        let start = SystemTime::now();
        let timeout = Duration::from_secs(timeout_secs);
        let mut bootstrap_attempted = false;
        let mut connected_peers = HashSet::new();
        
        if let Some(swarm) = &self.swarm {
            loop {
                if SystemTime::now().duration_since(start)? > timeout {
                    println!("Bootstrap timed out after {} seconds", timeout_secs);
                    return Ok(false);
                }

                let mut swarm = swarm.lock().await;
                
                // Track connected peers
                connected_peers.extend(swarm.connected_peers().cloned());
                
                if !bootstrap_attempted && !connected_peers.is_empty() {
                    println!("Starting DHT bootstrap with {} connected peers...", connected_peers.len());
                    swarm.behaviour_mut().kademlia.bootstrap()?;
                    bootstrap_attempted = true;
                }

                if let Poll::Ready(Some(event)) = futures::Stream::poll_next(Pin::new(&mut *swarm), &mut Context::from_waker(futures::task::noop_waker_ref())) {
                    match event {
                        SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Kademlia(
                            libp2p::kad::Event::OutboundQueryProgressed { result: libp2p::kad::QueryResult::Bootstrap(_), .. }
                        )) => {
                            if !connected_peers.is_empty() {
                                println!("Bootstrap completed successfully with {} peers", connected_peers.len());
                                return Ok(true);
                            }
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            println!("Connected to peer: {}", peer_id);
                            connected_peers.insert(peer_id);
                            
                            // Try to bootstrap again with new peer
                            if bootstrap_attempted {
                                println!("Retrying bootstrap with newly connected peer...");
                                swarm.behaviour_mut().kademlia.bootstrap()?;
                            }
                        }
                        _ => {}
                    }
                }
                
                // Give up CPU time
                drop(swarm);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Ok(false)
    }
}

/// Command line arguments
#[derive(Debug, clap::Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Command to run
    #[command(subcommand)]
    command: Option<Command>,
}

/// Available commands
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Run the daemon
    Daemon,

    /// Run the UI
    Ui,

    /// Mount a cluster
    Mount,

    /// Unmount a cluster
    Unmount,

    /// Cluster management commands
    #[command(subcommand)]
    Cluster(ClusterAction),
}

/// Cluster management actions
#[derive(Debug, clap::Subcommand, PartialEq)]
enum ClusterAction {
    /// Create a new cluster
    Create {
        /// Name of the cluster
        name: String,
    },

    /// Join an existing cluster
    Join {
        /// Name of the cluster
        cluster: String,

        /// Join ticket
        #[arg(short, long)]
        ticket: Option<String>,
    },

    /// Share a cluster with others
    Share {
        /// Name of the cluster to share
        cluster: String,
    },

    /// List available clusters
    List,
}

/// Parses CLI arguments and returns a CliCommand.
fn process_args(args: &[String]) -> CliCommand {
    let mut config = None;
    let mut pos_args = Vec::new();
    let mut iter = args.iter().skip(1).peekable();
    
    while let Some(arg) = iter.next() {
        if arg == "--help" || arg == "-h" {
            return CliCommand::Help;
        } else if arg == "--config" {
            if let Some(cfg) = iter.next() {
                config = Some(cfg.clone());
            } else {
                eprintln!("Error: --config requires a value.");
            }
        } else {
            pos_args.push(arg.clone());
        }
    }

    if pos_args.is_empty() {
        return CliCommand::Interactive { config };
    }

    match pos_args[0].as_str() {
        "cluster" | "clusters" => {
            if pos_args.len() <= 1 {
                return CliCommand::Cluster { action: ClusterAction::List, config };
            }

            match pos_args[1].as_str() {
                "create" => {
                    if pos_args.len() <= 2 {
                        eprintln!("Error: cluster create requires a name");
                        return CliCommand::Help;
                    }
                    CliCommand::Cluster { 
                        action: ClusterAction::Create { name: pos_args[2].clone() },
                        config 
                    }
                }
                "join" => {
                    if pos_args.len() <= 2 {
                        eprintln!("Error: cluster join requires a cluster name");
                        return CliCommand::Help;
                    }
                    let cluster = pos_args[2].clone();
                    let ticket = if pos_args.len() > 3 {
                        Some(pos_args[3].clone())
                    } else {
                        env::var("LIS_TICKET").ok()
                    };
                    CliCommand::Cluster { 
                        action: ClusterAction::Join { cluster, ticket },
                        config 
                    }
                }
                "share" => {
                    if pos_args.len() <= 2 {
                        eprintln!("Error: cluster share requires a cluster name");
                        return CliCommand::Help;
                    }
                    CliCommand::Cluster {
                        action: ClusterAction::Share { cluster: pos_args[2].clone() },
                        config
                    }
                }
                _ => CliCommand::Cluster { action: ClusterAction::List, config }
            }
        }
        "daemon" => CliCommand::Daemon { config },
        "mount"  => CliCommand::Mount { config },
        "unmount"=> CliCommand::Unmount { config },
        _ => {
            eprintln!("Unknown command: {}", pos_args[0]);
            CliCommand::Help
        }
    }
}

/// Prints the help message as described in the README
fn print_help() {
    println!("lis is a distributed filesystem!\n");
    println!("Usage: lis [OPTIONS] <COMMAND>\n");
    println!("Commands:");
    println!("  [no arguments]         Run Lis in CLI mode (interactive)");
    println!("  cluster create <name>  Create a new cluster");
    println!("  cluster join <name> [<ticket>]\n                         Join an existing cluster (ticket can be provided via LIS_TICKET env var)");
    println!("  cluster share <name>   Share a cluster and generate a join ticket");
    println!("  cluster                List clusters");
    println!("  daemon                 Run Lis in daemon mode");
    println!("  mount                  Mount a Lis filesystem");
    println!("  unmount                Unmount a Lis filesystem\n");
    println!("Options:");
    println!("  --config <CONFIG>      Path to the Lis configuration file, defaults to ~/.lis/config.toml");
}

fn unmount_fuse(mount_point: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        mount::unmount(mount_point, MntFlags::empty())?;
    }
    #[cfg(target_os = "linux")]
    {
        mount::umount(mount_point)?;
    }
    Ok(())
}

/// Main entrypoint
#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let args: Vec<String> = env::args().collect();
    match process_args(&args) {
        CliCommand::Help => {
            print_help();
            Ok(())
        },
        CliCommand::Interactive { config } => {
            let mut app_state = AppState::new(config)?;
            app_state.init_p2p().await?;
            run_interactive_with_state(app_state).await
        },
        CliCommand::Daemon { config } => run_daemon(config).await,
        CliCommand::Cluster { action, config } => run_cluster(action, config).await,
        CliCommand::Mount { config } => run_mount(config).await,
        CliCommand::Unmount { config } => run_unmount(config).await,
    }
}

async fn run_interactive_with_state(mut app_state: AppState) -> Result<()> {
    app_state.load_clusters()?;
    app_state.start_listening().await?;
    
    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    // Run the main loop
    let res = run_app(&mut terminal, &mut app_state).await;
    
    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    
    // Return any error that occurred
    res
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app_state: &mut AppState) -> Result<()> {
    loop {
        terminal.draw(|frame| draw_ui(frame, &app_state))?;
        
        // First check for keyboard events with a short timeout
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app_state.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Up => {
                            if let Some(selected) = app_state.selected_cluster {
                                if selected > 0 {
                                    app_state.selected_cluster = Some(selected - 1);
                                }
                            } else if !app_state.clusters.is_empty() {
                                app_state.selected_cluster = Some(0);
                            }
                        }
                        KeyCode::Down => {
                            if let Some(selected) = app_state.selected_cluster {
                                if selected < app_state.clusters.len().saturating_sub(1) {
                                    app_state.selected_cluster = Some(selected + 1);
                                }
                            } else if !app_state.clusters.is_empty() {
                                app_state.selected_cluster = Some(0);
                            }
                        }
                        KeyCode::Char('c') => {
                            app_state.input_mode = InputMode::Editing;
                            app_state.input_buffer.clear();
                            app_state.message = Some("Enter cluster name:".to_string());
                        }
                        _ => {}
                    },
                    InputMode::Editing => match key.code {
                        KeyCode::Enter => {
                            let name = app_state.input_buffer.trim().to_string();
                            if !name.is_empty() {
                                if let Err(e) = app_state.create_cluster(&name) {
                                    app_state.message = Some(format!("Error creating cluster: {}", e));
                                }
                            }
                            app_state.input_mode = InputMode::Normal;
                            app_state.input_buffer.clear();
                        }
                        KeyCode::Esc => {
                            app_state.input_mode = InputMode::Normal;
                            app_state.input_buffer.clear();
                            app_state.message = None;
                        }
                        KeyCode::Char(c) => {
                            app_state.input_buffer.push(c);
                        }
                        KeyCode::Backspace => {
                            app_state.input_buffer.pop();
                        }
                        _ => {}
                    }
                }
            }
        }
        
        // Then check for swarm events without blocking
        if let Some(swarm) = &app_state.swarm {
            let mut swarm = swarm.lock().await;
            if let Poll::Ready(Some(event)) = futures::Stream::poll_next(Pin::new(&mut *swarm), &mut Context::from_waker(futures::task::noop_waker_ref())) {
                // Drop the lock before handling the event
                drop(swarm);
                app_state.handle_swarm_event(event).await?;
            }
        }
    }
    
    Ok(())
}

/// Draw the interactive UI
fn draw_ui(frame: &mut Frame, app_state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(5),     // Clusters list
            Constraint::Length(3),  // Network status
            Constraint::Length(3),  // Help/Input
        ])
        .split(frame.size());

    // Title
    let title = Paragraph::new("Lis Distributed Filesystem")
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    // Clusters list
    let clusters: Vec<ListItem> = app_state.clusters
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if Some(i) == app_state.selected_cluster {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(name.as_str()).style(style)
        })
        .collect();

    let clusters_list = List::new(clusters)
        .block(Block::default().title("Clusters").borders(Borders::ALL));
    frame.render_widget(clusters_list, chunks[1]);

    // Network status
    let status_text = app_state.network_status.as_deref().unwrap_or("No network activity");
    let status_widget = Paragraph::new(status_text)
        .block(Block::default().title("Network Status").borders(Borders::ALL));
    frame.render_widget(status_widget, chunks[2]);

    // Help/Input area
    let bottom_text = match app_state.input_mode {
        InputMode::Normal => {
            app_state.message.as_deref().unwrap_or("Press: (q) Quit, (c) Create cluster, (↑/↓) Navigate")
        }
        InputMode::Editing => &app_state.input_buffer
    };
    
    let bottom_widget = Paragraph::new(bottom_text)
        .block(Block::default()
            .title(if app_state.input_mode == InputMode::Editing { "Input" } else { "Help" })
            .borders(Borders::ALL));
    frame.render_widget(bottom_widget, chunks[3]);
}

/// Network behavior for the Lis application
#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "LisNetworkBehaviourEvent")]
#[behaviour(event_process = false)]
struct LisNetworkBehaviour {
    kademlia: Kademlia<MemoryStore>,
    relay: libp2p_relay::Behaviour,
    dcutr: libp2p_dcutr::Behaviour,
    identify: identify::Behaviour,
}

#[derive(Debug)]
enum LisNetworkBehaviourEvent {
    Kademlia(libp2p::kad::Event),
    Relay(libp2p_relay::Event),
    Dcutr(libp2p_dcutr::Event),
    Identify(identify::Event),
}

impl From<libp2p::kad::Event> for LisNetworkBehaviourEvent {
    fn from(event: libp2p::kad::Event) -> Self {
        LisNetworkBehaviourEvent::Kademlia(event)
    }
}

impl From<libp2p_relay::Event> for LisNetworkBehaviourEvent {
    fn from(event: libp2p_relay::Event) -> Self {
        LisNetworkBehaviourEvent::Relay(event)
    }
}

impl From<libp2p_dcutr::Event> for LisNetworkBehaviourEvent {
    fn from(event: libp2p_dcutr::Event) -> Self {
        LisNetworkBehaviourEvent::Dcutr(event)
    }
}

impl From<identify::Event> for LisNetworkBehaviourEvent {
    fn from(event: identify::Event) -> Self {
        LisNetworkBehaviourEvent::Identify(event)
    }
}

/// Cluster state
#[derive(Debug, Clone)]
struct ClusterState {
    name: String,
    nodes: HashMap<PeerId, NodeInfo>,
    last_updated: u64,
}

/// Node information
#[derive(Debug, Clone)]
struct NodeInfo {
    peer_id: PeerId,
    addr: Multiaddr,
    status: NodeStatus,
    last_seen: u64,
    latency: Option<Duration>,
}

/// Node status
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum NodeStatus {
    Online,
    Offline,
    Degraded,
    Connecting,
}

/// Cluster status
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum ClusterStatus {
    Offline,
    Degraded,
    NoQuorum,
    Healthy,
    Connecting,
}

/// Implementation for cluster commands
async fn run_cluster(action: ClusterAction, config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config)?;
    app_state.init_p2p().await?;
    
    match action {
        ClusterAction::Create { name } => {
            app_state.create_cluster(&name)?;
            println!("Created cluster: {}", name);
        }
        ClusterAction::Join { cluster, ticket } => {
            println!("Joining cluster: {}", cluster);
            let ticket = if let Some(t) = ticket {
                t
            } else {
                env::var("LIS_TICKET").map_err(|_| eyre!("No ticket provided and LIS_TICKET not set"))?
            };

            match app_state.join_cluster(&cluster, &ticket).await {
                Ok(()) => {
                    println!("Successfully joined cluster '{}'", cluster);
                    println!("Connected to cluster host and synchronized configuration");
                }
                Err(e) => {
                    println!("Failed to join cluster: {}", e);
                }
            }
        }
        ClusterAction::Share { cluster } => {
            match app_state.generate_share_ticket(&cluster) {
                Ok(ticket) => {
                    println!("Generated share ticket for cluster '{}':", cluster);
                    println!("Ticket: {}", ticket);
                    println!("\nOthers can join using:");
                    println!("  lis cluster join {} {}", cluster, ticket);
                    println!("  # or");
                    println!("  LIS_TICKET={} lis cluster join {}", ticket, cluster);
                }
                Err(e) => {
                    println!("Error generating share ticket: {}", e);
                }
            }
        }
        ClusterAction::List => {
            app_state.load_clusters()?;
            if app_state.clusters.is_empty() {
                println!("No clusters found.");
            } else {
                println!("Available clusters:");
                for cluster in &app_state.clusters {
                    println!("  - {}", cluster);
                }
            }
        }
    }
    Ok(())
}

/// Implementation for mounting the filesystem
async fn run_mount(config: Option<String>) -> Result<()> {
    let app_state = AppState::new(config)?;
    println!("Mounting filesystem using config: {}", app_state.config_path.display());
    println!("Mounting filesystem... (not fully implemented)");
    Ok(())
}

/// Implementation for unmounting the filesystem
async fn run_unmount(config: Option<String>) -> Result<()> {
    let app_state = AppState::new(config)?;
    println!("Unmounting filesystem using config: {}", app_state.config_path.display());
    println!("Unmounting filesystem... (not fully implemented)");
    unmount_fuse(&app_state.config_path)?;
    Ok(())
}

/// Implementation for daemon mode
async fn run_daemon(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config)?;
    println!("Starting daemon with config: {}", app_state.config_path.display());
    
    // Initialize P2P networking
    app_state.init_p2p().await?;
    
    // Start listening on interfaces
    app_state.start_listening().await?;
    
    // Load all available clusters and their tickets
    app_state.load_clusters()?;
    let clusters_dir = app_state.config_path.parent().unwrap().join("clusters");
    
    let mut hosted_tickets = HashMap::new();
    let connected_peers: Arc<Mutex<HashSet<PeerId>>> = Arc::new(Mutex::new(HashSet::new()));
    
    // Load all tickets from all clusters
    for cluster in &app_state.clusters {
        let tickets_file = clusters_dir.join(cluster).join("tickets.toml");
        if tickets_file.exists() {
            if let Ok(content) = fs::read_to_string(&tickets_file) {
                if let Ok(tickets) = toml::from_str::<toml::Table>(&content) {
                    hosted_tickets.insert(cluster.clone(), tickets);
                    println!("Loaded tickets for cluster: {}", cluster);
                }
            }
        }
    }

    if app_state.clusters.is_empty() {
        println!("No clusters found in {}", clusters_dir.display());
    } else {
        println!("Found clusters:");
        for cluster in &app_state.clusters {
            println!("  - {}", cluster);
            if let Some(tickets) = hosted_tickets.get(cluster) {
                println!("    Active tickets: {}", tickets.len());
            }
        }
    }

    // Start hosting clusters
    if let Some(peer_id) = app_state.peer_id {
        println!("Daemon running with peer ID: {}", peer_id);
        
        // Create a channel for shutdown signal
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);
        let shutdown_tx_clone = shutdown_tx.clone();
        
        // Handle Ctrl+C
        tokio::spawn(async move {
            if let Ok(()) = tokio::signal::ctrl_c().await {
                println!("\nReceived Ctrl+C, initiating shutdown...");
                let _ = shutdown_tx_clone.send(()).await;
            }
        });

        // Periodically announce ourselves and print peer status
        let swarm_clone = app_state.swarm.clone();
        let clusters = app_state.clusters.clone();
        let connected_peers_clone = connected_peers.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15)); // More frequent updates
            loop {
                interval.tick().await;
                if let Some(swarm) = &swarm_clone {
                    let mut swarm = swarm.lock().await;
                    println!("\nPeriodic Status Update:");
                    let peers = connected_peers_clone.lock().await;
                    println!("Connected Peers: {}", peers.len());
                    for peer in peers.iter() {
                        println!("  - {}", peer);
                        // Try to maintain connection by refreshing the routing table entry
                        swarm.behaviour_mut().kademlia.get_closest_peers(*peer);
                    }
                    println!("Active Clusters:");
                    for cluster in &clusters {
                        let topic = format!("lis-cluster:{}", cluster);
                        println!("  - {} (announcing and searching)", cluster);
                        // Announce ourselves as a provider multiple times to increase visibility
                        let _ = swarm.behaviour_mut().kademlia.start_providing(topic.as_bytes().to_vec().into());
                        swarm.behaviour_mut().kademlia.get_providers(topic.as_bytes().to_vec().into());
                        
                        // Also try to discover new peers through bootstrap nodes
                        let _ = swarm.behaviour_mut().kademlia.bootstrap();
                        
                        // Try to discover peers from known peers
                        let connected = swarm.connected_peers().cloned().collect::<Vec<_>>();
                        for peer in connected {
                            swarm.behaviour_mut().kademlia.get_closest_peers(peer);
                        }
                    }
                    
                    // Try to connect to bootstrap nodes periodically
                    for node in BOOTSTRAP_NODES.iter() {
                        if let Ok(addr) = node.parse::<Multiaddr>() {
                            if let Some(peer_id) = addr.iter().find_map(|p| {
                                if let libp2p::multiaddr::Protocol::P2p(hash) = p {
                                    Some(PeerId::from(hash))
                                } else {
                                    None
                                }
                            }) {
                                let local_peer_id = *swarm.local_peer_id();
                                let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                if peer_id != local_peer_id && !connected_peers.contains(&peer_id) {
                                    println!("Attempting to connect to bootstrap node: {}", addr);
                                    let _ = swarm.dial(addr.clone());
                                }
                            }
                        }
                    }
                }
            }
        });

        // Keep track of connected peers for each cluster
        let mut cluster_peers: HashMap<String, HashSet<PeerId>> = HashMap::new();
        for cluster in &app_state.clusters {
            cluster_peers.insert(cluster.clone(), HashSet::new());
        }

        // Main event loop
        println!("Daemon is running. Press Ctrl+C to stop.");
        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = shutdown_rx.recv() => {
                    println!("Shutting down daemon...");
                    break;
                }
                
                // Handle swarm events
                event = async {
                    if let Some(swarm) = &app_state.swarm {
                        let mut swarm = swarm.lock().await;
                        if let Poll::Ready(Some(event)) = futures::Stream::poll_next(Pin::new(&mut *swarm), &mut Context::from_waker(futures::task::noop_waker_ref())) {
                            Some(event)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } => {
                    if let Some(event) = event {
                        match event {
                            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Kademlia(kad_event)) => {
                                match kad_event {
                                    libp2p::kad::Event::OutboundQueryProgressed { result, stats, .. } => {
                                        match result {
                                            libp2p::kad::QueryResult::Bootstrap(_) => {
                                                println!("🔄 Bootstrap progress: {} peers in {}ms", 
                                                    stats.num_successes(),
                                                    stats.duration().map_or(0, |d| d.as_millis())
                                                );
                                            }
                                            libp2p::kad::QueryResult::GetClosestPeers(Ok(ok)) => {
                                                println!("👥 Found {} close peers", ok.peers.len());
                                                if let Some(swarm) = &app_state.swarm {
                                                    let mut swarm = swarm.lock().await;
                                                    let local_peer_id = *swarm.local_peer_id();
                                                    let connected = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                                    for peer_info in &ok.peers {
                                                        let peer_id = peer_info.peer_id;
                                                        if peer_id != local_peer_id && !connected.contains(&peer_id) {
                                                            println!("Attempting to connect to peer: {}", peer_id);
                                                            let _ = swarm.dial(peer_id);
                                                        }
                                                    }
                                                }
                                            }
                                            libp2p::kad::QueryResult::GetProviders(Ok(ok)) => {
                                                match ok {
                                                    libp2p::kad::GetProvidersOk::FoundProviders { providers, .. } => {
                                                        if let Some(swarm) = &app_state.swarm {
                                                            let mut swarm = swarm.lock().await;
                                                            let local_peer_id = *swarm.local_peer_id();
                                                            let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                                            for provider in providers {
                                                                println!("Found provider: {}", provider);
                                                                let provider_id = provider.clone();
                                                                if provider_id != local_peer_id && !connected_peers.contains(&provider) {
                                                                    println!("Attempting to connect to provider: {}", provider);
                                                                    let _ = swarm.dial(provider_id);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    libp2p::kad::GetProvidersOk::FinishedWithNoAdditionalRecord { closest_peers } => {
                                                        println!("No providers found, but got {} closest peers", closest_peers.len());
                                                        if let Some(swarm) = &app_state.swarm {
                                                            let mut swarm = swarm.lock().await;
                                                            let local_peer_id = *swarm.local_peer_id();
                                                            let connected_peers = swarm.connected_peers().cloned().collect::<Vec<_>>();
                                                            for peer in closest_peers {
                                                                let peer_id = peer.clone();
                                                                if peer_id != local_peer_id && !connected_peers.contains(&peer) {
                                                                    println!("Attempting to connect to closest peer: {}", peer);
                                                                    let _ = swarm.dial(peer_id);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            SwarmEvent::Behaviour(LisNetworkBehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                                println!("🔍 Identified peer {} running {}", peer_id, info.protocol_version);
                                if let Some(addr) = info.listen_addrs.first() {
                                    println!("  📍 Peer {} is listening on {}", peer_id, addr);
                                    if let Some(swarm) = &app_state.swarm {
                                        let mut swarm = swarm.lock().await;
                                        swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                                    }
                                }
                            }
                            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                                println!("🌟 Connected to peer: {} at {}", peer_id, endpoint.get_remote_address());
                                if let Some(swarm) = &app_state.swarm {
                                    let mut swarm = swarm.lock().await;
                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, endpoint.get_remote_address().clone());
                                }
                            }
                            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                                println!("❌ Disconnected from peer: {} (cause: {:?})", peer_id, cause);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    } else {
        println!("Failed to initialize P2P networking");
    }
    
    Ok(())
}

/// FUSE filesystem implementation
struct LisFs {
    app_state: Arc<AppState>,
    cluster: String,
}

impl LisFs {
    fn new(app_state: AppState, cluster: String) -> Result<Self> {
        Ok(Self {
            app_state: Arc::new(app_state),
            cluster,
        })
    }
}

impl Filesystem for LisFs {
    fn lookup(&mut self, _req: &Request<'_>, _parent: u64, name: &OsStr, reply: ReplyEntry) {
        let path = PathBuf::from(name);
        match self.app_state.get_inode(&path) {
            Ok(inode) => {
                let ttl = Duration::from_secs(1);
                let attr = FileAttr {
                    ino: inode,
                    size: 0,
                    blocks: 0,
                    atime: SystemTime::now(),
                    mtime: SystemTime::now(),
                    ctime: SystemTime::now(),
                    crtime: SystemTime::now(),
                    kind: FileType::RegularFile,
                    perm: 0o644,
                    nlink: 1,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    flags: 0,
                    blksize: 512,
                };
                reply.entry(&ttl, &attr, 0);
            }
            Err(_e) => {
                reply.error(libc::ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyAttr) {
        let ttl = Duration::from_secs(1);
        let attr = FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            rdev: 0,
            flags: 0,
            blksize: 512,
        };
        reply.attr(&ttl, &attr);
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        match self.app_state.get_document(ino) {
            Ok(data) => {
                let start = offset as usize;
                let end = (offset as usize + size as usize).min(data.len());
                if start >= data.len() {
                    reply.data(&[]);
                } else {
                    reply.data(&data[start..end]);
                }
            }
            Err(_e) => {
                reply.error(libc::ENOENT);
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let entries = vec![
            (1, FileType::Directory, "."),
            (1, FileType::Directory, ".."),
        ];

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(entry.0, (i + 1) as i64, entry.1, entry.2) {
                break;
            }
        }
        reply.ok();
    }
}

async fn handle_stream<S>(mut stream: S) -> Result<()> 
where 
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
{
    let mut buf = vec![0u8; 1024];
    let request_bytes = b"request data";
    
    stream.write_all(request_bytes).await?;
    let n = stream.read(&mut buf).await?;
    
    if n > 0 {
        // Process buf[..n]
    }
    
    Ok(())
}

async fn handle_connection(addr: Multiaddr, mut transport: Boxed<(PeerId, StreamMuxerBox)>, _peer_id: PeerId) -> Result<()> {
    use libp2p::core::transport::DialOpts;
    use libp2p::core::connection::Endpoint;
    use libp2p::core::transport::PortUse;
    
    let dial_opts = DialOpts {
        role: Endpoint::Dialer,
        port_use: PortUse::New,
    };
    
    let future = transport.dial(addr, dial_opts)?;
    match future.await {
        Ok((_peer_id, mut connection)) => {
            if let Poll::Ready(Ok(substream)) = StreamMuxerExt::poll_outbound_unpin(&mut connection, &mut Context::from_waker(futures::task::noop_waker_ref())) {
                handle_stream(substream).await?;
            }
        }
        Err(e) => {
            eprintln!("Failed to dial: {}", e);
        }
    }
    
    Ok(())
}

fn create_noise_config(local_key: &identity::Keypair) -> std::io::Result<noise::Config> {
    noise::Config::new(local_key)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}