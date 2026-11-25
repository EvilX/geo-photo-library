//! Сборка под Windows: иконка и сведения о версии в exe (packaging/windows/geo-photo-library.rc).
//! Компилятор ресурсов — `windres` из MSYS2 (binutils в UCRT64, llvm-windres в CLANGARM64).
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") || env::var_os("CARGO_FEATURE_GTK_UI").is_none() {
        return;
    }
    let rc = PathBuf::from("packaging/windows/geo-photo-library.rc");
    println!("cargo:rerun-if-changed={}", rc.display());
    println!("cargo:rerun-if-changed=packaging/windows/geo-photo-library.ico");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let num: Vec<&str> = version.split(['.', '-']).take(3).collect();
    let dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    std::fs::write(
        dir.join("version.h"),
        format!("#define VER_NUM {},0\n#define VER_STR \"{version}\"\n", num.join(",")),
    )
    .unwrap();
    let out = dir.join("resources.o");
    let windres = env::var("WINDRES").unwrap_or_else(|_| "windres".into());
    let status = Command::new(&windres)
        .arg("--include-dir")
        .arg(&dir)
        .args(["--include-dir", "packaging/windows", "--input-format=rc", "--output-format=coff", "-i"])
        .arg(&rc)
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap_or_else(|e| panic!("не запустить {windres}: {e}"));
    assert!(status.success(), "{windres} завершился с ошибкой");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
