use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub model: String,
    pub is_emulator: bool,
    pub status: String,
}

pub struct DeviceManager {
    adb_path: PathBuf,
}

#[allow(dead_code)]
impl DeviceManager {
    pub fn new() -> Result<Self> {
        let adb = Self::find_adb()
            .ok_or_else(|| anyhow!("No se encontró el ejecutable de ADB en el sistema ni en ANDROID_HOME"))?;
        Ok(Self { adb_path: adb })
    }

    pub fn find_adb() -> Option<PathBuf> {
        if let Ok(android_home) = std::env::var("ANDROID_HOME") {
            let p = Path::new(&android_home).join("platform-tools/adb");
            if p.exists() {
                return Some(p);
            }
        }
        let default_sdk = Path::new("/home/yader/Android/Sdk/platform-tools/adb");
        if default_sdk.exists() {
            return Some(default_sdk.to_path_buf());
        }
        if let Ok(output) = Command::new("which").arg("adb").output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(PathBuf::from(s));
                }
            }
        }
        None
    }

    /// Lista todos los dispositivos y emuladores conectados detectados por ADB
    pub fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        let output = Command::new(&self.adb_path)
            .arg("devices")
            .arg("-l")
            .output()
            .context("Error al ejecutar adb devices -l")?;

        if !output.status.success() {
            return Err(anyhow!("adb devices devolvió un código de error"));
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let mut devices = Vec::new();

        for line in text.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let id = parts[0].to_string();
            let status = parts[1].to_string();
            let is_emulator = id.starts_with("emulator-");

            let mut model = if is_emulator { "Emulador Android".to_string() } else { "Dispositivo Android".to_string() };
            for part in &parts[2..] {
                if let Some(m) = part.strip_prefix("model:") {
                    model = m.replace('_', " ");
                }
            }

            devices.push(DeviceInfo {
                id,
                model,
                is_emulator,
                status,
            });
        }

        Ok(devices)
    }

    /// Redirige un puerto local al puerto del dispositivo mediante ADB forward
    pub fn forward_port(&self, device_id: &str, local_port: u16, remote_port: u16) -> Result<()> {
        let status = Command::new(&self.adb_path)
            .arg("-s")
            .arg(device_id)
            .arg("forward")
            .arg(format!("tcp:{}", local_port))
            .arg(format!("tcp:{}", remote_port))
            .status()
            .context("Error al ejecutar adb forward")?;

        if !status.success() {
            return Err(anyhow!("Fallo al redirigir puerto tcp:{} -> tcp:{}", local_port, remote_port));
        }
        Ok(())
    }

    /// Inicia una app en el dispositivo
    pub fn launch_app(&self, device_id: &str, package: &str, activity: &str) -> Result<()> {
        let status = Command::new(&self.adb_path)
            .arg("-s")
            .arg(device_id)
            .arg("shell")
            .arg("am")
            .arg("start")
            .arg("-n")
            .arg(format!("{}/{}", package, activity))
            .status()
            .context("Error al ejecutar am start")?;

        if !status.success() {
            return Err(anyhow!("Fallo al iniciar actividad {}/{}", package, activity));
        }
        Ok(())
    }

    /// Detiene la app en el dispositivo
    pub fn stop_app(&self, device_id: &str, package: &str) -> Result<()> {
        let _ = Command::new(&self.adb_path)
            .arg("-s")
            .arg(device_id)
            .arg("shell")
            .arg("am")
            .arg("force-stop")
            .arg(package)
            .status();
        Ok(())
    }

    /// Escanea logcat para descubrir la URL activa del Dart VM Service
    pub fn discover_vm_service_url(&self, device_id: &str) -> Result<(u16, String)> {
        let output = Command::new(&self.adb_path)
            .arg("-s")
            .arg(device_id)
            .arg("logcat")
            .arg("-d")
            .arg("-s")
            .arg("flutter")
            .output()
            .context("Error al consultar logcat de Flutter")?;

        let text = String::from_utf8_lossy(&output.stdout);
        // Formato típico: "The Dart VM service is listening on http://127.0.0.1:44399/JTiHelTJmG0=/"
        for line in text.lines().rev() {
            if line.contains("The Dart VM service is listening on") {
                if let Some(pos) = line.find("http://") {
                    let rest = &line[pos + 7..];
                    if let Some(slash_pos) = rest.find('/') {
                        let host_port = &rest[..slash_pos];
                        let after_slash = &rest[slash_pos + 1..];
                        let auth_code = after_slash.trim_end_matches('/').trim().to_string();

                        let port_str = host_port.split(':').nth(1).unwrap_or("0");
                        if let Ok(port) = port_str.parse::<u16>() {
                            return Ok((port, auth_code));
                        }
                    }
                }
            }
        }

        Err(anyhow!("No se encontró ninguna sesión activa de Dart VM en logcat. ¿La app Flutter está en ejecución?"))
    }
}
