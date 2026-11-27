//! Gives HomeKVM.exe its manifest, icon and version details (seen in Explorer and file
//! Properties).
//! The icon is drawn by the same code the app uses, so they always match.

use std::path::{Path, PathBuf};

#[path = "src/icon_art.rs"]
mod icon_art;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = out.join("homekvm.ico");
    std::fs::write(&ico, ico_file(&[16, 20, 24, 32, 40, 48, 64, 256])).expect("write icon");

    let version = std::env::var("CARGO_PKG_VERSION").expect("version");
    let parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let (a, b, c) = (parts[0], parts.get(1).copied().unwrap_or(0), parts.get(2).copied().unwrap_or(0));

    // Application manifest: runs as the user (never asks for admin), declares Windows 10/11
    // support, per-monitor DPI awareness, long paths and modern common controls.
    let manifest = out.join("homekvm.manifest");
    std::fs::write(
        &manifest,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="HomeKVM" version="{a}.{b}.{c}.0" processorArchitecture="amd64"/>
  <description>HomeKVM: one keyboard and mouse across your Windows PCs</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#
        ),
    )
    .expect("write manifest");

    // Resource 1 of type 24 (RT_MANIFEST) is the manifest Windows reads for an exe.
    let rc = format!(
        r#"1 24 "{manifest}"
1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "HomeKVM"
      VALUE "ProductName", "HomeKVM"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "InternalName", "HomeKVM"
      VALUE "OriginalFilename", "HomeKVM.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        manifest = forward_slashes(&manifest),
        ico = forward_slashes(&ico)
    );
    let rc_path = out.join("homekvm.rc");
    std::fs::write(&rc_path, rc).expect("write resource script");
    embed_resource::compile(&rc_path, embed_resource::NONE);
}

fn forward_slashes(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// A .ico file holding the icon at each size (32-bit images with alpha).
fn ico_file(sizes: &[usize]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| bmp_image(n, &icon_art::draw(n, None))).collect();
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes()); // reserved
    out.extend(1u16.to_le_bytes()); // type: icon
    out.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&n, img) in sizes.iter().zip(&images) {
        let dim = if n >= 256 { 0 } else { n as u8 }; // 0 means 256
        out.extend([dim, dim, 0, 0]);
        out.extend(1u16.to_le_bytes()); // planes
        out.extend(32u16.to_le_bytes()); // bits per pixel
        out.extend((img.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}

/// One icon image: a bottom-up 32-bit BGRA bitmap plus an empty transparency mask.
fn bmp_image(n: usize, rgba: &[u8]) -> Vec<u8> {
    let mask_row = n.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend(40u32.to_le_bytes()); // header size
    out.extend((n as i32).to_le_bytes());
    out.extend((2 * n as i32).to_le_bytes()); // color + mask
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // no compression
    out.extend(((n * n * 4 + mask_row * n) as u32).to_le_bytes());
    out.extend([0u8; 16]); // resolution and palette fields
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            out.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.extend(vec![0u8; mask_row * n]);
    out
}
