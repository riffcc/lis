use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dirs;
use env_logger;
use fuser::{FileAttr, FileType, Filesystem, MountOption, Request, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry};
use libp2p::{
    core::{
        muxing::StreamMuxerBox,
        transport::Boxed,
        upgrade,
    },
    futures::StreamExt,
    identity,
    kad::{
        store::MemoryStore,
        Behaviour as Kademlia,
    },
    noise,
    swarm::{NetworkBehaviour, Swarm, Config as SwarmConfig},
    tcp,
    yamux,
    Multiaddr,
    PeerId,
    Transport,
};
use nix::mount;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use redb::TableDefinition;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use toml;
use uuid::Uuid;
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use std::future::Future;
use std::pin::Pin;

const DOCUMENTS: TableDefinition<&str, &[u8]> = TableDefinition::new("documents");
const ROOT_DOC_KEY: &str = "root";
const NODE_TIMEOUT_SECS: u64 = 60;

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
#[derive(Clone)]
struct AppState {
    config_path: PathBuf,
    clusters: Arc<RwLock<HashSet<String>>>,
    peer_id: PeerId,
    transport: Option<TransportWrapper>,
    swarm: Option<SwarmWrapper>,
    cluster_status: Arc<RwLock<HashMap<String, ClusterStatus>>>,
    selected_cluster: Option<usize>,
    input_mode: InputMode,
}

impl AppState {
    async fn new(config: Option<String>) -> Result<Self> {
        let config_path = config.map(PathBuf::from).unwrap_or_else(|| {
            let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
            path.push("lis");
            path.push("config.toml");
            path
        });
        
        // Create config directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Generate peer ID
        let local_key = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(local_key.public());

        Ok(Self {
            config_path,
            clusters: Arc::new(RwLock::new(HashSet::new())),
            peer_id,
            transport: None,
            swarm: None,
            cluster_status: Arc::new(RwLock::new(HashMap::new())),
            selected_cluster: None,
            input_mode: InputMode::Normal,
        })
    }

