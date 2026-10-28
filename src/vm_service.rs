use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub struct DartVmClient {
    write_stream: Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    read_stream: Arc<Mutex<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
    request_id: AtomicU64,
}

impl DartVmClient {
    /// Conecta con el Dart VM Service Protocol a través de WebSocket
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws_stream, _) = connect_async(url)
            .await
            .with_context(|| format!("Error conectando con el WebSocket de Dart VM en {}", url))?;

        let (write_stream, read_stream) = ws_stream.split();

        Ok(Self {
            write_stream: Arc::new(Mutex::new(write_stream)),
            read_stream: Arc::new(Mutex::new(read_stream)),
            request_id: AtomicU64::new(1),
        })
    }

    /// Envía una solicitud JSON-RPC 2.0 y espera la respuesta correspondiente
    pub async fn call_method(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id.to_string(),
        });

        let payload = serde_json::to_string(&request)?;
        {
            let mut write = self.write_stream.lock().await;
            write.send(Message::Text(payload.into())).await
                .context("Error enviando mensaje por WebSocket")?;
        }

        // Lee respuestas hasta encontrar el ID coincidente
        let mut read = self.read_stream.lock().await;
        while let Some(msg) = read.next().await {
            let msg = msg.context("Error recibiendo mensaje de Dart VM")?;
            if let Message::Text(text) = msg {
                if let Ok(val) = serde_json::from_str::<Value>(&text) {
                    if val.get("id").and_then(|i| i.as_str()) == Some(&id.to_string()) {
                        if let Some(error) = val.get("error") {
                            return Err(anyhow!("Dart VM RPC Error: {}", error));
                        }
                        return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                    }
                }
            }
        }

        Err(anyhow!("Conexión cerrada antes de recibir respuesta RPC de Dart VM"))
    }

    /// Obtiene la versión del protocolo de Dart VM Service
    pub async fn get_version(&self) -> Result<String> {
        let res = self.call_method("getVersion", json!({})).await?;
        let major = res.get("major").and_then(|v| v.as_i64()).unwrap_or(0);
        let minor = res.get("minor").and_then(|v| v.as_i64()).unwrap_or(0);
        Ok(format!("{}.{}", major, minor))
    }

    /// Obtiene la lista de Isolates activos (ID y nombre)
    pub async fn get_isolates(&self) -> Result<Vec<(String, String)>> {
        let res = self.call_method("getVM", json!({})).await?;
        let mut isolates = Vec::new();
        if let Some(list) = res.get("isolates").and_then(|v| v.as_array()) {
            for item in list {
                let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed").to_string();
                if !id.is_empty() {
                    isolates.push((id, name));
                }
            }
        }
        Ok(isolates)
    }

    /// Recarga el código fuente en un Isolate específico (`reloadSources`)
    pub async fn reload_sources(&self, isolate_id: &str) -> Result<Value> {
        self.call_method("reloadSources", json!({
            "isolateId": isolate_id,
            "force": false
        })).await
    }

    /// Ejecuta el Flutter Service Extension `ext.flutter.reassemble` para reconstruir los widgets
    pub async fn reassemble(&self, isolate_id: &str) -> Result<Value> {
        self.call_method("ext.flutter.reassemble", json!({
            "isolateId": isolate_id,
        })).await
    }

    /// Realiza un Hot Reload completo y quirúrgico
    pub async fn hot_reload(&self) -> Result<u128> {
        let start = Instant::now();
        let isolates = self.get_isolates().await?;
        if isolates.is_empty() {
            return Err(anyhow!("No se encontraron isolates activos en la Dart VM"));
        }

        for (id, _name) in &isolates {
            // 1. Recargar código fuente en la VM
            let _ = self.reload_sources(id).await;
            // 2. Reensamblar el árbol de widgets de Flutter
            let _ = self.reassemble(id).await;
        }

        let elapsed = start.elapsed().as_millis();
        Ok(elapsed)
    }

    /// Realiza un Hot Restart completo reiniciando los isolates
    pub async fn hot_restart(&self) -> Result<u128> {
        let start = Instant::now();
        let isolates = self.get_isolates().await?;
        for (id, _name) in &isolates {
            let _ = self.call_method("hotRestart", json!({ "isolateId": id })).await;
        }
        let elapsed = start.elapsed().as_millis();
        Ok(elapsed)
    }
}
