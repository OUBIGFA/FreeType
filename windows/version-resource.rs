use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "windows" {
        return;
    }
    let package = env::var("CARGO_PKG_NAME").unwrap();
    println!("cargo:rerun-if-env-changed=FREETYPE_BUILD_VERSION");
    let version = env::var("FREETYPE_BUILD_VERSION")
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").unwrap());
    let numbers: Vec<u16> = version
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect();
    assert_eq!(
        numbers.len(),
        3,
        "Windows builds require a numeric X.Y.Z version"
    );
    let (filename, description, file_type) = match package.as_str() {
        "freetype-server" => ("FreeTypeServer.exe", "FreeType input service", "VFT_APP"),
        "freetype-tsf" => ("freetype_tsf.dll", "FreeType text service", "VFT_DLL"),
        _ => panic!("Unsupported resource target: {package}"),
    };
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let template = manifest.parent().unwrap().join("version.rc");
    println!("cargo:rerun-if-changed={}", template.display());
    let resource = fs::read_to_string(template)
        .unwrap()
        .replace("@VERSION@", &version)
        .replace(
            "@NUMERIC_VERSION@",
            &format!("{},{},{},0", numbers[0], numbers[1], numbers[2]),
        )
        .replace("@FILENAME@", filename)
        .replace("@DESCRIPTION@", description)
        .replace("@FILETYPE@", file_type);
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("freetype-version.rc");
    fs::write(&output, resource).unwrap();
    let result = if package == "freetype-tsf" {
        embed_resource::compile_for_cdylib(output, embed_resource::NONE)
    } else {
        embed_resource::compile_for(output, ["FreeTypeServer"], embed_resource::NONE)
    };
    result
        .manifest_required()
        .expect("Windows version resources must be present");
}
