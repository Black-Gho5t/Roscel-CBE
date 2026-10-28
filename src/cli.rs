use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "roscel")]
#[command(version = "0.1.0")]
#[command(about = "🦀 Roscel: Orquestador Ligero de Flutter en Rust y Motor de Vela Studio", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Lista y escanea los dispositivos y emuladores conectados (ADB)
    Devices,

    /// Se conecta vía WebSocket al Dart VM Service y activa el Hot Reload en vivo con watcher
    Attach {
        /// ID del dispositivo (ej. emulator-5554)
        #[arg(short, long)]
        device: Option<String>,

        /// Puerto local expuesto
        #[arg(short, long)]
        port: Option<u16>,

        /// URL completa del WebSocket (ej. ws://127.0.0.1:44399/auth/ws)
        #[arg(short, long)]
        url: Option<String>,
    },

    /// Ejecuta un Hot Reload quirúrgico inmediato
    Reload {
        #[arg(short, long)]
        device: Option<String>,

        #[arg(short, long)]
        url: Option<String>,
    },

    /// Ejecuta un Hot Restart completo inmediato
    Restart {
        #[arg(short, long)]
        device: Option<String>,

        #[arg(short, long)]
        url: Option<String>,
    },

    /// Inicia la app y orquesta el monitoreo con Hot Reload instantáneo
    Run {
        #[arg(short, long)]
        device: Option<String>,

        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    /// Compila la app Flutter asegurando el motor nativo
    Build {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    /// Recompila y sincroniza libroscel_cbe.so en el proyecto actual
    Sync {
        #[arg(short, long)]
        target: Option<String>,
    },

    /// Diagnóstico completo del entorno de desarrollo
    Doctor,
}
