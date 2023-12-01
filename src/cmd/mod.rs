use std::collections::HashMap;

use clap::{Args, Parser, Subcommand};
use regex::Regex;
use serde::Serialize;

pub mod admin;
pub mod capexec;
pub mod ingest;
pub mod notebooks;
pub mod shell;

const DEFAULT_STATEDB_FS_PATH: &str = "resource-surveillance.sqlite.db";
const DEFAULT_MERGED_STATEDB_FS_PATH: &str = "resource-surveillance-aggregated.sqlite.db";

const DEFAULT_INGEST_FS_IGNORE_PATHS: &str = r"/(\.git|node_modules)/";
const DEFAULT_CAPTURE_EXEC_REGEX_PATTERN: &str = r"surveilr\[(?P<nature>[^\]]*)\]";
const DEFAULT_CAPTURE_SQL_EXEC_REGEX_PATTERN: &str = r"surveilr-SQL";

// this file is similar to .gitignore and, if it appears in a directory or
// parent, it allows `surveilr` to ignore globs specified within it
const DEFAULT_IGNORE_GLOBS_CONF_FILE: &str = ".surveilr_ignore";

// Function to parse a key-value pair in the form of `key=value`.
fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid key-value pair: {}", s));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[derive(Debug, Serialize, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// How to identify this device
    #[arg(long, num_args = 0..=1, default_value = super::DEVICE.name(), default_missing_value = "always", env="SURVEILR_DEVICE_NAME")]
    pub device_name: Option<String>,

    /// Turn debugging information on (repeat for higher levels)
    #[arg(short, long, action = clap::ArgAction::Count, env="SURVEILR_DEBUG")]
    pub debug: u8,

    #[command(subcommand)]
    pub command: CliCommands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize, Subcommand)]
pub enum CliCommands {
    Admin(AdminArgs),
    CapturableExec(CapturableExecArgs),
    Ingest(IngestArgs),
    Notebooks(NotebooksArgs),
    Shell(ShellArgs),
}

/// Admin / maintenance utilities
#[derive(Debug, Serialize, Args)]
pub struct AdminArgs {
    #[command(subcommand)]
    pub command: AdminCommands,
}

#[derive(Debug, Serialize, Subcommand)]
pub enum AdminCommands {
    /// initialize an empty database with bootstrap.sql
    Init {
        /// target SQLite database
        #[arg(short='d', long, default_value = DEFAULT_STATEDB_FS_PATH, default_missing_value = "always", env="SURVEILR_STATEDB_FS_PATH")]
        state_db_fs_path: String,

        /// one or more globs to match as SQL files and batch execute them in alpha order
        #[arg(short = 'I', long)]
        state_db_init_sql: Vec<String>,

        /// remove the existing database first
        #[arg(short, long)]
        remove_existing_first: bool,

        /// add the current device in the empty database's device table
        #[arg(long)]
        with_device: bool,
    },

    /// merge multiple surveillance state databases into a single one
    Merge {
        /// one or more DB name globs to match and merge
        #[arg(short, long, default_value = "*.db")]
        candidates: Vec<String>,

        /// one or more DB name globs to ignore if they match
        #[arg(short = 'i', long)]
        ignore_candidates: Vec<String>,

        /// target SQLite database with merged content
        #[arg(short='d', long, default_value = DEFAULT_MERGED_STATEDB_FS_PATH, default_missing_value = "always", env="SURVEILR_MERGED_STATEDB_FS_PATH")]
        state_db_fs_path: String,

        /// one or more globs to match as SQL files and batch execute them in alpha order
        #[arg(short = 'I', long)]
        state_db_init_sql: Vec<String>,

        /// remove the existing database first
        #[arg(short, long)]
        remove_existing_first: bool,

        /// only generate SQL and emit to STDOUT (no actual merge)
        #[arg(long)]
        sql_only: bool,
    },

    /// generate CLI help markdown
    CliHelpMd,
}

/// Capturable Executables (CE) maintenance tools
#[derive(Debug, Serialize, Args)]
pub struct CapturableExecArgs {
    #[command(subcommand)]
    pub command: CapturableExecCommands,
}

