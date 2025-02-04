use std::env;
use std::time::Duration;
use std::path::{Path, PathBuf};
use std::fs;

use color_eyre::{eyre::Context, Result, eyre::eyre};
use ratatui::{
    crossterm::event::{self, Event, KeyCode},
    widgets::{Paragraph, Block, Borders, List, ListItem},
    layout::{Layout, Direction, Constraint},
    style::{Style, Color},
    DefaultTerminal, Frame,
};

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
        },
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

/// Runs the interactive CLI mode using ratatui.
fn run_interactive(config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config)?;
    app_state.load_clusters()?;
    
    let mut terminal = ratatui::init();
    
    loop {
        terminal.draw(|frame| draw_ui(frame, &app_state))?;
        
        if event::poll(Duration::from_millis(250))? {
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
    
    ratatui::restore();
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

/// Dummy implementation for daemon mode
fn run_daemon(config: Option<String>) -> Result<()> {
    let app_state = AppState::new(config)?;
    println!("Daemon mode using config: {}", app_state.config_path.display());
    println!("Running daemon mode... (not fully implemented)");
    Ok(())
}

/// Implementation for cluster commands
fn run_cluster(action: ClusterAction, config: Option<String>) -> Result<()> {
    let mut app_state = AppState::new(config)?;
    
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

/// Dummy implementation for mounting the filesystem
fn run_mount(config: Option<String>) -> Result<()> {
    let app_state = AppState::new(config)?;
    println!("Mounting filesystem using config: {}", app_state.config_path.display());
    println!("Mounting filesystem... (not fully implemented)");
    Ok(())
}

/// Dummy implementation for unmounting the filesystem
fn run_unmount(config: Option<String>) -> Result<()> {
    let app_state = AppState::new(config)?;
    println!("Unmounting filesystem using config: {}", app_state.config_path.display());
    println!("Unmounting filesystem... (not fully implemented)");
    Ok(())
}

/// Checks if the user has pressed 'q' to quit (used in interactive UI mode).
fn should_quit() -> Result<bool> {
    if event::poll(Duration::from_millis(250)).context("event poll failed")? {
        if let Event::Key(key) = event::read().context("event read failed")? {
            return Ok(KeyCode::Char('q') == key.code);
        }
    }
    Ok(false)
}

/// Main entrypoint
fn main() -> Result<()> {
    color_eyre::install()?;
    let args: Vec<String> = env::args().collect();
    match process_args(&args) {
        CliCommand::Help => {
            print_help();
            Ok(())
        },
        CliCommand::Interactive { config } => run_interactive(config),
        CliCommand::Daemon { config } => run_daemon(config),
        CliCommand::Cluster { action, config } => run_cluster(action, config),
        CliCommand::Mount { config } => run_mount(config),
        CliCommand::Unmount { config } => run_unmount(config),
    }
}

// Detailed test cases for CLI argument parsing
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_args_interactive_no_args() {
        let args = vec!["lis".to_string()];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Interactive { config: None });
    }

    #[test]
    fn test_process_args_with_help_flag() {
        let args = vec!["lis".to_string(), "--help".to_string()];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Help);
    }

    #[test]
    fn test_process_args_with_config() {
        let args = vec![
            "lis".to_string(),
            "--config".to_string(),
            "myconfig.toml".to_string(),
        ];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Interactive { config: Some("myconfig.toml".to_string()) });
    }

    #[test]
    fn test_process_args_cluster_list() {
        let args = vec!["lis".to_string(), "cluster".to_string()];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Cluster { action: ClusterAction::List, config: None });
    }

    #[test]
    fn test_process_args_cluster_join_with_ticket_arg() {
        let args = vec![
            "lis".to_string(),
            "cluster".to_string(),
            "join".to_string(),
            "test_cluster".to_string(),
            "ticket123".to_string(),
        ];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Cluster {
            action: ClusterAction::Join { cluster: "test_cluster".to_string(), ticket: Some("ticket123".to_string()) },
            config: None
        });
    }

    #[test]
    fn test_process_args_cluster_join_without_ticket_arg() {
        // Ensure that if no ticket is provided, we get None (unless the env var is set, but for test we assume it is not)
        env::remove_var("LIS_TICKET");
        let args = vec![
            "lis".to_string(),
            "cluster".to_string(),
            "join".to_string(),
            "test_cluster".to_string(),
        ];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Cluster {
            action: ClusterAction::Join { cluster: "test_cluster".to_string(), ticket: None },
            config: None
        });
    }

    #[test]
    fn test_process_args_daemon() {
        let args = vec!["lis".to_string(), "daemon".to_string()];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Daemon { config: None });
    }

    #[test]
    fn test_process_args_mount() {
        let args = vec!["lis".to_string(), "mount".to_string()];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Mount { config: None });
    }

    #[test]
    fn test_process_args_unmount() {
        let args = vec!["lis".to_string(), "unmount".to_string()];
        let cmd = process_args(&args);
        assert_eq!(cmd, CliCommand::Unmount { config: None });
    }

    #[test]
    fn test_unknown_command() {
        let args = vec!["lis".to_string(), "foobar".to_string()];
        let cmd = process_args(&args);
        // For unknown commands, our parser returns Help.
        assert_eq!(cmd, CliCommand::Help);
    }
}