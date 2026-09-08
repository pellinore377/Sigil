use super::*;
pub(super) fn render(path: &Path, index: u32, width: u32) -> Result<Preview, Error> {
    check_zip(path)?;
    let office = std::env::var_os("SIGIL_OFFICE").ok_or(Error::Unavailable)?;
    let root = Path::new("/tmp/sigil-office");
    std::fs::create_dir(root)?;
    std::fs::create_dir_all(root.join("profile/user"))?;
    std::fs::write(root.join("profile/user/registrymodifications.xcu"), br#"<?xml version="1.0" encoding="UTF-8"?><oor:items xmlns:oor="http://openoffice.org/2001/registry"><item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop><prop oor:name="DisableActiveContent" oor:op="fuse"><value>true</value></prop><prop oor:name="DisablePythonRuntime" oor:op="fuse"><value>true</value></prop><prop oor:name="DisableOLEAutomation" oor:op="fuse"><value>true</value></prop></item></oor:items>"#)?;
    let mut command = std::process::Command::new(office);
    command
        .args([
            "-env:UserInstallation=file:///tmp/sigil-office/profile",
            "--headless",
            "--norestore",
            "--nodefault",
            "--nolockcheck",
            "--convert-to",
            "pdf",
            "--outdir",
        ])
        .arg(root)
        .arg(path);
    av::output(&mut command, 65536)?;
    let mut pdf = std::path::PathBuf::from(path.file_name().ok_or(Error::Invalid)?);
    pdf.set_extension("pdf");
    let path = root.join(pdf);
    if std::fs::metadata(&path)?.len() > MAX_INPUT {
        return Err(Error::Limit);
    }
    pdf::render(&path, index, width)
}
