use std::ffi::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use taffy::prelude::*;

static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Retorna la cadena de versión y handshake de Roscel CBE.
#[unsafe(no_mangle)]
pub extern "C" fn roscel_cbe_version() -> *const c_char {
    c"Roscel CBE v0.1.0 (Rust Core)".as_ptr()
}

/// Inicializa el motor Roscel CBE. Retorna 0 en éxito.
#[unsafe(no_mangle)]
pub extern "C" fn roscel_cbe_init() -> i32 {
    IS_INITIALIZED.store(true, Ordering::SeqCst);
    #[cfg(target_os = "android")]
    {
        // En Android podemos redirigir salidas al logcat si fuera necesario
    }
    println!("[Roscel CBE] Motor inicializado correctamente.");
    0
}

/// Función de verificación rápida (Ping-Pong FFI).
#[unsafe(no_mangle)]
pub extern "C" fn roscel_cbe_ping(value: i32) -> i32 {
    value * 42
}

/// Prueba de cálculo de layout de alta velocidad con Taffy.
/// Construye un contenedor con 2 hijos en Flexbox y retorna la suma de los anchos calculados.
#[unsafe(no_mangle)]
pub extern "C" fn roscel_cbe_layout_benchmark(container_width: f32, container_height: f32) -> f32 {
    let mut taffy: TaffyTree<()> = TaffyTree::new();

    let child_style = Style {
        size: Size {
            width: Dimension::percent(0.5),
            height: Dimension::percent(1.0),
        },
        ..Default::default()
    };

    let child1 = taffy.new_leaf(child_style.clone()).unwrap();
    let child2 = taffy.new_leaf(child_style).unwrap();

    let root_style = Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        size: Size {
            width: Dimension::length(container_width),
            height: Dimension::length(container_height),
        },
        ..Default::default()
    };

    let root = taffy.new_with_children(root_style, &[child1, child2]).unwrap();

    taffy.compute_layout(root, Size::MAX_CONTENT).unwrap();

    let layout1 = taffy.layout(child1).unwrap();
    let layout2 = taffy.layout(child2).unwrap();

    layout1.size.width + layout2.size.width
}

/// Finaliza el motor y limpia recursos globales.
#[unsafe(no_mangle)]
pub extern "C" fn roscel_cbe_shutdown() -> i32 {
    IS_INITIALIZED.store(false, Ordering::SeqCst);
    println!("[Roscel CBE] Motor apagado correctamente.");
    0
}

mod offscreen_renderer;

/// Renderiza un fotograma gráfico RGBA (width x height) generado por Roscel CBE.
/// Retorna 0 en éxito y escribe el nombre del backend gráfico activo en status_buf.
#[unsafe(no_mangle)]
pub extern "C" fn roscel_cbe_render_frame(
    width: u32,
    height: u32,
    frame_index: u32,
    pixel_buffer: *mut u8,
    pixel_buffer_len: usize,
    status_buf: *mut c_char,
    status_buf_len: usize,
) -> i32 {
    if pixel_buffer.is_null() {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(pixel_buffer, pixel_buffer_len) };
    match offscreen_renderer::render_frame(width, height, frame_index, slice) {
        Ok(backend) => {
            if !status_buf.is_null() && status_buf_len > 0 {
                let bytes = backend.as_bytes();
                let copy_len = bytes.len().min(status_buf_len - 1);
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, status_buf, copy_len);
                    *status_buf.add(copy_len) = 0;
                }
            }
            0
        }
        Err(err) => {
            eprintln!("[Roscel CBE] Error al renderizar: {}", err);
            -2
        }
    }
}