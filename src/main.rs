use std::env;
use std::time::{Duration, UNIX_EPOCH, SystemTime};
use std::path::PathBuf;
use std::fs;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::RwLock;
use libp2p::{
    core::upgrade,
    futures::StreamExt,
    identity,
    kad::{
        store::MemoryStore,
        Mode as KademliaMode,
        QueryResult,
        Event as KademliaEvent,
        Behaviour as KademliaProtocol,
        Record,
        GetRecordOk,
        PeerRecord,
    },
    noise,
    swarm::{NetworkBehaviour, SwarmEvent, Config as SwarmConfig},
    tcp,
    PeerId,
    Multiaddr,
    yamux,
    Swarm,
    Transport,
};

use color_eyre::{Result, eyre::eyre};
use crossterm::{
    execute,
    event::{self, Event, KeyCode, KeyModifiers, EnableMouseCapture, DisableMouseCapture},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    widgets::{Paragraph, Block, Borders, List, ListItem},
    layout::{Layout, Direction, Constraint, Rect},
    style::{Style, Color},
    Terminal,
    Frame,
};
use blake3;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use redb::{Database, TableDefinition};
use fuser::{
    FileType, FileAttr, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEntry, Request, FUSE_ROOT_ID,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::fmt;
use libc::ENOENT;

const DOCUMENTS: TableDefinition<&str, &[u8]> = TableDefinition::new("documents");
const ROOT_DOC_KEY: &str = "root";
const NODE_TIMEOUT_SECS: u64 = 60;

// Document types for MerkleDAG structure
#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, PartialEq)]
enum ClusterAction {
    Create { name: String },
    Join { cluster: String, ticket: Option<String> },
    List,
}

#[derive(Debug)]
struct AppState {
    config_path: PathBuf,
    clusters: Vec<String>,
    selected_cluster: Option<usize>,
    message: Option<String>,
    root_doc: Option<RootDoc>,
    db: Option<Database>,
    p2p_node: Option<Arc<P2PNode>>,
    cluster_status: HashMap<String, ClusterStatus>,
    show_status: bool,
    status_scroll: usize,
}

impl AppState {
    async fn new(config: Option<String>) -> Result<Self> {
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

        Ok(Self {
            config_path,
            clusters: Vec::new(),
            selected_cluster: None,
            show_status: false,
            p2p_node: None,
            message: None,
            cluster_status: HashMap::new(),
            status_scroll: 0,
            root_doc: None,
            db: None,
        })
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

    fn load_cluster_state(&mut self, cluster_name: &str) -> Result<()> {
        let cluster_dir = self.config_path.parent().unwrap()
            .join("clusters")
            .join(cluster_name);
        
        // Open ReDB database
        let db_path = cluster_dir.join("cluster.db");
        let db = Database::create(db_path)?;
        let read_txn = db.begin_read()?;
        let table = read_txn.open_table(DOCUMENTS)?;
        
        // Load root document
        if let Some(root_doc_bytes) = table.get(ROOT_DOC_KEY)? {
            let root_doc: RootDoc = serde_json::from_slice(root_doc_bytes.value())?;
            self.root_doc = Some(root_doc);
        }

        self.db = Some(db);
        Ok(())
    }

    fn save_cluster_state(&self, _cluster_name: &str) -> Result<()> {
        if let Some(db) = &self.db {
            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(DOCUMENTS)?;
                
                // Save root document
                if let Some(root_doc) = &self.root_doc {
                    table.insert(ROOT_DOC_KEY, serde_json::to_vec(root_doc)?.as_slice())?;
                }
            }
            write_txn.commit()?;
        }
        Ok(())
    }

    fn validate_cluster_name(name: &str) -> Result<()> {
        // Check for empty name
        if name.is_empty() {
            return Err(eyre!("Cluster name cannot be empty"));
        }

        // Check length
        if name.len() > 255 {
            return Err(eyre!("Cluster name too long (max 255 characters)"));
        }

        // Check for valid characters
        let valid_chars = name.chars().all(|c| {
            c.is_alphanumeric() || c == '-' || c == '_'
        });

        if !valid_chars {
            return Err(eyre!("Cluster name can only contain alphanumeric characters, hyphens, and underscores"));
        }

        // Check that it doesn't start with a hyphen or underscore
        if name.starts_with('-') || name.starts_with('_') {
            return Err(eyre!("Cluster name must start with a letter or number"));
        }

        Ok(())
    }

    async fn init_p2p(&mut self) -> Result<()> {
        let p2p_node = P2PNode::new().await?;
        self.p2p_node = Some(Arc::new(p2p_node));
        
        if let Some(node) = &self.p2p_node {
            let node_clone = Arc::clone(node);
            tokio::spawn(async move {
                node_clone.run_network_loop().await;
            });
        }
        
        Ok(())
    }

    async fn create_cluster(&mut self, name: &str) -> Result<()> {
        // Create cluster directory
        let cluster_dir = self.config_path.parent().unwrap()
            .join("clusters")
            .join(name);
        fs::create_dir_all(&cluster_dir)?;

        // Create and initialize the database
        let db_path = cluster_dir.join("cluster.db");
        let db = Database::create(db_path)?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(DOCUMENTS)?;

            // Create empty children document for root directory
            let children = ChildrenDoc { entries: Vec::new() };
            let children_bytes = serde_json::to_vec(&children)?;
            let children_id = DocumentId::new(&children_bytes);
            table.insert(children_id.0.as_str(), children_bytes.as_slice())?;

            // Create root directory metadata
            let metadata = MetadataDoc {
                name: "/".to_string(),
                doc_type: DocType::Directory,
                size: 0,
                inode_uuid: Uuid::new_v4(),
                modified: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                hash: None,
            };
            let metadata_bytes = serde_json::to_vec(&metadata)?;
            let metadata_id = DocumentId::new(&metadata_bytes);
            table.insert(metadata_id.0.as_str(), metadata_bytes.as_slice())?;

            // Create root directory document
            let directory = DirectoryDoc {
                metadata: metadata_id,
                children: children_id,
            };
            let directory_bytes = serde_json::to_vec(&directory)?;
            let directory_id = DocumentId::new(&directory_bytes);
            table.insert(directory_id.0.as_str(), directory_bytes.as_slice())?;

            // Create empty inode map
            let inode_map = InodeMapDoc {
                inode_to_doc: HashMap::new(),
                doc_to_inode: HashMap::new(),
            };
            let inode_map_bytes = serde_json::to_vec(&inode_map)?;
            let inode_map_id = DocumentId::new(&inode_map_bytes);
            table.insert(inode_map_id.0.as_str(), inode_map_bytes.as_slice())?;

            // Create root document
            let root_doc = RootDoc {
                inode_map: inode_map_id,
                top_level_directory: directory_id,
            };
            let root_doc_bytes = serde_json::to_vec(&root_doc)?;
            table.insert(ROOT_DOC_KEY, root_doc_bytes.as_slice())?;
        }
        write_txn.commit()?;

        // Store database and root document in app state
        self.db = Some(db);
        if let Some(db) = &self.db {
            let read_txn = db.begin_read()?;
            let table = read_txn.open_table(DOCUMENTS)?;
            if let Some(root_doc_bytes) = table.get(ROOT_DOC_KEY)? {
                self.root_doc = Some(serde_json::from_slice(root_doc_bytes.value())?);
            }
        }

        // Initialize P2P node if needed
        if let Some(node) = &self.p2p_node {
            let token = node.share_token()?;
            self.clusters.push(name.to_string());
            self.message = Some(format!("Share this token: {}", token));
        }

        Ok(())
    }

    async fn join_cluster(&mut self, name: &str, token: &str) -> Result<()> {
        if let Some(node) = &mut self.p2p_node {
            let node = Arc::get_mut(node).ok_or_else(|| eyre!("Failed to get mutable reference to P2PNode"))?;
            node.join_cluster(name, token).await?;
            self.message = Some(format!("Joined cluster: {}", name));
            self.load_clusters()?;
        }
        Ok(())
    }

    async fn update_cluster_status(&mut self) -> Result<()> {
        if let Some(node) = &self.p2p_node {
            let states = node.clusters.read().await;
            for (name, state) in states.iter() {
                let online_count = state.nodes.values()
                    .filter(|n| n.status == NodeStatus::Online)
                    .count();
                let total_count = state.nodes.len();

                if !self.clusters.contains(name) {
                    self.clusters.push(name.clone());
                }

                let status = match (online_count, total_count) {
                    (0, _) => ClusterStatus::Offline,
                    (n, t) if n < t => ClusterStatus::Degraded,
                    (n, t) if n < t * 2/3 => ClusterStatus::NoQuorum,
                    _ => ClusterStatus::Healthy,
                };

                self.cluster_status.insert(name.clone(), status);
            }
        }
        Ok(())
    }

    async fn get_cluster_nodes(&self, name: &str) -> Result<Vec<NodeInfo>> {
        if let Some(node) = &self.p2p_node {
            node.get_cluster_nodes(name).await
        } else {
            Ok(Vec::new())
        }
    }

    fn get_document(&self, id: &DocumentId) -> Result<Vec<u8>> {
        if let Some(db) = &self.db {
            let read_txn = db.begin_read()?;
            let table = read_txn.open_table(DOCUMENTS)?;
            if let Some(content) = table.get(id.0.as_str())? {
                return Ok(content.value().to_vec());
            }
        }
        Err(eyre!("Document not found: {:?}", id))
    }

    fn put_document(&mut self, content: Vec<u8>) -> Result<DocumentId> {
        let id = DocumentId::new(&content);
        if let Some(db) = &self.db {
            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(DOCUMENTS)?;
                table.insert(id.0.as_str(), content.as_slice())?;
            }
            write_txn.commit()?;
        }
        Ok(id)
    }

    pub fn create_file(&mut self, parent_dir_id: &DocumentId, name: &str, content: Vec<u8>) -> Result<DocumentId> {
        // Create file metadata
        let metadata = MetadataDoc {
            name: name.to_string(),
            doc_type: DocType::File,
            size: content.len() as u64,
            inode_uuid: Uuid::new_v4(),
            modified: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            hash: Some(hex::encode(blake3::hash(&content).as_bytes())),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)?;
        let metadata_id = self.put_document(metadata_bytes)?;

        // Create file document
        let file_doc = FileDoc {
            metadata: metadata_id.clone(),
            chunks: vec![self.put_document(content)?],
        };
        let file_doc_bytes = serde_json::to_vec(&file_doc)?;
        let file_doc_id = self.put_document(file_doc_bytes)?;

        // Update parent directory
        let parent_dir_bytes = self.get_document(parent_dir_id)?;
        let parent_dir: DirectoryDoc = serde_json::from_slice(&parent_dir_bytes)?;
        
        let children_bytes = self.get_document(&parent_dir.children)?;
        let mut children: ChildrenDoc = serde_json::from_slice::<ChildrenDoc>(&children_bytes)?;
        
        children.entries.push(DirectoryEntry::File {
            name: name.to_string(),
            file_doc: file_doc_id.clone(),
        });
        
        let children_bytes = serde_json::to_vec(&children)?;
        let children_id = self.put_document(children_bytes)?;
        
        let updated_dir = DirectoryDoc {
            metadata: parent_dir.metadata,
            children: children_id,
        };
        let updated_dir_bytes = serde_json::to_vec(&updated_dir)?;

        // Update the parent directory ID in the database
        if let Some(db) = &self.db {
            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(DOCUMENTS)?;
                table.insert(parent_dir_id.0.as_str(), &*updated_dir_bytes)?;
            }
            write_txn.commit()?;
        }

        // Update inode map
        if let Some(root_doc) = &self.root_doc {
            let inode_map_id = root_doc.inode_map.clone();
            let inode_map_bytes = self.get_document(&inode_map_id)?;
            let mut inode_map: InodeMapDoc = serde_json::from_slice(&inode_map_bytes)?;
            
            inode_map.inode_to_doc.insert(metadata.inode_uuid, file_doc_id.clone());
            inode_map.doc_to_inode.insert(file_doc_id.clone(), metadata.inode_uuid);
            
            let inode_map_bytes = serde_json::to_vec(&inode_map)?;

            // Update the inode map ID in the database
            if let Some(db) = &self.db {
                let write_txn = db.begin_write()?;
                {
                    let mut table = write_txn.open_table(DOCUMENTS)?;
                    table.insert(inode_map_id.0.as_str(), &*inode_map_bytes)?;
                }
                write_txn.commit()?;
            }
        }

        Ok(file_doc_id)
    }

    async fn get_cluster_status(&self, cluster: &str) -> Result<ClusterStatus> {
        if let Some(node) = &self.p2p_node {
            let states = node.clusters.read().await;
            if let Some(state) = states.get(cluster) {
                let online_count = state.nodes.values()
                    .filter(|n| n.status == NodeStatus::Online)
                    .count();
                let total_count = state.nodes.len();

                match (online_count, total_count) {
                    (0, _) => Ok(ClusterStatus::Offline),
                    (n, t) if n < t => Ok(ClusterStatus::Degraded),
                    (n, t) if n < t * 2/3 => Ok(ClusterStatus::NoQuorum),
                    _ => Ok(ClusterStatus::Healthy),
                }
            } else {
                Ok(ClusterStatus::Offline)
            }
        } else {
            Ok(ClusterStatus::Offline)
        }
    }
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
            if pos_args.len() > 1 {
                match pos_args[1].as_str() {
                    "create" => {
                        if pos_args.len() > 2 {
                            return CliCommand::Cluster { 
                                action: ClusterAction::Create { name: pos_args[2].clone() },
                                config 
                            };
                        } else {
                            eprintln!("Error: cluster create requires a name");
                            return CliCommand::Help;
                        }
                    }
                    "join" => {
                        if pos_args.len() > 2 {
                            let cluster = pos_args[2].clone();
                            let ticket = if pos_args.len() > 3 {
                                Some(pos_args[3].clone())
                            } else {
                                env::var("LIS_TICKET").ok()
                            };
                            return CliCommand::Cluster { 
                                action: ClusterAction::Join { cluster, ticket },
                                config 
                            };
                        } else {
                            eprintln!("Error: cluster join requires a cluster name");
                            return CliCommand::Help;
                        }
                    }
                    _ => return CliCommand::Cluster { action: ClusterAction::List, config },
                }
            } else {
                return CliCommand::Cluster { action: ClusterAction::List, config };
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

    if app_state.show_status {
        draw_cluster_status(frame, app_state, chunks[1]);
    } else {
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
                
                // Add status color indicator
                let status_color = app_state.cluster_status.get(name)
                    .map(|status| match status {
                        ClusterStatus::Offline => Color::Red,
                        ClusterStatus::Degraded => Color::LightRed,
                        ClusterStatus::NoQuorum => Color::Yellow,
                        ClusterStatus::Healthy => Color::Green,
                    })
                    .unwrap_or(Color::White);
                
                let text = format!("● {}", name);
                ListItem::new(text).style(style.fg(status_color))
            })
            .collect();

        let clusters_list = List::new(clusters)
            .block(Block::default().title("Clusters").borders(Borders::ALL));
        frame.render_widget(clusters_list, chunks[1]);
    }

    // Status/help message
    let help_text = if let Some(ref msg) = app_state.message {
        msg.as_str()
    } else if app_state.show_status {
        "Press: (q) Back, (↑/↓) Scroll"
    } else {
        "Press: (q) Quit, (c) Create cluster, (s) Share cluster, (Enter) Show status, (↑/↓) Navigate"
    };
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(help, chunks[2]);
}

