use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/Vermouth.ico");

    if env::var("PROFILE").as_deref() != Ok("release")
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let icon = root.join("assets/icons/Vermouth.ico");

    assert!(icon.is_file(), "Missing Windows icon: {}", icon.display());

    fs::copy(&icon, out_dir.join("Vermouth.ico")).expect("Failed to copy Vermouth icon");

    fs::write(out_dir.join("Vermouth.rc"), "1 ICON \"Vermouth.ico\"\n")
        .expect("Failed to write Windows resource script");

    let rc =
        find_resource_compiler().expect("Windows SDK resource compiler (x64/rc.exe) not found");

    let output = Command::new(&rc)
        .current_dir(&out_dir)
        .args(["/nologo", "Vermouth.rc"])
        .output()
        .expect("Failed to start Windows resource compiler");

    assert!(
        output.status.success(),
        "Windows resource compilation failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let resource = out_dir.join("Vermouth.res");

    assert!(
        resource.is_file(),
        "Windows resource compiler did not produce Vermouth.res"
    );

    println!("cargo:rustc-link-arg-bin=vermouth={}", resource.display());
}

fn find_resource_compiler() -> Option<PathBuf> {
    let sdk_bin = PathBuf::from(env::var_os("ProgramFiles(x86)")?)
        .join("Windows Kits")
        .join("10")
        .join("bin");

    let mut compilers = fs::read_dir(sdk_bin)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64").join("rc.exe"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();

    compilers.sort();
    compilers.pop()
}
