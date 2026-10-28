mod cli;
mod devices;
mod pubspec;
mod vm_service;
mod watcher;

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use clap::Parser;
use cli::{Cli, Commands};
use devices::DeviceManager;
use pubspec::PubspecScanner;
use vm_service::DartVmClient;
use watcher::FileWatcher;

const BANNER: &str = "\x1b[1;36m
  🦀 Roscel Orquestador \x1b[0m\x1b[90mv0.1.0\x1b[0m
  \x1b[38;5;141m⚡ Toolchain Ligera de Flutter en Rust (Dart VM Service + WGPU Core)\x1b[0m
";

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Devices => handle_devices()?,
        Commands::Attach { device, port, url } => handle_attach(device, port, url).await?,
        Commands::Reload { device, url } => handle_single_reload(device, url).await?,
        Commands::Restart { device, url } => handle_single_restart(device, url).await?,
        Commands::Run { device, extra_args } => handle_run(device, extra_args).await?,
        Commands::Build { extra_args } => handle_build(extra_args)?,
        Commands::Sync { target } => handle_sync(target)?,
        Commands::Doctor => handle_doctor()?,
    }

    Ok(())
}

fn handle_devices() -> Result<()> {
    println!("{}", BANNER);
    let dm = DeviceManager::new()?;
    let list = dm.list_devices()?;

    println!("\x1b[1m📱 Dispositivos Detectados:\x1b[0m");
    if list.is_empty() {
        println!("  \x1b[33mNo se detectaron dispositivos ni emuladores activos.\x1b[0m");
        println!("  Inicia un emulador con: \x1b[32memulator -avd <NOMBRE>\x1b[0m");
        return Ok(());
    }

    for dev in list {
        let icon = if dev.is_emulator { "🤖 [Emulador]" } else { "📱 [Físico]" };
        println!("  • \x1b[1;32m{}\x1b[0m {} - {} (\x1b[34m{}\x1b[0m)", dev.id, icon, dev.model, dev.status);
    }
    println!();
    Ok(())
}

async fn detect_dds_redirect(host: &str, port: u16, auth_code: &str) -> Option<String> {
    let mut stream = tokio::net::TcpStream::connect(format!("{}:{}", host, port)).await.ok()?;
    let path = if auth_code.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", auth_code)
    };
    let req = format!("GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n", path, host, port);
    stream.write_all(req.as_bytes()).await.ok()?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.ok()?;
    let resp = String::from_utf8_lossy(&buf[..n]);

    for line in resp.lines() {
        if line.to_lowercase().starts_with("location:") {
            let loc = line.split_once(':')?.1.trim();
            if let Some(uri_idx) = loc.find("?uri=") {
                let encoded_uri = &loc[uri_idx + 5..];
                let decoded = encoded_uri
                    .replace("%3A", ":")
                    .replace("%2F", "/")
                    .replace("%3D", "=")
                    .replace("%3a", ":")
                    .replace("%2f", "/")
                    .replace("%3d", "=");
                return Some(decoded);
            }
            if let Some(pos) = loc.find("http://") {
                let rest = &loc[pos + 7..];
                if let Some(slash) = rest.find('/') {
                    let addr = &rest[..slash];
                    let sub = &rest[slash + 1..];
                    let sub_clean = sub.split('?').next().unwrap_or("").trim_end_matches('/');
                    return Some(format!("ws://{}/{}/ws", addr, sub_clean));
                }
            }
        }
    }
    None
}

async fn resolve_ws_url(device: Option<String>, port: Option<u16>, url: Option<String>) -> Result<String> {
    if let Some(u) = url {
        return Ok(u);
    }

    let dm = DeviceManager::new()?;
    let devices = dm.list_devices()?;
    if devices.is_empty() {
        return Err(anyhow!("No hay ningún dispositivo Android conectado."));
    }

    let target_device = if let Some(d) = device {
        d
    } else {
        devices[0].id.clone()
    };

    println!("\x1b[34m[Roscel]\x1b[0m Buscando Dart VM Service en dispositivo \x1b[1m{}\x1b[0m...", target_device);
    let (remote_port, auth_code) = dm.discover_vm_service_url(&target_device)?;

    let local_port = port.unwrap_or(remote_port);
    dm.forward_port(&target_device, local_port, remote_port)?;

    // Detecta si DDS está activo y redirigiendo las conexiones
    if let Some(dds_url) = detect_dds_redirect("127.0.0.1", local_port, &auth_code).await {
        println!("\x1b[35m[Roscel]\x1b[0m Servicio DDS detectado. Conectando a través de: \x1b[36m{}\x1b[0m", dds_url);
        return Ok(dds_url);
    }

    let ws_url = if auth_code.is_empty() {
        format!("ws://127.0.0.1:{}/ws", local_port)
    } else {
        format!("ws://127.0.0.1:{}/{}/ws", local_port, auth_code)
    };

    Ok(ws_url)
}

async fn handle_attach(device: Option<String>, port: Option<u16>, url: Option<String>) -> Result<()> {
    println!("{}", BANNER);
    let ws_url = resolve_ws_url(device, port, url).await?;

    println!("\x1b[1;32m[Roscel]\x1b[0m Conectando al WebSocket de Dart VM en: \x1b[36m{}\x1b[0m", ws_url);
    let client = Arc::new(DartVmClient::connect(&ws_url).await?);

    let ver = client.get_version().await.unwrap_or_else(|_| "desconocida".to_string());
    println!("\x1b[1;32m[Roscel]\x1b[0m ✅ Conectado a Dart VM Service (Protocolo v{})", ver);

    let isolates = client.get_isolates().await?;
    for (id, name) in &isolates {
        println!("  ⚡ Isolate activo: \x1b[1m{}\x1b[0m ({})", name, id);
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let lib_dir = cwd.join("lib");

    let _watcher = if lib_dir.exists() {
        println!("\x1b[1;35m[Roscel Watcher]\x1b[0m Monitoreando cambios en \x1b[1m{:?}\x1b[0m...", lib_dir);
        let client_clone = Arc::clone(&client);
        Some(FileWatcher::watch_directory(lib_dir, 150, move |path| {
            let client = Arc::clone(&client_clone);
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("archivo").to_string();
            tokio::spawn(async move {
                print!("\r\x1b[K\x1b[1;33m[Roscel Hot Reload]\x1b[0m Modificado '{}'. Recargando... ", filename);
                io::stdout().flush().ok();
                match client.hot_reload().await {
                    Ok(ms) => {
                        println!("\x1b[1;32mHecho en {} ms! ⚡\x1b[0m", ms);
                    }
                    Err(e) => {
                        println!("\x1b[1;31mFallo: {}\x1b[0m", e);
                    }
                }
                print!("\x1b[90m[r: Hot Reload | R: Hot Restart | q: Salir] > \x1b[0m");
                io::stdout().flush().ok();
            });
        })?)
    } else {
        println!("\x1b[33m[Roscel Watcher]\x1b[0m No se encontró carpeta 'lib/' en el directorio actual. Modo manual activo.");
        None
    };

    println!("\n\x1b[1;32mRoscel en vivo.\x1b[0m Presiona \x1b[1m'r'\x1b[0m para Hot Reload, \x1b[1m'R'\x1b[0m para Hot Restart, o \x1b[1m'q'\x1b[0m para salir.");
    print!("\x1b[90m[r: Hot Reload | R: Hot Restart | q: Salir] > \x1b[0m");
    io::stdout().flush().ok();

    // Loop de comandos interactivos por consola
    let (tx_line, mut rx_line) = tokio::sync::mpsc::channel::<String>(10);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut line = String::new();
        while stdin.read_line(&mut line).is_ok() {
            if tx_line.blocking_send(line.trim().to_string()).is_err() {
                break;
            }
            line.clear();
        }
    });

    while let Some(cmd) = rx_line.recv().await {
        match cmd.as_str() {
            "r" => {
                print!("\x1b[1;33m[Roscel]\x1b[0m Disparando Hot Reload manual... ");
                io::stdout().flush().ok();
                match client.hot_reload().await {
                    Ok(ms) => println!("\x1b[1;32mCompletado en {} ms! ⚡\x1b[0m", ms),
                    Err(e) => println!("\x1b[1;31mError: {}\x1b[0m", e),
                }
            }
            "R" => {
                print!("\x1b[1;33m[Roscel]\x1b[0m Disparando Hot Restart manual... ");
                io::stdout().flush().ok();
                match client.hot_restart().await {
                    Ok(ms) => println!("\x1b[1;32mReiniciado en {} ms! 🔄\x1b[0m", ms),
                    Err(e) => println!("\x1b[1;31mError: {}\x1b[0m", e),
                }
            }
            "q" | "exit" => {
                println!("\x1b[1;34m[Roscel]\x1b[0m Desconectando de Dart VM.");
                break;
            }
            _ => {}
        }
        print!("\x1b[90m[r: Hot Reload | R: Hot Restart | q: Salir] > \x1b[0m");
        io::stdout().flush().ok();
    }

    Ok(())
}

async fn handle_single_reload(device: Option<String>, url: Option<String>) -> Result<()> {
    let ws_url = resolve_ws_url(device, None, url).await?;
    let client = DartVmClient::connect(&ws_url).await?;
    let ms = client.hot_reload().await?;
    println!("\x1b[1;32m[Roscel]\x1b[0m ⚡ Hot Reload exitoso en {} ms.", ms);
    Ok(())
}

async fn handle_single_restart(device: Option<String>, url: Option<String>) -> Result<()> {
    let ws_url = resolve_ws_url(device, None, url).await?;
    let client = DartVmClient::connect(&ws_url).await?;
    let ms = client.hot_restart().await?;
    println!("\x1b[1;32m[Roscel]\x1b[0m 🔄 Hot Restart exitoso en {} ms.", ms);
    Ok(())
}

fn find_flutter_bin() -> PathBuf {
    let default_path = PathBuf::from("/home/yader/flutter/bin/flutter");
    if default_path.exists() {
        return default_path;
    }
    PathBuf::from("flutter")
}

fn find_roscel_root() -> PathBuf {
    PathBuf::from("/home/yader/Roscel-CBE")
}

fn ensure_native_synced(target_dir: &Path) {
    let jni_dir = target_dir.join("android/app/src/main/jniLibs");
    let x86_so = jni_dir.join("x86_64/libroscel_cbe.so");
    let arm_so = jni_dir.join("arm64-v8a/libroscel_cbe.so");

    if !x86_so.exists() || !arm_so.exists() {
        println!("\x1b[1;33m[Roscel CBE]\x1b[0m Binarios nativos no encontrados en jniLibs. Sincronizando...");
        let roscel_root = find_roscel_root();
        let script = roscel_root.join("build_and_sync_android.sh");

        if script.exists() {
            let status = Command::new("bash")
                .arg(&script)
                .arg(target_dir)
                .current_dir(&roscel_root)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("\x1b[1;32m[Roscel CBE]\x1b[0m Motor nativo compilado y sincronizado correctamente.");
                }
                _ => {
                    eprintln!("\x1b[1;31m[Roscel CBE]\x1b[0m Advertencia al compilar binarios nativos.");
                }
            }
        }
    } else {
        println!("\x1b[1;32m[Roscel CBE]\x1b[0m Motor nativo verificado en jniLibs (x86_64 y arm64-v8a).");
    }
}

