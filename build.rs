use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-env-changed=DUBCAL_OAUTH_JSON");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("oauth_embedded.json");
    if let Some(path) = env::var_os("DUBCAL_OAUTH_JSON") {
        let path = PathBuf::from(path);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = std::fs::read(&path).expect("Cannot read the OAuth configuration file");
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("Invalid OAuth JSON"));
        let installed = &value["installed"];
        let client_id = installed["client_id"]
            .as_str()
            .filter(|v| v.ends_with(".apps.googleusercontent.com"))
            .expect("A Desktop app OAuth client ID is required");
        let client_secret = installed["client_secret"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .expect("A Desktop app OAuth client secret is required");
        // Embed only app credentials, not the original file's unrelated metadata.
        let minimal = serde_json::json!({ "installed": { "client_id": client_id, "client_secret": client_secret } });
        std::fs::write(output, serde_json::to_vec(&minimal).unwrap())
            .expect("Cannot write embedded OAuth configuration");
    } else {
        std::fs::write(output, b"").expect("Cannot write empty OAuth configuration");
    }

    println!("cargo:rerun-if-changed=app.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    } else {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let resource = out.join("manifest.rc");
        let object = out.join("manifest.o");
        std::fs::write(
            &resource,
            format!(
                "1 24 \"{}\"\n",
                manifest.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        let compiler = env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
        let status = Command::new(compiler)
            .arg(&resource)
            .arg("-O")
            .arg("coff")
            .arg("-o")
            .arg(&object)
            .status()
            .expect("MinGW windres is required");
        assert!(status.success(), "manifest compilation failed");
        println!("cargo:rustc-link-arg={}", object.display());
    }
}