    async fn init_p2p(&mut self, is_daemon: bool) -> Result<()> {
        let local_key = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(local_key.public());
        self.peer_id = peer_id;

        let transport = tcp::tokio::Transport::new(tcp::Config::default())
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&local_key)?)
            .multiplex(yamux::Config::default())
            .boxed();

        let transport_wrapper = TransportWrapper::new(transport);

        if is_daemon {
            // Set up Kademlia for the daemon
            let store = MemoryStore::new(peer_id);
            let behaviour = LisNetworkBehaviour {
                kademlia: Kademlia::new(peer_id, store),
            };
            
            // Create a new transport for the swarm
            let swarm_transport = tcp::tokio::Transport::new(tcp::Config::default())
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::Config::new(&local_key)?)
                .multiplex(yamux::Config::default())
                .boxed();

            let mut swarm = Swarm::new(
                swarm_transport,
                behaviour,
                peer_id,
                SwarmConfig::with_tokio_executor(),
            );

            // Listen on a fixed port for UI connections
            let addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse()?;
            swarm.listen_on(addr)?;

            self.swarm = Some(SwarmWrapper::new(swarm));
        }

        self.transport = Some(transport_wrapper);
        Ok(())
    }

    async fn load_clusters(&mut self) -> Result<()> {
        let mut clusters = self.clusters.write().await;
        clusters.clear();

        if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path)?;
            let config: toml::Value = toml::from_str(&content)?;

            if let Some(cluster_table) = config.get("clusters").and_then(|v| v.as_table()) {
                for name in cluster_table.keys() {
                    clusters.insert(name.clone());
                }
            }
        }

        Ok(())
    }

    async fn create_cluster(&mut self, name: &str) -> Result<()> {
        let mut clusters = self.clusters.write().await;
        clusters.insert(name.to_string());

        let config = if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path)?;
            let mut config: toml::Value = toml::from_str(&content)?;

            if let Some(cluster_table) = config.get_mut("clusters").and_then(|v| v.as_table_mut()) {
                cluster_table.insert(name.to_string(), toml::Value::Table(toml::Table::new()));
            } else {
                let mut cluster_table = toml::Table::new();
                cluster_table.insert(name.to_string(), toml::Value::Table(toml::Table::new()));
                config.as_table_mut().unwrap().insert("clusters".to_string(), toml::Value::Table(cluster_table));
            }

            config
        } else {
            let mut config = toml::Table::new();
            let mut cluster_table = toml::Table::new();
            cluster_table.insert(name.to_string(), toml::Value::Table(toml::Table::new()));
            config.insert("clusters".to_string(), toml::Value::Table(cluster_table));
            toml::Value::Table(config)
        };

        let content = toml::to_string_pretty(&config)?;
        fs::write(&self.config_path, content)?;

        Ok(())
    }

    async fn join_cluster(&mut self, cluster: String, ticket: String) -> Result<()> {
        // TODO: Implement cluster joining logic
        println!("Joining cluster {} with ticket {}", cluster, ticket);
        Ok(())
    }

    async fn get_cluster_status(&self, cluster: &str) -> Result<ClusterStatus> {
        // For now, just check if we have a connection to the daemon
        if let Some(swarm) = &self.swarm {
            Ok(ClusterStatus::Healthy)
        } else if let Some(transport) = &self.transport {
            // Try to connect to the daemon
            let addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse()?;
            let fut = transport.0.dial(addr, libp2p::core::transport::DialOpts::with_peer_id(self.peer_id))?;
            match fut.await {
                Ok((peer_id, mut connection)) => {
                    if let Ok(mut substream) = connection.open_outbound() {
                        // Handle stream
                        Ok(ClusterStatus::Healthy)
                    } else {
                        Ok(ClusterStatus::Offline)
                    }
                }
                Err(_) => Ok(ClusterStatus::Offline),
            }
        } else {
            Ok(ClusterStatus::Offline)
        }
    }

    fn get_inode(&self, _path: &Path) -> Result<u64> {
        // TODO: Implement proper inode mapping
        Ok(1)
    }

    fn get_document(&self, _inode: u64) -> Result<Vec<u8>> {
        // TODO: Implement document retrieval
        Ok(Vec::new())
    }

    async fn update_cluster_status(&mut self) -> Result<()> {
        let clusters = self.clusters.read().await;
        for cluster in clusters.iter() {
            let status = self.get_cluster_status(cluster).await?;
            self.cluster_status.write().await.insert(cluster.clone(), status);
        }
        Ok(())
    }

    fn handle_input(&mut self, key: event::KeyEvent) -> Result<()> {
        match self.input_mode {
            InputMode::Normal => {
                match key.code {
                    KeyCode::Char('q') => return Err(eyre!("quit")),
                    KeyCode::Char('i') => self.input_mode = InputMode::Editing,
                    KeyCode::Up => {
                        if let Some(selected) = self.selected_cluster {
                            if selected > 0 {
                                self.selected_cluster = Some(selected - 1);
                            }
                        } else {
                            self.selected_cluster = Some(0);
                        }
                    }
                    KeyCode::Down => {
                        if let Some(selected) = self.selected_cluster {
                            self.selected_cluster = Some(selected + 1);
                        } else {
                            self.selected_cluster = Some(0);
                        }
                    }
                    _ => {}
                }
            }
            InputMode::Editing => {
                match key.code {
                    KeyCode::Esc => self.input_mode = InputMode::Normal,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    async fn load_cluster_state(&self, _cluster_name: &str) -> Result<()> {
        // For now, just create an empty cluster state
        Ok(())
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
    println!("  cluster                List clusters");
    println!("  daemon                 Run Lis in daemon mode");
    println!("  mount                  Mount a Lis filesystem");
    println!("  unmount                Unmount a Lis filesystem\n");
    println!("Options:");
    println!("  --config <CONFIG>      Path to the Lis configuration file, defaults to ~/.lis/config.toml");
}

fn unmount_fuse(mount_point: &Path) -> Result<()> {
    use std::process::Command;
    
    // First try to kill any processes using the mount point
    let lsof_output = Command::new("lsof")
        .arg(mount_point)
        .output();

    if let Ok(output) = lsof_output {
        // Parse lsof output to get PIDs
        let output_str = String::from_utf8_lossy(&output.stdout);
        for line in output_str.lines().skip(1) { // Skip header line
            if let Some(pid_str) = line.split_whitespace().nth(1) {
                if let Ok(pid) = pid_str.parse::<i32>() {
                    // Try to kill the process
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                }
            }
        }
    }

    // Try fusermount first (Linux)
    let status = Command::new("fusermount")
        .arg("-u")
        .arg("-z") // Lazy unmount
        .arg(mount_point)
        .status();

    match status {
        Ok(exit) if exit.success() => Ok(()),
        _ => {
            // Try lazy unmount with umount
            let status = Command::new("umount")
                .arg("-l")
                .arg(mount_point)
                .status();
            
            match status {
                Ok(exit) if exit.success() => Ok(()),
                _ => {
                    // Last resort: force unmount
                    if let Err(e) = mount::umount(mount_point) {
                        Err(eyre!("Failed to unmount {}: {}", mount_point.display(), e))
                } else {
                        Ok(())
                    }
                }
            }
        }
    }
}

/// Main entrypoint
#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    env_logger::init();

    let args = Args::parse();

    match args.command {
        Some(Command::Daemon) => {
            let mut app_state = AppState::new(args.config).await?;
            app_state.init_p2p(true).await?;

            // Keep the daemon running until interrupted
            tokio::signal::ctrl_c().await?;
            println!("Shutting down daemon...");
        }
        Some(Command::Ui) => {
            let mut app_state = AppState::new(args.config).await?;
            app_state.init_p2p(false).await?;

            // Connect to the daemon
            let addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse()?;
            if let Some(transport) = &app_state.transport {
                let fut = transport.0.dial(addr, libp2p::core::transport::DialOpts::with_peer_id(app_state.peer_id))?;
                match fut.await {
                    Ok((peer_id, mut connection)) => {
                        if let Ok(mut substream) = connection.open_outbound() {
                            println!("Connected to daemon at {}", addr);
                        }
                    }
                    Err(e) => println!("Failed to connect to daemon: {}", e),
                }
            }

            // Load and display clusters
            app_state.load_clusters().await?;
            let clusters = app_state.clusters.read().await;
            if clusters.is_empty() {
                println!("No clusters found.");
            } else {
                println!("Available clusters:");
                for cluster in clusters.iter() {
                    let status = app_state.get_cluster_status(cluster).await?;
                    let status_str = match status {
                        ClusterStatus::Offline => "offline",
                        ClusterStatus::Degraded => "degraded",
                        ClusterStatus::NoQuorum => "no quorum",
                        ClusterStatus::Healthy => "healthy",
                        ClusterStatus::Connecting => "connecting",
                    };
                    println!("  - {} ({})", cluster, status_str);
                }
            }
        }
        Some(Command::Mount) => {
            run_mount(args.config).await?;
        }
        Some(Command::Unmount) => {
            run_unmount(args.config).await?;
        }
        Some(Command::Cluster(action)) => {
            run_cluster(action, args.config).await?;
        }
        None => {
            println!("No command specified. Use --help for usage information.");
        }
    }

    Ok(())
}

/// Run the interactive CLI mode using ratatui.
async fn run_interactive(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    app_state.load_clusters().await?;

    // Set up transport with yamux for multiplexing
    let local_key = identity::Keypair::generate_ed25519();
    let transport = tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&local_key)?)
        .multiplex(yamux::Config::default())
        .boxed();

    let transport_wrapper = TransportWrapper::new(transport);

    // Set up background task to maintain daemon connection and status updates
    let app_state_clone = app_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            
            // Try to connect to daemon
            let addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse().unwrap();
            
            if let Ok(fut) = transport_wrapper.0.dial(addr, libp2p::core::transport::DialOpts::with_peer_id(app_state_clone.peer_id)) {
                if let Ok((peer_id, mut connection)) = fut.await {
                    if let Ok(mut substream) = connection.open_outbound() {
                        // Send status request
                        let request = ClusterMessage::StatusRequest { 
                            peer_id: app_state_clone.peer_id 
                        };
                        if let Ok(request_bytes) = serde_json::to_vec(&request) {
                            if let Ok(()) = substream.write_all(&request_bytes).await {
                                // Read response
                                let mut buf = vec![0; 1024];
                                if let Ok(n) = substream.read(&mut buf).await {
                                    if let Ok(ClusterMessage::StatusResponse { clusters, .. }) = 
                                        serde_json::from_slice(&buf[..n]) {
                                        *app_state_clone.cluster_status.write().await = clusters;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_state_clone = app_state.clone();
    let result = run_app(&mut terminal, app_state_clone).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        println!("Error: {}", err);
        std::process::exit(1);
    }

    Ok(())
}

/// Implementation for daemon mode
async fn run_daemon(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    
    // Set up transport with yamux for multiplexing
    let local_key = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(local_key.public());
    app_state.peer_id = peer_id;

    let transport = tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&local_key)?)
        .multiplex(yamux::Config::default())
        .boxed();

    // Create listener
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse()?;
    let mut swarm = {
        let store = MemoryStore::new(peer_id);
        let behaviour = LisNetworkBehaviour {
            kademlia: Kademlia::new(peer_id, store),
        };
        let mut swarm = Swarm::new(
            transport,
            behaviour,
            peer_id,
            SwarmConfig::with_tokio_executor(),
        );
        swarm.listen_on(addr)?;
        swarm
    };
    println!("Listening for UI connections on 127.0.0.1:33033");
    
    // Load all existing clusters
    app_state.load_clusters().await?;
    let clusters = app_state.clusters.read().await.clone();
    
    if clusters.is_empty() {
        println!("No clusters found. Create one first with 'lis cluster create <name>'");
        return Ok(());
    }

    println!("Initializing clusters:");
    for cluster_name in clusters {
        println!("  - Loading cluster: {}", cluster_name);
        
        // Initialize cluster state
        if let Err(e) = app_state.load_cluster_state(&cluster_name).await {
            eprintln!("Error loading cluster {}: {}", cluster_name, e);
            continue;
        }

        // Initialize cluster state with a single node (self)
        let mut cluster_state = ClusterState {
            name: cluster_name.clone(),
            nodes: HashMap::new(),
            last_updated: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_secs(),
        };

        // Add self as a node
        let node_addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse()?;
        cluster_state.nodes.insert(app_state.peer_id, NodeInfo {
            peer_id: app_state.peer_id,
            addr: node_addr,
            status: NodeStatus::Online,
            last_seen: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_secs(),
            latency: None,
        });

        // Update cluster status
        let mut status = app_state.cluster_status.write().await;
        status.insert(cluster_name.clone(), ClusterStatus::Healthy);
    }

    let app_state = Arc::new(app_state);
    let _app_state_clone = Arc::clone(&app_state);

    // Handle UI client connections
    tokio::spawn(async move {
        loop {
            if let Some(event) = swarm.next().await {
                match event {
                    // Handle swarm events
                    _ => {}
                }
            }
        }
    });

    println!("\nDaemon running. Press Ctrl+C to stop.");
    println!("Use 'lis mount' in another terminal to mount clusters.");
    
    // Set up Ctrl-C handler
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    
    // Keep the daemon running until Ctrl+C
    tokio::select! {
        _ = sigint.recv() => {
            println!("\nShutting down...");
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutting down...");
        }
    }

    Ok(())
}

/// Helper function to create a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ].as_ref())
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Messages exchanged between nodes in a cluster
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum ClusterMessage {
    StatusRequest {
        peer_id: PeerId,
    },
    StatusResponse {
        clusters: HashMap<String, ClusterStatus>,
        peer_id: PeerId,
    },
}

impl ClusterMessage {
    fn cluster_name(&self) -> String {
        match self {
            ClusterMessage::StatusRequest { .. } => "status".to_string(),
            ClusterMessage::StatusResponse { .. } => "status".to_string(),
        }
    }
}

fn parse_token(token: &str) -> Result<(Multiaddr, String)> {
    let parts: Vec<&str> = token.split('@').collect();
    if parts.len() != 2 {
        return Err(eyre!("Invalid token format - expected format: <cluster_id>@<multiaddr>"));
    }

    let cluster_id = parts[0].to_string();
    let addr: Multiaddr = parts[1].parse()
        .map_err(|_| eyre!("Invalid multiaddr in token"))?;

    Ok((addr, cluster_id))
}

#[derive(Debug, Clone)]
struct Cluster {
    id: String,
    name: String,
    dir: PathBuf,
    peers: HashSet<PeerId>,
    status: ClusterStatus,
    nodes: HashMap<PeerId, NodeInfo>,
    last_updated: u64,
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut app_state: AppState,
) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    
    // Set up Ctrl-C handler
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    
    loop {
        // Draw UI
        terminal.draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ].as_ref())
                .split(size);

            // Draw title
            let title = Paragraph::new("Lis Distributed Filesystem")
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(title, chunks[0]);

            // Draw clusters list
            let items: Vec<ListItem> = if let Ok(clusters) = app_state.clusters.try_read() {
                clusters
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let status = app_state.cluster_status
                            .try_read()
                            .ok()
                            .and_then(|status| {
                                status.get(name).cloned()
                            })
                            .unwrap_or(ClusterStatus::Offline);
                        let status_str = match status {
                            ClusterStatus::Offline => "offline",
                            ClusterStatus::Degraded => "degraded",
                            ClusterStatus::NoQuorum => "no quorum",
                            ClusterStatus::Healthy => "healthy",
                            ClusterStatus::Connecting => "connecting",
                        };
                        let selected = app_state.selected_cluster == Some(i);
                        let style = if selected {
                            Style::default().fg(Color::Yellow)
                        } else {
                            Style::default()
                        };
                        ListItem::new(format!("{} ({})", name, status_str)).style(style)
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let clusters_list = List::new(items)
                .block(Block::default().title("Clusters").borders(Borders::ALL))
                .highlight_style(Style::default().fg(Color::Yellow));
            f.render_widget(clusters_list, chunks[1]);

            // Draw status bar
            let status = match app_state.input_mode {
                InputMode::Normal => "Press 'i' to enter input mode, 'q' to quit",
                InputMode::Editing => "Press Esc to exit input mode",
            };
            let status_bar = Paragraph::new(status)
                .alignment(ratatui::layout::Alignment::Left);
            f.render_widget(status_bar, chunks[2]);
        })?;

        tokio::select! {
            _ = interval.tick() => {
                app_state.update_cluster_status().await?;
            }
            _ = sigint.recv() => {
                break;
            }
            Ok(event) = tokio::task::spawn_blocking(|| crossterm::event::read()) => {
                if let Ok(event) = event {
                    if let crossterm::event::Event::Key(key) = event {
                        if key.code == KeyCode::Char('c') && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                            break;
                        }
                        if let Err(e) = app_state.handle_input(key) {
                            eprintln!("Error handling input: {}", e);
                            break;
                        }
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// Network behavior for the Lis application
#[derive(NetworkBehaviour)]
struct LisNetworkBehaviour {
    kademlia: Kademlia<MemoryStore>,
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
    let mut app_state = AppState::new(config).await?;
    app_state.init_p2p(false).await?;
    
    match action {
        ClusterAction::Create { name } => {
            app_state.create_cluster(&name).await?;
            println!("Created cluster: {}", name);
        }
        ClusterAction::Join { cluster, ticket } => {
            if let Some(token) = ticket {
                app_state.join_cluster(cluster, token).await?;
            } else if let Ok(token) = env::var("LIS_TICKET") {
                app_state.join_cluster(cluster, token).await?;
            } else {
                return Err(eyre!("No join ticket provided. Use --ticket or set LIS_TICKET environment variable."));
            }
        }
        ClusterAction::List => {
            app_state.load_clusters().await?;
            let clusters = app_state.clusters.read().await;
            if clusters.is_empty() {
                println!("No clusters found.");
            } else {
                println!("Available clusters:");
                for cluster in clusters.iter() {
                    let status = app_state.get_cluster_status(cluster).await?;
                    let status_str = match status {
                        ClusterStatus::Offline => "offline",
                        ClusterStatus::Degraded => "degraded",
                        ClusterStatus::NoQuorum => "no quorum",
                        ClusterStatus::Healthy => "healthy",
                        ClusterStatus::Connecting => "connecting",
                    };
                    println!("  - {} ({})", cluster, status_str);
                }
            }
        }
    }
    Ok(())
}

/// Implementation for mounting the filesystem
async fn run_mount(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    
    // Initialize basic P2P
    let local_key = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(local_key.public());
    app_state.peer_id = peer_id;

    let _transport = tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&local_key)?)
        .multiplex(yamux::Config::default())
        .boxed();

    // Connect directly to the local daemon
    let daemon_addr: Multiaddr = "/ip4/127.0.0.1/tcp/33033".parse()?;
    println!("Connecting to daemon at {}", daemon_addr);
    
    // Get mount point
    print!("Enter mount point: ");
    std::io::stdout().flush()?;
    
    let mut mount_point = String::new();
    std::io::stdin().read_line(&mut mount_point)?;
    let mount_point = PathBuf::from(mount_point.trim());
    
    // Create mount point if it doesn't exist
    if !mount_point.exists() {
        fs::create_dir_all(&mount_point)?;
    }
    
    // Initialize FUSE filesystem
    let fs = LisFs::new(app_state, "default".to_string())?;
    
    // Mount the filesystem
    let options = vec![MountOption::RO, MountOption::FSName("lis".to_string())];
    println!("Mounting filesystem...");
    fuser::mount2(fs, &mount_point, &options)?;
    
    Ok(())
}

/// Implementation for unmounting the filesystem
async fn run_unmount(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    app_state.init_p2p(false).await?;
    println!("Unmounting filesystem using config: {}", app_state.config_path.display());
    println!("Unmounting filesystem... (not fully implemented)");
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
    S: AsyncRead + AsyncWrite + AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
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

async fn handle_connection(addr: Multiaddr, transport: Boxed<(PeerId, StreamMuxerBox)>, peer_id: PeerId) -> Result<()> {
    let fut = transport.dial(addr, libp2p::core::transport::DialOpts::with_peer_id(peer_id))?;
    
    match fut.await {
        Ok((peer_id, mut connection)) => {
            if let Ok(mut substream) = connection.open_outbound() {
                handle_stream(substream).await?;
            }
        }
        Err(e) => {
            eprintln!("Failed to dial: {}", e);
        }
    }
    
    Ok(())
}