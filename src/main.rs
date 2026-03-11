mod commands;
mod config;
mod docker;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use config::ConfigManager;
use docker::DockerManager;
use serde::Serialize;

#[derive(Parser)]
#[command(name = "paagan")]
#[command(about = "Manage multiple PostgreSQL versions via Docker", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    format: OutputFormat,

    /// Silence all stderr output
    #[arg(short, long, global = true)]
    quiet: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

pub trait CommandOutput: Serialize {
    fn to_text(&self) -> String;

    fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

#[derive(Subcommand)]
enum Commands {
    /// List all database containers managed by paagan
    List,
    /// Create instance
    Create {
        /// PostgreSQL version
        #[arg(short, long, default_value = "18")]
        version: String,
        /// Name of the instance
        name: String,
    },
    /// Connect to psql
    Psql {
        /// Name of the instance
        name: String,
    },
    /// Show details including connection string
    Show {
        /// Name of the instance
        name: String,
    },
    /// Fork instance
    Fork {
        /// Point-in-time timestamp (optional)
        #[arg(short, long)]
        at: Option<String>,
        /// Source instance name
        old_name: String,
        /// New instance name
        new_name: String,
    },
    /// Delete instance
    Delete {
        /// Name of the instance
        name: String,
        /// Force deletion without confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// Start instance
    Start {
        /// Name of the instance
        name: String,
    },
    /// Stop instance
    Stop {
        /// Name of the instance
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.quiet {
        // Redirect stderr to null if quiet flag is set
        let _ = gag::Redirect::stderr(std::fs::File::open("/dev/null")?)?;
    }

    let config_mgr = ConfigManager::new()?;
    let docker_mgr = DockerManager::new()?;

    let output = match cli.command {
        Commands::List => commands::list::list_instances(&config_mgr, &docker_mgr).await?,
        Commands::Create { version, name } => {
            commands::create::create_instance(&config_mgr, &docker_mgr, version, name).await?
        }
        Commands::Psql { name } => commands::psql::connect_psql(&docker_mgr, name).await?,
        Commands::Show { name } => commands::show::show_instance(&config_mgr, name).await?,
        Commands::Fork {
            at,
            old_name,
            new_name,
        } => {
            commands::fork::fork_instance(&config_mgr, &docker_mgr, at, old_name, new_name).await? 
        },
        
        Commands::Delete { name, force } => {
            commands::delete::delete_instance(&config_mgr, &docker_mgr, name, force).await?
        }
        Commands::Start { name } => {
            commands::start::start_instance(&config_mgr, &docker_mgr, name).await?
        }
        Commands::Stop { name } => {
            commands::stop::stop_instance(&docker_mgr, name).await?
        }
    };

    match cli.format {
        OutputFormat::Text => {
            let text = output.to_text();
            if !text.is_empty() {
                println!("{}", text);
            }
        }
        OutputFormat::Json => println!("{}", output.to_json()),
    }

    Ok(())
}