async fn handle_run(device: Option<String>, extra_args: Vec<String>) -> Result<()> {
    println!("{}", BANNER);
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Análisis instantáneo de pubspec con scanner de Rust
    let pubspec = PubspecScanner::scan(&cwd)?;
    println!("\x1b[1;34m[Roscel Scanner]\x1b[0m Proyecto: \x1b[1m{}\x1b[0m v{} (Flutter: {})", pubspec.name, pubspec.version, pubspec.is_flutter);

    ensure_native_synced(&cwd);

    let dm = DeviceManager::new()?;
    let devices = dm.list_devices()?;
    let target_dev = device.unwrap_or_else(|| {
        devices.first().map(|d| d.id.clone()).unwrap_or_else(|| "emulator-5554".to_string())
    });

    println!("\x1b[1;35m[Roscel]\x1b[0m Lanzando app en dispositivo: \x1b[1m{}\x1b[0m...", target_dev);

    let flutter_bin = find_flutter_bin();
    let mut cmd = Command::new(flutter_bin);
    cmd.arg("run");
    cmd.arg("-d");
    cmd.arg(&target_dev);
    cmd.args(&extra_args);
    cmd.env("ANDROID_HOME", "/home/yader/Android/Sdk");
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let status = cmd.status().map_err(|e| anyhow!("Error al ejecutar flutter: {}", e))?;
    std::process::exit(status.code().unwrap_or(0));
}

