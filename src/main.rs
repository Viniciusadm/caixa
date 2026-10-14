mod auth;
mod config;
mod drive;
mod links;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;

use config::Config;
use drive::{Drive, DriveFile, human_size};

const FOLDER_NAME: &str = "caixa";

#[derive(Parser)]
#[command(
    name = "cx",
    version,
    about = "Envia arquivos pro Google Drive e devolve a URL de download"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(visible_alias = "u", about = "Envia arquivos e imprime as URLs de download")]
    Up {
        #[arg(required = true, num_args = 1..)]
        files: Vec<PathBuf>,
        #[arg(short, long, help = "Não compartilha com \"qualquer pessoa com o link\"")]
        private: bool,
    },
    #[command(visible_alias = "l", about = "Lista os arquivos enviados pelo cx")]
    Ls {
        #[arg(long, help = "Saída em JSON")]
        json: bool,
    },
    #[command(about = "Imprime a URL de download de um arquivo")]
    Url { file: String },
    #[command(visible_alias = "g", about = "Baixa um arquivo pelo id ou link")]
    Get {
        file: String,
        #[arg(
            short,
            long,
            help = "Destino (arquivo ou pasta); padrão: nome original na pasta atual"
        )]
        output: Option<PathBuf>,
    },
    #[command(about = "Move arquivos pra lixeira do Drive")]
    Rm {
        #[arg(required = true, num_args = 1..)]
        files: Vec<String>,
    },
    #[command(about = "Configura o client ID e o secret do OAuth")]
    Config,
    #[command(about = "Faz login na conta Google pelo navegador")]
    Login,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("erro: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let http = Client::builder().timeout(Duration::from_secs(600)).build()?;

    if let Commands::Config = cli.command {
        return config::run_interactive();
    }

    let mut config = Config::load()?;

    if let Commands::Login = cli.command {
        config.refresh_token = Some(auth::login(&http, &config)?);
        config.save()?;
        println!("Login feito.");
        return Ok(());
    }

    let token = auth::access_token(&http, &config)?;
    let drive = Drive::new(http, token);

    match cli.command {
        Commands::Config | Commands::Login => unreachable!(),
        Commands::Up { files, private } => {
            for path in &files {
                if !path.is_file() {
                    bail!("{} não é um arquivo", path.display());
                }
            }
            let folder_id = drive.ensure_folder(config.folder_id.as_deref(), FOLDER_NAME)?;
            if config.folder_id.as_deref() != Some(&folder_id) {
                config.folder_id = Some(folder_id.clone());
                config.save()?;
            }
            for path in &files {
                let file = drive.upload(path, &folder_id)?;
                if !private && let Err(error) = drive.make_public(&file.id) {
                    eprintln!(
                        "aviso: não foi possível tornar {} público ({error:#}); o link só abre pra quem tem acesso",
                        file.name
                    );
                }
                println!("{}", links::download_url(&file.id));
            }
            Ok(())
        }
        Commands::Ls { json } => {
            let files = drive.list()?;
            if json {
                println!("{}", serde_json::to_string(&files)?);
            } else if files.is_empty() {
                println!("Nada enviado ainda. Envie com: cx up arquivo");
            } else {
                let colors = colors_enabled();
                for file in &files {
                    print_file(file, colors);
                }
            }
            Ok(())
        }
        Commands::Url { file } => {
            let file = drive.get(&links::parse_file_ref(&file)?)?;
            println!("{}", links::download_url(&file.id));
            Ok(())
        }
        Commands::Get { file, output } => {
            let file = drive.get(&links::parse_file_ref(&file)?)?;
            let destination = match output {
                Some(path) if path.is_dir() => path.join(&file.name),
                Some(path) => path,
                None => PathBuf::from(&file.name),
            };
            if destination.exists() {
                bail!("{} já existe", destination.display());
            }
            drive.download(&file, &destination)?;
            println!("{}", destination.display());
            Ok(())
        }
        Commands::Rm { files } => {
            for reference in &files {
                let file = drive.get(&links::parse_file_ref(reference)?)?;
                drive.trash(&file.id)?;
                println!("{} movido pra lixeira", file.name);
            }
            Ok(())
        }
    }
}

pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn colors_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn print_file(file: &DriveFile, colors: bool) {
    let (dim, bold, reset) = if colors {
        ("\x1b[2m", "\x1b[1m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let size = file.size.map(human_size).unwrap_or_default();
    let date = file.created_time.get(..10).unwrap_or(&file.created_time);
    println!("{dim}{date}{reset}  {:>9}  {bold}{}{reset}", size, file.name);
    println!("            {dim}{}{reset}", links::download_url(&file.id));
}
