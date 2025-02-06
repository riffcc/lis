use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
    task::{Context, Poll},
};

use clap::Parser;
use color_eyre::{eyre::Context as EyreContext, Result, eyre::eyre};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dirs;
use env_logger;
use fuser::{FileAttr, FileType, Filesystem, MountOption, Request, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry};
use libp2p::{
    core::{
        muxing::{StreamMuxerBox, StreamMuxerExt},
        transport::Boxed,
        upgrade,
        StreamMuxer,
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
    Transport,
};
use nix::mount::{self, MntFlags};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
    Frame,
};
use redb::TableDefinition;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use toml;
use uuid::Uuid;
use futures::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use std::future::Future;
use std::pin::Pin;
use futures::StreamExt;

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
struct AppState {
    config_path: PathBuf,
    clusters: Vec<String>,
    selected_cluster: Option<usize>,
    message: Option<String>,
    peer_id: Option<PeerId>,
    swarm: Option<Arc<Mutex<Swarm<LisNetworkBehaviour>>>>,
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
            peer_id: None,
            swarm: None,
        })
    }

    async fn init_p2p(&mut self) -> Result<()> {
        // Create a random PeerId
        let local_key = identity::Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(local_key.public());
        self.peer_id = Some(local_peer_id);

        // Create a transport with noise encryption and yamux multiplexing
        let transport = tcp::tokio::Transport::new(tcp::Config::default())
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&local_key)?)
            .multiplex(yamux::Config::default())
            .boxed();

        // Create a Kademlia behaviour
        let store = MemoryStore::new(local_peer_id);
        let behaviour = LisNetworkBehaviour {
            kademlia: Kademlia::new(local_peer_id, store),
        };

        // Create a Swarm
        let swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            libp2p::swarm::Config::with_tokio_executor(),
        );
        
        self.swarm = Some(Arc::new(Mutex::new(swarm)));
        Ok(())
    }

    async fn start_listening(&mut self) -> Result<()> {
        if let Some(swarm) = &self.swarm {
            let mut swarm = swarm.lock().await;
            swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
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

    fn handle_swarm_event(&mut self, event: SwarmEvent<LisNetworkBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                self.message = Some(format!("Listening on {}", address));
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                self.message = Some(format!("Connected to {}", peer_id));
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                self.message = Some(format!("Disconnected from {}", peer_id));
            }
            _ => {}
        }
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
    mount::unmount(mount_point, MntFlags::empty())?;
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
        CliCommand::Daemon { config } => {
            let mut app_state = AppState::new(config)?;
            app_state.init_p2p().await?;
            tokio::signal::ctrl_c().await?;
            println!("Shutting down daemon...");
            Ok(())
        },
        CliCommand::Cluster { action, config } => run_cluster(action, config).await,
        CliCommand::Mount { config } => run_mount(config).await,
        CliCommand::Unmount { config } => run_unmount(config).await,
    }
}

async fn run_interactive_with_state(mut app_state: AppState) -> Result<()> {
    app_state.load_clusters()?;
    app_state.start_listening().await?;
    
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    
    loop {
        terminal.draw(|frame| draw_ui(frame, &app_state))?;
        
        let mut event_received = None;
        if let Some(swarm) = &app_state.swarm {
            let mut swarm = swarm.lock().await;
            let stream = futures::stream::poll_fn(|cx| swarm.poll_next_unpin(cx));
            tokio::pin!(stream);
            
            if let Some(event) = stream.next().await {
                event_received = Some(event);
            }
        }
        
        if let Some(event) = event_received {
            app_state.handle_swarm_event(event);
        }
        
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
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
                        let name = format!("cluster_{}", app_state.clusters.len());
                        if let Err(e) = app_state.create_cluster(&name) {
                            app_state.message = Some(format!("Error creating cluster: {}", e));
                        }
                    }
                    _ => {}
                }
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
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
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

    // Status/help message
    let help_text = if let Some(ref msg) = app_state.message {
        msg.as_str()
    } else {
        "Press: (q) Quit, (c) Create cluster, (↑/↓) Navigate"
    };
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(help, chunks[2]);
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
    let mut app_state = AppState::new(config)?;
    app_state.init_p2p().await?;
    
    match action {
        ClusterAction::Create { name } => {
            app_state.create_cluster(&name)?;
            println!("Created cluster: {}", name);
        }
        ClusterAction::Join { cluster, ticket } => {
            println!("Joining cluster: {}", cluster);
            if let Some(t) = ticket {
                println!("Using ticket: {}", t);
            } else {
                println!("No ticket provided, attempting to read from environment variable LIS_TICKET.");
            }
            // TODO: Implement actual cluster joining
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

async fn handle_connection(addr: Multiaddr, mut transport: Boxed<(PeerId, StreamMuxerBox)>, peer_id: PeerId) -> Result<()> {
    use libp2p::core::{transport::{DialOpts, PortUse}, connection::Endpoint};
    let dial_opts = DialOpts {
        role: Endpoint::Dialer,
        port_use: PortUse::New,
    };
    let fut = transport.dial(addr, dial_opts)?;
    
    match fut.await {
        Ok((peer_id, mut connection)) => {
            use futures::task::noop_waker_ref;
            if let Poll::Ready(Ok(substream)) = StreamMuxerExt::poll_outbound_unpin(&mut connection, &mut Context::from_waker(noop_waker_ref())) {
                handle_stream(substream).await?;
            }
        }
        Err(e) => {
            eprintln!("Failed to dial: {}", e);
        }
    }
    
    Ok(())
}