fn handle_build(extra_args: Vec<String>) -> Result<()> {
    println!("{}", BANNER);
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let pubspec = PubspecScanner::scan(&cwd)?;
    println!("\x1b[1;34m[Roscel]\x1b[0m Compilando proyecto \x1b[1m{}\x1b[0m...", pubspec.name);

    ensure_native_synced(&cwd);

    let flutter_bin = find_flutter_bin();
    let mut cmd = Command::new(flutter_bin);
    cmd.arg("build");
    cmd.args(&extra_args);
    cmd.env("ANDROID_HOME", "/home/yader/Android/Sdk");
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let status = cmd.status().map_err(|e| anyhow!("Error al ejecutar flutter build: {}", e))?;
    std::process::exit(status.code().unwrap_or(0));
}

fn handle_sync(target: Option<String>) -> Result<()> {
    println!("{}", BANNER);
    let target_path = target
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from("/home/yader/roscel_sandbox")));

    println!("\x1b[1;34m[Roscel CBE]\x1b[0m Sincronizando motor nativo en {:?}...", target_path);
    let roscel_root = find_roscel_root();
    let script = roscel_root.join("build_and_sync_android.sh");

    let status = Command::new("bash")
        .arg(&script)
        .arg(&target_path)
        .current_dir(&roscel_root)
        .status()?;

    if status.success() {
        println!("\x1b[1;32m[Roscel CBE]\x1b[0m Sincronización completada con éxito.");
    } else {
        return Err(anyhow!("Error durante la sincronización"));
    }
    Ok(())
}