#[derive(Debug, Serialize, Subcommand)]
pub enum CapturableExecCommands {
    /// list potential capturable executables
    Ls {
        /// one or more root paths to ingest
        #[arg(short, long, default_value = ".", default_missing_value = "always")]
        root_fs_path: Vec<String>,

        /// reg-exes to use to ignore files in root-path(s)
        #[serde(with = "serde_regex")]
        #[arg(short, long, default_value = DEFAULT_INGEST_FS_IGNORE_PATHS, default_missing_value = "always")]
        ignore_fs_entry: Vec<Regex>,

        /// reg-exes to use to execute and capture STDOUT, STDERR (e.g. *.surveilr[json].sh) with "nature" capture group
        #[serde(with = "serde_regex")]
        #[arg(long,
            // if you want capturable executables stored in uniform_resource, be sure it's also in surveil_content
            default_value = DEFAULT_CAPTURE_EXEC_REGEX_PATTERN,
            default_missing_value = "always")]
        capture_fs_exec: Vec<regex::Regex>,

        /// reg-exes that will signify which captured executables' output should be treated as batch SQL
        #[serde(with = "serde_regex")]
        #[arg(long,
            // if you want capturable executables stored in uniform_resource, be sure it's also in surveil_content
            default_value = DEFAULT_CAPTURE_SQL_EXEC_REGEX_PATTERN,
            default_missing_value = "always")]
        captured_fs_exec_sql: Vec<regex::Regex>,

        /// emit the results as markdown, not a simple table
        #[arg(long)]
        markdown: bool,
    },

    /// test capturable executables
    Test {
        #[arg(short, long)]
        fs_path: String,

        /// reg-exes to use to execute and capture STDOUT, STDERR (e.g. *.surveilr[json].sh) with "nature" capture group
        #[serde(with = "serde_regex")]
        #[arg(long,
            // if you want capturable executables stored in uniform_resource, be sure it's also in surveil_content
            default_value = DEFAULT_CAPTURE_EXEC_REGEX_PATTERN,
            default_missing_value = "always")]
        capture_fs_exec: Vec<Regex>,

        /// reg-exes that will signify which captured executables' output should be treated as batch SQL
        #[serde(with = "serde_regex")]
        #[arg(long,
            // if you want capturable executables stored in uniform_resource, be sure it's also in surveil_content
            default_value = DEFAULT_CAPTURE_SQL_EXEC_REGEX_PATTERN,
            default_missing_value = "always")]
        captured_fs_exec_sql: Vec<Regex>,
    },
}

/// Ingest content from device file system and other sources
#[derive(Debug, Serialize, Args)]
pub struct IngestArgs {
    #[command(subcommand)]
    pub command: IngestCommands,
}

/// Ingest content from device file system and other sources
#[derive(Debug, Serialize, Args)]
pub struct IngestFilesArgs {
    /// don't run the ingestion, just report statistics
    #[arg(long)]
    pub dry_run: bool,

    /// the behavior name in `behavior` table
    #[arg(short, long, env = "SURVEILR_INGEST_BEHAVIOR_NAME")]
    pub behavior: Option<String>,

    /// one or more root paths to ingest
    #[arg(short, long, default_value = ".", default_missing_value = "always")]
    pub root_fs_path: Vec<String>,

    /// reg-exes to use to ignore files in root-path(s)
    #[serde(with = "serde_regex")]
    #[arg(
        short,
        long,
        default_value = DEFAULT_INGEST_FS_IGNORE_PATHS,
        default_missing_value = "always"
    )]
    pub ignore_fs_entry: Vec<Regex>,

    /// similar to .gitignore, ignore globs specified within it (works only with SmartIgnore walkers)
    #[arg(
        long,
        default_value = DEFAULT_IGNORE_GLOBS_CONF_FILE,
        default_missing_value = "always"
    )]
    pub ignore_globs_conf_file: String,

    /// surveil hidden files (they are ignored by default)
    #[arg(short, long)]
    pub surveil_hidden_files: bool,

    /// reg-exes to use to load content for entry instead of just walking
    #[serde(with = "serde_regex")]
    #[arg(
        long,
        default_values_t = [
            Regex::new(r"\.(md|mdx|html|json|jsonc|tap|txt|text|toml|yaml)$").unwrap(),
            // if you don't want capturable executables stored in uniform_resource, remove the following
            Regex::new(DEFAULT_CAPTURE_EXEC_REGEX_PATTERN).unwrap()],
        default_missing_value = "always"
    )]
    pub surveil_fs_content: Vec<Regex>,

    /// reg-exes to use to execute and capture STDOUT, STDERR (e.g. *.surveilr[json].sh) with "nature" capture group
    #[serde(with = "serde_regex")]
    #[arg(
        long,
        // if you want capturable executables stored in uniform_resource, be sure it's also in surveil_content
        default_value = DEFAULT_CAPTURE_EXEC_REGEX_PATTERN,
        default_missing_value = "always"
    )]
    pub capture_fs_exec: Vec<Regex>,

    /// reg-exes that will signify which captured executables' output should be treated as batch SQL
    #[serde(with = "serde_regex")]
    #[arg(
        long,
        // if you want capturable executables stored in uniform_resource, be sure it's also in surveil_content
        default_value = DEFAULT_CAPTURE_SQL_EXEC_REGEX_PATTERN,
        default_missing_value = "always"
    )]
    pub captured_fs_exec_sql: Vec<regex::Regex>,

    /// bind an unknown nature (file extension), the key, to a known nature the value
    /// "text=text/plain,yaml=application/yaml"
    #[arg(short = 'N', long, value_parser=parse_key_val)]
    pub nature_bind: Option<HashMap<String, String>>,

    /// target SQLite database
    #[arg(short='d', long, default_value = DEFAULT_STATEDB_FS_PATH, default_missing_value = "always", env="SURVEILR_STATEDB_FS_PATH")]
    pub state_db_fs_path: String,

    /// one or more globs to match as SQL files and batch execute them in alpha order
    #[arg(short = 'I', long)]
    pub state_db_init_sql: Vec<String>,

    /// include the surveil database in the ingestion candidates
    #[arg(long)]
    pub include_state_db_in_ingestion: bool,

    /// show stats as an ASCII table after completion
    #[arg(long)]
    pub stats: bool,

    /// show stats in JSON after completion
    #[arg(long)]
    pub stats_json: bool,

    /// save the options as a new behavior
    #[arg(long)]
    pub save_behavior: Option<String>,
}

