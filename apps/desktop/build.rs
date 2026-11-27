//! Windows `VERSIONINFO` for `tft.exe`: without it the Properties dialog and SmartScreen's
//! warning have nothing to call the program. TFT ships no executable icon; gpui still embeds
//! its own manifest as resource 1 of type `RT_MANIFEST`.
//!
//! Everything here is a no-op off Windows.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

fn windows_resources() {
    use std::path::PathBuf;

    // `0.4.0` is `0,4,0,0` to the resource compiler, which counts to four and takes numbers only.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<&str> = version.split(['.', '-', '+']).collect();
    parts.resize(4, "0");
    let quad: Vec<String> = parts
        .iter()
        .map(|p| p.parse::<u16>().unwrap_or(0).to_string())
        .collect();
    let quad = quad.join(",");

    let rc = format!(
        r#"1 VERSIONINFO
FILEVERSION {quad}
PRODUCTVERSION {quad}
FILEOS 0x4L
FILETYPE 0x1L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "TFT"
      VALUE "FileDescription", "TFT: a native Markdown knowledge base"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "tft"
      VALUE "LegalCopyright", "MIT licensed"
      VALUE "OriginalFilename", "tft.exe"
      VALUE "ProductName", "TFT"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );

    let script = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("tft.rc");
    std::fs::write(&script, rc).expect("write the resource script");
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_required()
        .expect("compile the Windows resources");
}