fn draw_cluster_status(frame: &mut Frame, app_state: &AppState, area: Rect) {
    if let Some(selected) = app_state.selected_cluster {
        let cluster_name = &app_state.clusters[selected];
        
        // Get cluster status
        let status = app_state.cluster_status.get(cluster_name)
            .cloned()
            .unwrap_or(ClusterStatus::Offline);
            
        let status_str = match status {
            ClusterStatus::Offline => "Offline",
            ClusterStatus::Degraded => "Degraded",
            ClusterStatus::NoQuorum => "No Quorum",
            ClusterStatus::Healthy => "Healthy",
        };
        
        let status_color = match status {
            ClusterStatus::Offline => Color::Red,
            ClusterStatus::Degraded => Color::LightRed,
            ClusterStatus::NoQuorum => Color::Yellow,
            ClusterStatus::Healthy => Color::Green,
        };
        
        // Create status text
        let mut text = vec![
            format!("Cluster: {}", cluster_name),
            format!("Status: {}", status_str),
            String::new(),
            "Nodes:".to_string(),
        ];
        
        // Add node information
        if let Ok(nodes) = tokio::runtime::Handle::current().block_on(app_state.get_cluster_nodes(cluster_name)) {
            for node in nodes {
                let node_status = match node.status {
                    NodeStatus::Online => "Online",
                    NodeStatus::Offline => "Offline",
                    NodeStatus::Degraded => "Degraded",
                };
                
                text.push(format!(
                    "  {} - {} - Last seen: {}s ago - Latency: {:?}",
                    node.peer_id,
                    node_status,
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        .saturating_sub(node.last_seen),
                    node.latency.map(|d| d.as_millis()).unwrap_or(0)
                ));
            }
        }
        
        let status_text = text.join("\n");
        let paragraph = Paragraph::new(status_text)
            .block(Block::default().title("Cluster Status").borders(Borders::ALL))
            .style(Style::default().fg(status_color))
            .scroll((app_state.status_scroll as u16, 0));
            
        frame.render_widget(paragraph, area);
    }
}

