#![forbid(unsafe_code)]
#[cfg(target_arch = "wasm32")]
mod renderer;

#[cfg(any(target_arch = "wasm32", test))]
mod bounds;
