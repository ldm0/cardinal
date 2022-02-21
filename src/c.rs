#[no_mangle]
pub extern "C" fn c_init_sdk() {
    crate::init_sdk_facade();
}

#[no_mangle]
pub extern "C" fn c_close_sdk() {
    crate::close_sdk_facade();
}
