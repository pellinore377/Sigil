use sigil_media::{decode, formats::Format, Error, Request};
fn run() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or(Error::Invalid)?;
    let format: Format =
        serde_json::from_str(&args.next().ok_or(Error::Invalid)?).map_err(|_| Error::Invalid)?;
    let request: Request =
        serde_json::from_str(&args.next().ok_or(Error::Invalid)?).map_err(|_| Error::Invalid)?;
    if args.next().is_some() {
        return Err(Error::Invalid);
    }
    decode::render(std::path::Path::new(&path), format, &request)?.write(std::io::stdout().lock())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(error.exit_code());
    }
}
