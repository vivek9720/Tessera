#![no_std]

#[macro_export]
macro_rules! fuzz_target {
    (|$data:ident: &[u8]| $body:block) => {
        #[no_mangle]
        pub extern "C" fn LLVMFuzzerTestOneInput(data: *const u8, size: usize) -> i32 {
            if data.is_null() {
                return 0;
            }
            let $data: &[u8] = unsafe { core::slice::from_raw_parts(data, size) };
            $body
            0
        }
    };
}
