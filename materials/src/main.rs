#![forbid(unsafe_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    sigil_materials::app::desktop_main()
}
