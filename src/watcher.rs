use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// Inicia un watcher sobre un directorio (ej. `lib/`) con debounce para disparar Hot Reload
    pub fn watch_directory<F>(dir: PathBuf, debounce_ms: u64, on_change: F) -> Result<Self>
    where
        F: Fn(PathBuf) + Send + 'static,
    {
        let (tx, rx) = channel::<notify::Result<Event>>();

        let mut watcher = RecommendedWatcher::new(
            tx,
            Config::default().with_poll_interval(Duration::from_millis(100)),
        ).context("Error creando watcher del sistema de archivos")?;

        watcher
            .watch(&dir, RecursiveMode::Recursive)
            .with_context(|| format!("Error vigilando directorio {:?}", dir))?;

        std::thread::spawn(move || {
            let mut last_event_time = Instant::now() - Duration::from_secs(10);
            let debounce_duration = Duration::from_millis(debounce_ms);

            while let Ok(res) = rx.recv() {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            for path in event.paths {
                                if let Some(ext) = path.extension() {
                                    if ext == "dart" {
                                        let now = Instant::now();
                                        if now.duration_since(last_event_time) >= debounce_duration {
                                            last_event_time = now;
                                            on_change(path);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        Ok(Self { _watcher: watcher })
    }
}
