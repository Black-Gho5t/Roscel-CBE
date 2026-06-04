#[no_mangle]
pub extern "C" fn vela_engine_init() {
    println!("Vela Engine initialized");
}

#[no_mangle]
pub extern "C" fn vela_engine_shutdown() {
    println!("Vela Engine shutdown");
}