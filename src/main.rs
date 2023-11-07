use std::path::PathBuf;

use clap::{Parser, Subcommand};
use regex::Regex;
use rusqlite::Connection;

#[macro_use]
extern crate lazy_static;

mod device;
lazy_static! {
    static ref DEVICE: device::Device = device::Device::new(None);
}

#[macro_use]
mod helpers;

mod persist;

mod fsresource;
mod resource;

use fsresource::*;
use persist::*;
use resource::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Generate a Markdown file of all CLI commands and options
    #[arg(long)]
    help_markdown: bool,

    /// How to identify this device
    #[arg(long, num_args = 0..=1, default_value = DEVICE.name(), default_missing_value = "always")]
    device_name: Option<String>,

    /// TODO: Use a Deno *.ts or Nickel config file for defaults, allowing CLI args as overrides
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// TODO: Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Walks the device file system
    FsWalk {
        /// one or more root paths to walk
        #[arg(short, long, default_value = ".", default_missing_value = "always")]
        root_path: Vec<String>,

        /// reg-exes to use to ignore files in root-path(s)
        #[arg(
            short,
            long,
            default_value = "/(\\.git|node_modules)/",
            default_missing_value = "always"
        )]
        ignore_entry: Vec<Regex>,

        /// reg-exes to use to compute digests for
        #[arg(long, default_value = ".*", default_missing_value = "always")]
        compute_digests: Vec<Regex>,

        /// reg-exes to use to load content for entry instead of just walking
        #[arg(
            long,
            default_value = "\\.(md|mdx|html|json)$",
            default_missing_value = "always"
        )]
        surveil_content: Vec<Regex>,

        /// reg-exes to use to load frontmatter for entry in addition to content
        #[arg(
            long,
            default_value = "./device-surveillance.sqlite.db",
            default_missing_value = "always"
        )]
        surveil_db_fs_path: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    if cli.help_markdown {
        clap_markdown::print_help_markdown::<Cli>();
        return;
    }

    // You can check the value provided by positional arguments, or option arguments
    if let Some(name) = cli.device_name.as_deref() {
        println!("Device: {name}");
    }

    if let Some(config_path) = cli.config.as_deref() {
        println!("config: {}", config_path.display());
    }

    // You can see how many times a particular flag or argument occurred
    // Note, only flags can have multiple occurrences
    match cli.debug {
        0 => println!("Debug mode is off"),
        1 => println!("Debug mode is kind of on"),
        2 => println!("Debug mode is on"),
        _ => println!("Don't be crazy"),
    }

    match &cli.command {
        Some(Commands::FsWalk {
            root_path,
            ignore_entry,
            surveil_content,
            surveil_db_fs_path,
            compute_digests,
        }) => {
            if let Some(db_fs_path) = surveil_db_fs_path.as_deref() {
                println!("Surveillance DB URL: {db_fs_path}");

                if let Ok(conn) = Connection::open(db_fs_path) {
                    println!("RusqliteContext preparing {}", db_fs_path);
                    if let Ok(mut ctx) = RusqliteContext::new(&conn) {
                        match select_notebook_cell_code(
                            &conn,
                            "ConstructionSqlNotebook",
                            "initialDDL",
                        ) {
                            Ok((id, _code)) => {
                                println!("select_notebook_cell_code {}", id)
                            }
                            Err(err) => println!("select_notebook_cell_code Error {}", err),
                        }
                        notebook_cells(&conn, |index, id, nb, cell| {
                            println!("{} {} {} {}", index, id, nb, cell);
                            Ok(())
                        })
                        .unwrap();
                        match ctx.execute_batch_stateful(
                            &INIT_DDL_EC,
                            "NONE",
                            "EXECUTED",
                            "initialDDL schema migration",
                        ) {
                            Some(result) => match result {
                                Ok(_) => {
                                    println!("RusqliteContext initDDL executed and state recorded")
                                }
                                Err(err) => println!("RusqliteContext initDDL Error {}", err),
                            },
                            None => println!(
                                "RusqliteContext initDDL already executed, not re-executed"
                            ),
                        };
                        let _ = ctx.upserted_device(&DEVICE);
                        // let _ = ctx.conn.execute_batch("BEGIN TRANSACTION;");
                        // let _ = ctx.conn.execute_batch("COMMIT TRANSACTION;");
                    }
                    println!("RusqliteContext Prepared {}", db_fs_path);
                } else {
                    println!("RusqliteContext Could not open or create: {}", db_fs_path);
                };
            }

            println!("Root paths: {}", root_path.join(", "));
            println!(
                "Ignore entries reg exes: {}",
                ignore_entry
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<&str>>()
                    .join(", ")
            );

            println!(
                "Compute digests reg exes: {}",
                compute_digests
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<&str>>()
                    .join(", ")
            );

            println!(
                "Content surveillance entries reg exes: {}",
                surveil_content
                    .iter()
                    .map(|r| r.as_str())
                    .collect::<Vec<&str>>()
                    .join(", ")
            );

            let walker = FileSysResourcesWalker::new(root_path, ignore_entry, surveil_content);
            match walker {
                Ok(walker) => {
                    let _ = walker.walk_resources(|resource: UniformResource<ContentResource>| {
                        match resource {
                            UniformResource::Html(html) => {
                                println!("HTML: {:?} {:?}", html.resource.uri, html.resource.nature)
                            }
                            UniformResource::Json(json) => {
                                println!("JSON: {:?} {:?}", json.resource.uri, json.resource.nature)
                            }
                            UniformResource::Image(img) => {
                                println!("Image: {:?} {:?}", img.resource.uri, img.resource.nature)
                            }
                            UniformResource::Markdown(md) => {
                                println!("Markdown: {:?} {:?}", md.resource.uri, md.resource.nature)
                            }
                            UniformResource::Unknown(unknown) => {
                                println!("Unknown: {:?} {:?}", unknown.uri, unknown.nature)
                            }
                        }
                    });
                }
                Err(_) => {
                    print!("Error preparing walker")
                }
            }
        }
        None => {}
    }
}
