use std::sync::{mpsc, OnceLock};

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    backend_name: String,
}

static GPU_CONTEXT: OnceLock<Option<GpuContext>> = OnceLock::new();

/// Retorna o inicializa el contexto GPU global de forma diferida (lazy singleton).
fn get_gpu_context() -> Option<&'static GpuContext> {
    GPU_CONTEXT
        .get_or_init(|| pollster::block_on(init_gpu_context()))
        .as_ref()
}

async fn init_gpu_context() -> Option<GpuContext> {
    let instance = wgpu::Instance::default();

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .ok()?;

    let info = adapter.get_info();
    let backend_name = format!("{:?} - {}", info.backend, info.name);

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Roscel Persistent WGPU Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        })
        .await
        .ok()?;

    Some(GpuContext {
        device,
        queue,
        backend_name,
    })
}

/// Renderiza un fotograma offscreen RGBA de tamaño width x height en out_buffer.
/// Retorna Ok(backend_name) si se completó exitosamente.
pub fn render_frame(
    width: u32,
    height: u32,
    frame_index: u32,
    out_buffer: &mut [u8],
) -> Result<String, String> {
    let expected_len = (width * height * 4) as usize;
    if out_buffer.len() < expected_len {
        return Err(format!(
            "Buffer demasiado pequeño: esperado {}, recibido {}",
            expected_len,
            out_buffer.len()
        ));
    }

    if let Some(ctx) = get_gpu_context() {
        match render_with_context(ctx, width, height, frame_index, out_buffer) {
            Ok(backend) => return Ok(backend),
            Err(err) => {
                eprintln!("[Roscel CBE] Error en renderizado con GPU: {:?}", err);
            }
        }
    }

    // Fallback elegante procedural (SIMD/CPU) si el contexto GPU no está disponible
    render_procedural_fallback(width, height, frame_index, out_buffer);
    Ok("Software Rasterizer Fallback (GPU no disponible)".to_string())
}

fn render_with_context(
    ctx: &GpuContext,
    width: u32,
    height: u32,
    frame_index: u32,
    out_buffer: &mut [u8],
) -> Result<String, String> {
    let device = &ctx.device;
    let queue = &ctx.queue;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Roscel Offscreen Frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Color dinámico según frame_index
    let hue = (frame_index as f64 * 3.0).to_radians();
    let r = 0.5 + 0.4 * (hue).sin();
    let g = 0.5 + 0.4 * (hue + 2.094).sin(); // +120 deg
    let b = 0.5 + 0.4 * (hue + 4.188).sin(); // +240 deg

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Roscel Render Encoder"),
    });

    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Roscel Offscreen Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    // Alinear bytes_per_row a múltiplos de 256
    let unpadded_bytes_per_row = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;
    let staging_buffer_size = (padded_bytes_per_row * height) as u64;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Roscel Staging Readback Buffer"),
        size: staging_buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let buffer_slice = staging_buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |v| {
        let _ = tx.send(v);
    });

    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("Poll error: {:?}", e))?;
    rx.recv()
        .map_err(|_| "Canal de mapeo desconectado".to_string())?
        .map_err(|e| format!("Buffer map async falló: {:?}", e))?;

    let mapped_view = buffer_slice.get_mapped_range();
    for row in 0..height as usize {
        let src_start = row * padded_bytes_per_row as usize;
        let src_end = src_start + unpadded_bytes_per_row as usize;
        let dst_start = row * unpadded_bytes_per_row as usize;
        let dst_end = dst_start + unpadded_bytes_per_row as usize;
        out_buffer[dst_start..dst_end].copy_from_slice(&mapped_view[src_start..src_end]);
    }

    Ok(ctx.backend_name.clone())
}

/// Generador de patrón procedural animado (anillos concéntricos y gradiente cibernético)
fn render_procedural_fallback(width: u32, height: u32, frame_index: u32, out_buffer: &mut [u8]) {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let time = frame_index as f32 * 0.08;

    for y in 0..height {
        let dy = y as f32 - cy;
        for x in 0..width {
            let dx = x as f32 - cx;
            let dist = (dx * dx + dy * dy).sqrt();

            let ring = ((dist * 0.1 - time * 2.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            let angle = dy.atan2(dx);
            let sweep = ((angle * 2.0 + time).sin() * 0.5 + 0.5).clamp(0.0, 1.0);

            let r = ((0.2 + 0.8 * ring) * 255.0) as u8;
            let g = ((0.1 + 0.7 * sweep) * 255.0) as u8;
            let b = ((0.6 + 0.4 * (1.0 - ring)) * 255.0) as u8;
            let a = 255u8;

            let idx = ((y * width + x) * 4) as usize;
            out_buffer[idx] = r;
            out_buffer[idx + 1] = g;
            out_buffer[idx + 2] = b;
            out_buffer[idx + 3] = a;
        }
    }
}