fn handle_doctor() -> Result<()> {
    println!("{}", BANNER);
    println!("\x1b[1;34m=== Diagnóstico del Entorno Roscel CBE ===\x1b[0m\n");

    // 1. Rust
    print!("🦀 Rust Compiler: ");
    if let Ok(out) = Command::new("rustc").arg("--version").output() {
        println!("\x1b[32m{}\x1b[0m", String::from_utf8_lossy(&out.stdout).trim());
    } else {
        println!("\x1b[31mNo encontrado\x1b[0m");
    }

    // 2. Android SDK
    print!("🤖 Android SDK: ");
    let sdk_path = Path::new("/home/yader/Android/Sdk");
    if sdk_path.exists() {
        println!("\x1b[32mDisponible en {:?}\x1b[0m", sdk_path);
    } else {
        println!("\x1b[31mNo encontrado\x1b[0m");
    }

    // 3. Android NDK
    print!("📦 Android NDK: ");
    let ndk_path = Path::new("/home/yader/Android/Sdk/ndk/28.2.13676358");
    if ndk_path.exists() {
        println!("\x1b[32mDisponible en {:?}\x1b[0m", ndk_path);
    } else {
        println!("\x1b[31mNo encontrado\x1b[0m");
    }

    // 4. Flutter SDK
    print!("🦋 Flutter SDK: ");
    let flutter_bin = find_flutter_bin();
    if let Ok(out) = Command::new(&flutter_bin).arg("--version").output() {
        let first_line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string();
        println!("\x1b[32m{}\x1b[0m", first_line);
    } else {
        println!("\x1b[31mNo encontrado\x1b[0m");
    }

    // 5. Dispositivos ADB
    println!("\n📱 Dispositivos Conectados:");
    if let Ok(dm) = DeviceManager::new() {
        if let Ok(devs) = dm.list_devices() {
            if devs.is_empty() {
                println!("  \x1b[33m(Ninguno)\x1b[0m");
            } else {
                for d in devs {
                    let kind = if d.is_emulator { "Emulador" } else { "Físico" };
                    println!("  • \x1b[32m{}\x1b[0m [{}] - {}", d.id, kind, d.model);
                }
            }
        }
    }

    println!("\n\x1b[1;32mTodo listo para orquestar con Roscel.\x1b[0m");
    Ok(())
}