/// Notebooks maintenance utilities
#[derive(Debug, Serialize, Args)]
pub struct IngestTasksArgs {
    /// target SQLite database
    #[arg(short='d', long, default_value = DEFAULT_STATEDB_FS_PATH, default_missing_value = "always", env="SURVEILR_STATEDB_FS_PATH")]
    pub state_db_fs_path: String,

    /// one or more globs to match as SQL files and batch execute them in alpha order
    #[arg(short = 'I', long)]
    pub state_db_init_sql: Vec<String>,

    /// read tasks from STDIN
    #[arg(long)]
    pub stdin: bool,
}

/// Ingest uniform resources content from multiple sources
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize, Subcommand)]
pub enum IngestCommands {
    Files(IngestFilesArgs),
    Tasks(IngestTasksArgs),
}

/// Notebooks maintenance utilities
#[derive(Debug, Serialize, Args)]
pub struct NotebooksArgs {
    /// target SQLite database
    #[arg(short='d', long, default_value = DEFAULT_STATEDB_FS_PATH, default_missing_value = "always", env="SURVEILR_STATEDB_FS_PATH")]
    pub state_db_fs_path: Option<String>,

    /// one or more globs to match as SQL files and batch execute them in alpha order
    #[arg(short = 'I', long)]
    state_db_init_sql: Vec<String>,

    #[command(subcommand)]
    pub command: NotebooksCommands,
}

#[derive(Debug, Serialize, Subcommand)]
pub enum NotebooksCommands {
    /// Notebooks' cells emit utilities
    Cat {
        /// search for these notebooks (include % for LIKE otherwise =)
        #[arg(short, long)]
        notebook: Vec<String>,

        /// search for these cells (include % for LIKE otherwise =)
        #[arg(short, long)]
        cell: Vec<String>,

        /// add separators before each cell
        #[arg(short, long)]
        seps: bool,
    },

    /// list all notebooks
    Ls {
        /// list all SQL cells that will be handled by execute_migrations
        #[arg(short, long)]
        migratable: bool,
    },
}

/// Deno Task Shell utilities
#[derive(Debug, Serialize, Args)]
pub struct ShellArgs {
    #[command(subcommand)]
    pub command: ShellCommands,
}

#[derive(Debug, Serialize, Subcommand)]
pub enum ShellCommands {
    /// Execute a command string in [Deno Task Shell](https://docs.deno.com/runtime/manual/tools/task_runner) returns JSON
    Json {
        /// the command that would work as a Deno Task
        #[arg(short, long)]
        command: String,

        /// use this as the current working directory (CWD)
        #[arg(long)]
        cwd: Option<String>,

        /// emit stdout only, without the exec status code and stderr
        #[arg(short, long, default_value = "false")]
        stdout_only: bool,
    },
}

impl CliCommands {
    pub fn execute(&self, cli: &Cli) -> anyhow::Result<()> {
        match self {
            CliCommands::Admin(args) => args.command.execute(cli, args),
            CliCommands::CapturableExec(args) => args.command.execute(cli, args),
            CliCommands::Ingest(args) => args.command.execute(cli, args),
            CliCommands::Notebooks(args) => args.command.execute(cli, args),
            CliCommands::Shell(args) => args.command.execute(cli, args),
        }
    }
}