/// Run the interactive CLI mode using ratatui.
async fn run_interactive(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    app_state.init_p2p().await?;
    app_state.load_clusters()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, app_state).await;

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

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app_state: AppState,
) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut creating_cluster = false;
    let mut cluster_name_input = String::new();
    let mut sharing_cluster = false;

    loop {
        terminal.draw(|f| ui(f, &app_state, &creating_cluster, &cluster_name_input, &sharing_cluster))?;

        tokio::select! {
            _ = interval.tick() => {
                app_state.update_cluster_status().await?;
            }
            Ok(event) = tokio::task::spawn_blocking(|| crossterm::event::read()) => {
                if let Ok(Event::Key(key)) = event {
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }

                    if creating_cluster {
                        match key.code {
                            KeyCode::Enter => {
                                if !cluster_name_input.is_empty() {
                                    app_state.create_cluster(&cluster_name_input).await?;
                                    creating_cluster = false;
                                    cluster_name_input.clear();
                                }
                            }
                            KeyCode::Esc => {
                                creating_cluster = false;
                                cluster_name_input.clear();
                            }
                            KeyCode::Char(c) => {
                                cluster_name_input.push(c);
                            }
                            KeyCode::Backspace => {
                                cluster_name_input.pop();
                            }
                            _ => {}
                        }
                    } else if sharing_cluster {
                        match key.code {
                            KeyCode::Esc => {
                                sharing_cluster = false;
                            }
                            _ => {}
                        }
                    } else if app_state.show_status {
                        match key.code {
                            KeyCode::Char('q') => {
                                app_state.show_status = false;
                            }
                            _ => {}
                        }
                    } else {
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
                                creating_cluster = true;
                            }
                            KeyCode::Char('s') => {
                                if app_state.selected_cluster.is_some() {
                                    sharing_cluster = true;
                                }
                            }
                            KeyCode::Enter => {
                                if app_state.selected_cluster.is_some() {
                                    app_state.show_status = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Draw the cluster creation UI
fn draw_create_cluster_ui(frame: &mut Frame, input: &str) {
    let area = centered_rect(60, 20, frame.size());
    
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .margin(1)
        .split(area);

    let title = Paragraph::new("Create New Cluster")
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let input = Paragraph::new(input)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("Enter cluster name (Enter to confirm, Esc to cancel)"));
    frame.render_widget(input, chunks[1]);
}

/// Draw the cluster sharing UI
fn draw_share_cluster_ui(frame: &mut Frame, app_state: &AppState) {
    let area = centered_rect(60, 20, frame.size());
    
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .margin(1)
        .split(area);

    let title = Paragraph::new("Share Cluster")
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let selected_cluster = app_state.selected_cluster
        .and_then(|i| app_state.clusters.get(i));
        
    let content = if let Some(_) = selected_cluster {
        if let Some(node) = &app_state.p2p_node {
            match node.share_token() {
                Ok(token) => format!("Share this token: {}", token),
                Err(_) => "Failed to generate share token".to_string(),
            }
        } else {
            "P2P node not initialized".to_string()
        }
    } else {
        "No cluster selected".to_string()
    };

    let message = Paragraph::new(content)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("Press Esc to close"));
    frame.render_widget(message, chunks[1]);
}

/// Helper function to create a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
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
        CliCommand::Interactive { config } => run_interactive(config).await,
        CliCommand::Daemon { config } => run_daemon(config).await,
        CliCommand::Cluster { action, config } => run_cluster(action, config).await,
        CliCommand::Mount { config } => run_mount(config).await,
        CliCommand::Unmount { config } => run_unmount(config).await,
    }
}

/// Implementation for daemon mode
async fn run_daemon(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    app_state.init_p2p().await?;
    println!("Daemon mode using config: {}", app_state.config_path.display());
    println!("Running daemon mode... (not fully implemented)");
    
    // Keep the daemon running
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Implementation for cluster commands
async fn run_cluster(action: ClusterAction, config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    app_state.init_p2p().await?;
    
    match action {
        ClusterAction::Create { name } => {
            app_state.create_cluster(&name).await?;
            println!("Created cluster: {}", name);
        }
        ClusterAction::Join { cluster, ticket } => {
            if let Some(token) = ticket {
                app_state.join_cluster(&cluster, &token).await?;
            } else if let Ok(token) = env::var("LIS_TICKET") {
                app_state.join_cluster(&cluster, &token).await?;
            } else {
                return Err(eyre!("No join ticket provided. Use --ticket or set LIS_TICKET environment variable."));
            }
        }
        ClusterAction::List => {
            app_state.load_clusters()?;
            if app_state.clusters.is_empty() {
                println!("No clusters found.");
            } else {
                println!("Available clusters:");
                for cluster in &app_state.clusters {
                    let status = app_state.get_cluster_status(cluster).await?;
                    let status_str = match status {
                        ClusterStatus::Offline => "offline",
                        ClusterStatus::Degraded => "degraded",
                        ClusterStatus::NoQuorum => "no quorum",
                        ClusterStatus::Healthy => "healthy",
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
    app_state.init_p2p().await?;
    
    // List available clusters and prompt user to select one
    let clusters = app_state.clusters.clone();
    if clusters.is_empty() {
        return Err(eyre!("No clusters available. Create one first with 'lis cluster create <name>'"));
    }
    
    println!("Available clusters:");
    for (i, cluster) in clusters.iter().enumerate() {
        let status = app_state.get_cluster_status(cluster).await?;
        let status_str = match status {
            ClusterStatus::Offline => "offline",
            ClusterStatus::Degraded => "degraded",
            ClusterStatus::NoQuorum => "no quorum",
            ClusterStatus::Healthy => "healthy",
        };
        println!("  {}. {} ({})", i + 1, cluster, status_str);
    }
    
    print!("Select cluster number (1-{}): ", clusters.len());
    std::io::stdout().flush()?;
    
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let selection = input.trim().parse::<usize>()
        .map_err(|_| eyre!("Invalid selection"))
        .and_then(|n| {
            if n > 0 && n <= clusters.len() {
                Ok(n - 1)
            } else {
                Err(eyre!("Selection out of range"))
            }
        })?;
    
    let cluster_name = clusters[selection].clone();
    
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
    let fs = LisFs::new(app_state, cluster_name)?;
    
    // Mount the filesystem
    let options = vec![MountOption::RO, MountOption::FSName("lis".to_string())];
    fuser::mount2(fs, &mount_point, &options)?;
    
    Ok(())
}

/// Implementation for unmounting the filesystem
async fn run_unmount(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config).await?;
    app_state.init_p2p().await?;
    println!("Unmounting filesystem using config: {}", app_state.config_path.display());
    println!("Unmounting filesystem... (not fully implemented)");
    Ok(())
}

struct LisFs {
    app_state: AppState,
    inode_to_attr: HashMap<u64, FileAttr>,
}

impl LisFs {
    fn new(mut app_state: AppState, cluster_name: String) -> Result<Self> {
        app_state.load_cluster_state(&cluster_name)?;
        Ok(LisFs {
            app_state,
            inode_to_attr: HashMap::new(),
        })
    }

    fn get_attr_from_metadata(&self, metadata: &MetadataDoc) -> FileAttr {
        let now = SystemTime::now();
        FileAttr {
            ino: metadata.inode_uuid.as_u64_pair().0,
            size: metadata.size,
            blocks: (metadata.size + 511) / 512,
            atime: now,
            mtime: UNIX_EPOCH + Duration::from_secs(metadata.modified),
            ctime: now,
            crtime: now,
            kind: match metadata.doc_type {
                DocType::Directory => FileType::Directory,
                DocType::File => FileType::RegularFile,
            },
            perm: 0o755,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }
}

impl Filesystem for LisFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let parent_id = if parent == FUSE_ROOT_ID {
            if let Some(root_doc) = &self.app_state.root_doc {
                &root_doc.top_level_directory
            } else {
                reply.error(ENOENT);
                return;
            }
        } else {
            // TODO: Implement lookup for non-root directories
            reply.error(ENOENT);
            return;
        };

        // Get parent directory
        let parent_dir_bytes = match self.app_state.get_document(parent_id) {
            Ok(bytes) => bytes,
            Err(_) => {
                reply.error(ENOENT);
                return;
            }
        };

        let parent_dir: DirectoryDoc = match serde_json::from_slice(&parent_dir_bytes) {
            Ok(dir) => dir,
            Err(_) => {
                reply.error(ENOENT);
                return;
            }
        };

        // Get children
        let children_bytes = match self.app_state.get_document(&parent_dir.children) {
            Ok(bytes) => bytes,
            Err(_) => {
                reply.error(ENOENT);
                return;
            }
        };

        let children: ChildrenDoc = match serde_json::from_slice::<ChildrenDoc>(&children_bytes) {
            Ok(children) => children,
            Err(_) => {
                reply.error(ENOENT);
                return;
            }
        };

        // Find matching entry
        let name = name.to_str().unwrap_or("");
        for entry in children.entries {
            match entry {
                DirectoryEntry::File { name: ref entry_name, file_doc } if entry_name == name => {
                    let file_bytes = match self.app_state.get_document(&file_doc) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    };
                    let file: FileDoc = match serde_json::from_slice::<FileDoc>(&file_bytes) {
                        Ok(file) => file,
                        Err(_) => continue,
                    };
                    let metadata_bytes = match self.app_state.get_document(&file.metadata) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    };
                    let metadata: MetadataDoc = match serde_json::from_slice::<MetadataDoc>(&metadata_bytes) {
                        Ok(metadata) => metadata,
                        Err(_) => continue,
                    };
                    let attr = self.get_attr_from_metadata(&metadata);
                    reply.entry(&Duration::from_secs(1), &attr, 0);
                    return;
                }
                DirectoryEntry::Folder { name: ref entry_name, directory_doc } if entry_name == name => {
                    let dir_bytes = match self.app_state.get_document(&directory_doc) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    };
                    let dir: DirectoryDoc = match serde_json::from_slice::<DirectoryDoc>(&dir_bytes) {
                        Ok(dir) => dir,
                        Err(_) => continue,
                    };
                    let metadata_bytes = match self.app_state.get_document(&dir.metadata) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    };
                    let metadata: MetadataDoc = match serde_json::from_slice::<MetadataDoc>(&metadata_bytes) {
                        Ok(metadata) => metadata,
                        Err(_) => continue,
                    };
                    let attr = self.get_attr_from_metadata(&metadata);
                    reply.entry(&Duration::from_secs(1), &attr, 0);
                    return;
                }
                _ => continue,
            }
        }
        reply.error(ENOENT);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        if ino == FUSE_ROOT_ID {
            if let Some(root_doc) = &self.app_state.root_doc {
                if let Ok(dir_bytes) = self.app_state.get_document(&root_doc.top_level_directory) {
                    if let Ok(dir) = serde_json::from_slice::<DirectoryDoc>(&dir_bytes) {
                        if let Ok(metadata_bytes) = self.app_state.get_document(&dir.metadata) {
                            if let Ok(metadata) = serde_json::from_slice::<MetadataDoc>(&metadata_bytes) {
                                let attr = self.get_attr_from_metadata(&metadata);
                                reply.attr(&Duration::from_secs(1), &attr);
                                return;
                            }
                        }
                    }
                }
            }
        }

        // For non-root inodes, look up in the inode map
        if let Some(attr) = self.inode_to_attr.get(&ino) {
            reply.attr(&Duration::from_secs(1), attr);
            return;
        }

        reply.error(ENOENT);
    }

    fn read(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, size: u32, _flags: i32, _lock_owner: Option<u64>, reply: ReplyData) {
        // Get the document ID for this inode from the inode map
        if let Some(root_doc) = &self.app_state.root_doc {
            if let Ok(inode_map_bytes) = self.app_state.get_document(&root_doc.inode_map) {
                if let Ok(inode_map) = serde_json::from_slice::<InodeMapDoc>(&inode_map_bytes) {
                    let inode_uuid = Uuid::from_u64_pair(ino, 0);
                    if let Some(doc_id) = inode_map.inode_to_doc.get(&inode_uuid) {
                        // Get the file document
                        if let Ok(file_bytes) = self.app_state.get_document(doc_id) {
                            if let Ok(file) = serde_json::from_slice::<FileDoc>(&file_bytes) {
                                // Get the file content from chunks
                                if let Some(chunk_id) = file.chunks.first() {
                                    if let Ok(content) = self.app_state.get_document(chunk_id) {
                                        let content_len = content.len() as i64;
                                        if offset < content_len {
                                            let start = offset as usize;
                                            let end = std::cmp::min(
                                                (offset + size as i64) as usize,
                                                content.len()
                                            );
                                            reply.data(&content[start..end]);
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        reply.error(ENOENT);
    }

    fn readdir(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
        if ino != FUSE_ROOT_ID {
            reply.error(ENOENT);
            return;
        }

        if let Some(root_doc) = &self.app_state.root_doc {
            if let Ok(dir_bytes) = self.app_state.get_document(&root_doc.top_level_directory) {
                if let Ok(dir) = serde_json::from_slice::<DirectoryDoc>(&dir_bytes) {
                    if let Ok(children_bytes) = self.app_state.get_document(&dir.children) {
                        if let Ok(children) = serde_json::from_slice::<ChildrenDoc>(&children_bytes) {
                            let mut entries = vec![
                                (FUSE_ROOT_ID, FileType::Directory, ".".to_string()),
                                (FUSE_ROOT_ID, FileType::Directory, "..".to_string()),
                            ];

                            for entry in children.entries {
                                match entry {
                                    DirectoryEntry::File { name, file_doc } => {
                                        if let Ok(file_bytes) = self.app_state.get_document(&file_doc) {
                                            if let Ok(file) = serde_json::from_slice::<FileDoc>(&file_bytes) {
                                                if let Ok(metadata_bytes) = self.app_state.get_document(&file.metadata) {
                                                    if let Ok(metadata) = serde_json::from_slice::<MetadataDoc>(&metadata_bytes) {
                                                        entries.push((
                                                            metadata.inode_uuid.as_u64_pair().0,
                                                            FileType::RegularFile,
                                                            name,
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    DirectoryEntry::Folder { name, directory_doc } => {
                                        if let Ok(dir_bytes) = self.app_state.get_document(&directory_doc) {
                                            if let Ok(dir) = serde_json::from_slice::<DirectoryDoc>(&dir_bytes) {
                                                if let Ok(metadata_bytes) = self.app_state.get_document(&dir.metadata) {
                                                    if let Ok(metadata) = serde_json::from_slice::<MetadataDoc>(&metadata_bytes) {
                                                        entries.push((
                                                            metadata.inode_uuid.as_u64_pair().0,
                                                            FileType::Directory,
                                                            name,
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            for (i, (ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
                                if reply.add(ino, (i + 1) as i64, kind, &name) {
                                    break;
                                }
                            }

                            reply.ok();
                            return;
                        }
                    }
                }
            }
        }

        reply.error(ENOENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_test_cluster() -> Result<(TempDir, AppState)> {
        let temp_dir = TempDir::new().unwrap();
        let mut app = AppState::new(Some(temp_dir.path().join("config.toml").to_string_lossy().into_owned())).await?;
        app.create_cluster("test_cluster").await?;
        Ok((temp_dir, app))
    }

    #[tokio::test]
    async fn test_cluster_creation() -> Result<()> {
        let (_temp_dir, app) = setup_test_cluster().await?;
        assert!(app.root_doc.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_file() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_cluster().await?;
        let top_level_directory = app.root_doc.as_ref().unwrap().top_level_directory.clone();
        let file_id = app.create_file(&top_level_directory, "test.txt", b"Hello, World!".to_vec())?;
        
        // Verify file exists
        let file_bytes = app.get_document(&file_id)?;
        let file: FileDoc = serde_json::from_slice(&file_bytes)?;
        let content = app.get_document(&file.chunks[0])?;
        assert_eq!(content, b"Hello, World!".to_vec());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_multiple_files() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_cluster().await?;
        let top_level_directory = app.root_doc.as_ref().unwrap().top_level_directory.clone();
        
        // Create multiple files
        app.create_file(&top_level_directory, "file1.txt", b"Content 1".to_vec())?;
        app.create_file(&top_level_directory, "file2.txt", b"Content 2".to_vec())?;
        app.create_file(&top_level_directory, "file3.txt", b"Content 3".to_vec())?;

        // Verify directory contents
        let dir_bytes = app.get_document(&top_level_directory)?;
        let dir: DirectoryDoc = serde_json::from_slice(&dir_bytes)?;
        let children_bytes = app.get_document(&dir.children)?;
        let children: ChildrenDoc = serde_json::from_slice::<ChildrenDoc>(&children_bytes)?;

        let file_names: Vec<String> = children.entries.iter().filter_map(|entry| {
            match entry {
                DirectoryEntry::File { name, .. } => Some(name.clone()),
                _ => None,
            }
        }).collect();

        assert_eq!(file_names.len(), 3);
        assert!(file_names.contains(&"file1.txt".to_string()));
        assert!(file_names.contains(&"file2.txt".to_string()));
        assert!(file_names.contains(&"file3.txt".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_large_file() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_cluster().await?;
        let top_level_directory = app.root_doc.as_ref().unwrap().top_level_directory.clone();
        
        // Create a large file (1MB)
        let large_content = vec![42u8; 1024 * 1024];
        let file_id = app.create_file(&top_level_directory, "large.bin", large_content.clone())?;

        // Verify file content
        let file_bytes = app.get_document(&file_id)?;
        let file: FileDoc = serde_json::from_slice(&file_bytes)?;
        let content = app.get_document(&file.chunks[0])?;
        assert_eq!(content, large_content);
        Ok(())
    }

    #[tokio::test]
    async fn test_inode_mapping() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_cluster().await?;
        let top_level_directory = app.root_doc.as_ref().unwrap().top_level_directory.clone();
        let inode_map_id = app.root_doc.as_ref().unwrap().inode_map.clone();
        
        // Create a file
        let file_id = app.create_file(&top_level_directory, "test.txt", b"Test content".to_vec())?;

        // Verify inode mapping
        let inode_map_bytes = app.get_document(&inode_map_id)?;
        let inode_map: InodeMapDoc = serde_json::from_slice(&inode_map_bytes)?;

        // Check that the file is in the inode map
        assert!(inode_map.doc_to_inode.contains_key(&file_id));
        let inode_uuid = inode_map.doc_to_inode[&file_id];
        assert_eq!(inode_map.inode_to_doc[&inode_uuid], file_id);
        Ok(())
    }

    #[tokio::test]
    async fn test_file_metadata() -> Result<()> {
        let (_temp_dir, mut app) = setup_test_cluster().await?;
        let top_level_directory = app.root_doc.as_ref().unwrap().top_level_directory.clone();
        
        let content = b"Test content".to_vec();
        let file_id = app.create_file(&top_level_directory, "test.txt", content.clone())?;

        // Get file metadata
        let file_bytes = app.get_document(&file_id)?;
        let file: FileDoc = serde_json::from_slice(&file_bytes)?;
        let metadata_bytes = app.get_document(&file.metadata)?;
        let metadata: MetadataDoc = serde_json::from_slice(&metadata_bytes)?;

        assert_eq!(metadata.name, "test.txt");
        assert_eq!(metadata.doc_type, DocType::File);
        assert_eq!(metadata.size, content.len() as u64);
        assert!(metadata.hash.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_root_directory_structure() -> Result<()> {
        let (_temp_dir, app) = setup_test_cluster().await?;
        let root_doc = app.root_doc.as_ref().unwrap();

        // Verify root directory structure
        let dir_bytes = app.get_document(&root_doc.top_level_directory)?;
        let dir: DirectoryDoc = serde_json::from_slice(&dir_bytes)?;
        
        // Check metadata
        let metadata_bytes = app.get_document(&dir.metadata)?;
        let metadata: MetadataDoc = serde_json::from_slice(&metadata_bytes)?;
        assert_eq!(metadata.name, "/");
        assert_eq!(metadata.doc_type, DocType::Directory);

        // Check children
        let children_bytes = app.get_document(&dir.children)?;
        let children: ChildrenDoc = serde_json::from_slice::<ChildrenDoc>(&children_bytes)?;
        assert!(children.entries.is_empty()); // Root directory should start empty
        Ok(())
    }
}

// P2P types
#[derive(NetworkBehaviour)]
struct LisNetworkBehaviour {
    kademlia: KademliaProtocol<MemoryStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClusterState {
    name: String,
    nodes: HashMap<PeerId, NodeInfo>,
    last_updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeInfo {
    peer_id: PeerId,
    addr: String,
    status: NodeStatus,
    last_seen: u64,
    latency: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum NodeStatus {
    Online,
    Offline,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ClusterStatus {
    Offline,
    Degraded,
    NoQuorum,
    Healthy,
}

impl fmt::Debug for P2PNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("P2PNode")
            .field("peer_id", &self.peer_id)
            .field("clusters", &self.clusters)
            .finish()
    }
}

struct P2PNode {
    swarm: Swarm<LisNetworkBehaviour>,
    peer_id: PeerId,
    clusters: Arc<RwLock<HashMap<String, ClusterState>>>,
}

impl P2PNode {
    async fn new() -> Result<Self> {
        let local_key = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(local_key.public());

        let transport = tcp::tokio::Transport::new(tcp::Config::default())
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&local_key)?)
            .multiplex(yamux::Config::default())
            .boxed();

        let mut swarm = Swarm::new(
            transport,
            LisNetworkBehaviour {
                kademlia: {
                    let mut kad = KademliaProtocol::new(peer_id, MemoryStore::new(peer_id));
                    kad.set_mode(Some(KademliaMode::Server));
                    kad
                },
            },
            peer_id,
            SwarmConfig::with_tokio_executor(),
        );

        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

        Ok(Self {
            swarm,
            peer_id,
            clusters: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    async fn create_cluster(&self, name: &str) -> Result<String> {
        let mut clusters = self.clusters.write().await;
        let state = ClusterState {
            name: name.to_string(),
            nodes: {
                let mut nodes = HashMap::new();
                nodes.insert(self.peer_id, NodeInfo {
                    peer_id: self.peer_id,
                    addr: self.swarm.listeners().next().unwrap().to_string(),
                    status: NodeStatus::Online,
                    last_seen: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    latency: None,
                });
                nodes
            },
            last_updated: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        clusters.insert(name.to_string(), state);

        let token = format!("{}:{}", self.peer_id, self.swarm.listeners().next().unwrap());
        Ok(STANDARD.encode(token))
    }

    async fn join_cluster(&mut self, name: &str, token: &str) -> Result<()> {
        let decoded = String::from_utf8(STANDARD.decode(token)?)?;
        let mut parts = decoded.split(':');
        let bootstrap_peer = parts.next().ok_or_else(|| eyre!("Invalid token format"))?.parse()?;
        let bootstrap_addr: Multiaddr = parts.next().ok_or_else(|| eyre!("Invalid token format"))?.parse()?;

        // Add the bootstrap peer to the routing table
        self.swarm.behaviour_mut().kademlia.add_address(&bootstrap_peer, bootstrap_addr.clone());

        // Dial the bootstrap peer
        self.swarm.dial(bootstrap_addr.clone())?;

        // Start the bootstrap process
        self.swarm.behaviour_mut().kademlia.bootstrap()?;

        // Create initial cluster state
        let mut clusters = self.clusters.write().await;
        let state = ClusterState {
            name: name.to_string(),
            nodes: {
                let mut nodes = HashMap::new();
                nodes.insert(self.peer_id, NodeInfo {
                    peer_id: self.peer_id,
                    addr: self.swarm.listeners().next().unwrap().to_string(),
                    status: NodeStatus::Online,
                    last_seen: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    latency: None,
                });
                nodes.insert(bootstrap_peer, NodeInfo {
                    peer_id: bootstrap_peer,
                    addr: bootstrap_addr.to_string(),
                    status: NodeStatus::Online,
                    last_seen: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    latency: None,
                });
                nodes
            },
            last_updated: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        clusters.insert(name.to_string(), state);

        Ok(())
    }

    fn share_token(&self) -> Result<String> {
        let listeners = self.swarm.listeners().collect::<Vec<_>>();
        if let Some(addr) = listeners.first() {
            let token = format!("{}:{}", self.peer_id, addr);
            Ok(STANDARD.encode(token))
        } else {
            Err(eyre!("No active listeners"))
        }
    }

    async fn get_cluster_nodes(&self, cluster: &str) -> Result<Vec<NodeInfo>> {
        let clusters = self.clusters.read().await;
        if let Some(state) = clusters.get(cluster) {
            Ok(state.nodes.values().cloned().collect())
        } else {
            Ok(Vec::new())
        }
    }

    async fn handle_swarm_event(&self, event: LisNetworkBehaviourEvent) -> Result<(), Box<dyn std::error::Error>> {
        match event {
            LisNetworkBehaviourEvent::Kademlia(kad_event) => {
                if let KademliaEvent::OutboundQueryProgressed { result, .. } = kad_event {
                    if let QueryResult::GetRecord(Ok(get_record)) = result {
                        match get_record {
                            GetRecordOk::FoundRecord(PeerRecord { record, .. }) => {
                                let value = String::from_utf8_lossy(&record.value);
                                let mut clusters = self.clusters.write().await;
                                clusters.insert(value.to_string(), ClusterState {
                                    name: value.to_string(),
                                    nodes: HashMap::new(),
                                    last_updated: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                                });
                            }
                            GetRecordOk::FinishedWithNoAdditionalRecord { cache_candidates: _ } => {
                                // No record found, nothing to do
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn update_node_status(&self, cluster_name: &str, peer_id: PeerId, status: NodeStatus) -> Result<()> {
        let mut clusters = self.clusters.write().await;
        if let Some(state) = clusters.get_mut(cluster_name) {
            if let Some(node) = state.nodes.get_mut(&peer_id) {
                node.status = status;
                node.last_seen = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            }
        }
        Ok(())
    }

    async fn run_network_loop(mut self: Arc<Self>) -> Result<(), Box<dyn std::error::Error>> {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Periodically check node status
                    let clusters = self.clusters.read().await;
                    for (name, cluster) in clusters.iter() {
                        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
                        for (peer_id, node) in cluster.nodes.iter() {
                            if now - node.last_seen > NODE_TIMEOUT_SECS {
                                let mut clusters = self.clusters.write().await;
                                if let Some(cluster) = clusters.get_mut(name) {
                                    cluster.nodes.remove(peer_id);
                                }
                            }
                        }
                    }
                }
                event = Arc::get_mut(&mut self).unwrap().swarm.select_next_some() => {
                    match event {
                        SwarmEvent::Behaviour(event) => {
                            self.handle_swarm_event(event).await?;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

unsafe impl Send for P2PNode {}
unsafe impl Sync for P2PNode {}

fn ui(frame: &mut Frame, app_state: &AppState, creating_cluster: &bool, cluster_name_input: &str, sharing_cluster: &bool) {
    if *creating_cluster {
        draw_create_cluster_ui(frame, cluster_name_input);
    } else if *sharing_cluster {
        draw_share_cluster_ui(frame, app_state);
    } else {
        draw_ui(frame, app_state);
    }
}